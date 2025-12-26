//! Execution engine for KyroQL IR.
//!
//! This module provides a synchronous executor that applies operations (`KyroIR`) against
//! pluggable storage backends.

mod write_path;

/// Routed runtime enforcing Reflex/Reflection isolation.
pub mod runtime;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::{OnceLock, RwLock};

use chrono::{DateTime, Utc};

use crate::belief::{Belief, ConsistencyStatus};
use crate::confidence::{BeliefId, Confidence};
use crate::conflict::{Conflict, ConflictId};
use crate::derivation::{DerivationId, DerivationRecord};
use crate::entity::{EntityId};
use crate::error::{ExecutionError, KyroError, KyroResult, ValidationError};
use crate::frame::{BeliefFrame, Evidence, KnowledgeGap, RankedClaim};
use crate::inference::{ConflictResolutionPolicy, PolicyDecision};
use crate::ir::{
    ConsistencyMode, DefinePatternPayload, DerivePayload, KyroIR, MonitorPayload, Operation,
    ResolvePayload, RetractPayload, SimulatePayload,
};
use crate::monitor::{MonitorRegistration, MonitorSystem, MonitorSystemConfig};
use crate::monitor::matcher::AssertObservation;
use crate::pattern::{Pattern, PatternId, PatternRule};
use crate::simulation::{SimulateConstraints, SimulationBaseStores, SimulationContext};
use crate::storage::{
    BeliefStore, ConflictStore, DerivationStore, EntityStore, PatternStore, StorageError,
};
use crate::time::TimeRange;
use crate::value::Value;
use crate::trust::{TrustModel, SimpleTrustModel};
use crate::meta::MetaAnalyzer;

const REGEX_CACHE_MAX: usize = 1024;

static REGEX_CACHE: OnceLock<RwLock<HashMap<String, regex::Regex>>> = OnceLock::new();

fn cached_regex(pattern: &str) -> KyroResult<regex::Regex> {
    let cache = REGEX_CACHE.get_or_init(|| RwLock::new(HashMap::new()));

    {
        let guard = cache
            .read()
            .map_err(|_| KyroError::internal("regex cache lock poisoned"))?;
        if let Some(re) = guard.get(pattern) {
            return Ok(re.clone());
        }
    }

    let compiled = regex::Regex::new(pattern).map_err(|e| {
        KyroError::Validation(ValidationError::InvalidPatternRule {
            reason: format!("invalid regex '{pattern}': {e}"),
        })
    })?;

    let mut guard = cache
        .write()
        .map_err(|_| KyroError::internal("regex cache lock poisoned"))?;

    if guard.len() >= REGEX_CACHE_MAX {
        // Keep the cache bounded to avoid unbounded memory usage.
        guard.clear();
    }

    // Another thread may have inserted it while we compiled.
    guard
        .entry(pattern.to_string())
        .or_insert_with(|| compiled.clone());
    Ok(compiled)
}

/// Result of executing a KyroQL operation.
#[derive(Debug)]
pub enum EngineResponse {
    /// Result of an ASSERT.
    Assert {
        /// The inserted belief ID.
        belief_id: BeliefId,
        /// Any detected conflicts.
        conflict_ids: Vec<ConflictId>,
    },

    /// Result of a RESOLVE.
    Resolve {
        /// The produced belief frame.
        frame: BeliefFrame,
    },

    /// Result of a RETRACT.
    Retract {
        /// The retraction belief ID.
        retraction_belief_id: BeliefId,
    },

    /// Result of DEFINE_PATTERN.
    DefinePattern {
        /// The stored pattern ID.
        pattern_id: PatternId,
    },

    /// Result of a SIMULATE.
    Simulate {
        /// The created simulation context.
        simulation: Arc<SimulationContext>,
    },

    /// Result of a MONITOR registration.
    Monitor {
        /// The created registration (includes event stream).
        registration: MonitorRegistration,
    },

    /// Result of a DERIVE.
    Derive {
        /// The stored derivation record ID.
        derivation_id: DerivationId,
    },
}

/// KyroQL execution engine.
#[derive(Clone)]
pub struct KyroEngine {
    entities: Arc<dyn EntityStore>,
    beliefs: Arc<dyn BeliefStore>,
    patterns: Arc<dyn PatternStore>,
    conflicts: Arc<dyn ConflictStore>,
    derivations: Arc<dyn DerivationStore>,
    monitor: Arc<MonitorSystem>,
    trust: Arc<dyn TrustModel>,
}

impl KyroEngine {
    /// Create a new engine using the given stores.
    #[must_use]
    pub fn new(
        entities: Arc<dyn EntityStore>,
        beliefs: Arc<dyn BeliefStore>,
        patterns: Arc<dyn PatternStore>,
        conflicts: Arc<dyn ConflictStore>,
        derivations: Arc<dyn DerivationStore>,
    ) -> Self {
        let monitor = Arc::new(MonitorSystem::new(
            MonitorSystemConfig::default(),
            Arc::clone(&beliefs),
        ));
        let trust = Arc::new(SimpleTrustModel::new());
        Self {
            entities,
            beliefs,
            patterns,
            conflicts,
            derivations,
            monitor,
            trust,
        }
    }

    /// Create a new engine with an explicit trust model.
    #[must_use]
    pub fn with_trust_model(
        entities: Arc<dyn EntityStore>,
        beliefs: Arc<dyn BeliefStore>,
        patterns: Arc<dyn PatternStore>,
        conflicts: Arc<dyn ConflictStore>,
        derivations: Arc<dyn DerivationStore>,
        trust: Arc<dyn TrustModel>,
    ) -> Self {
        let monitor = Arc::new(MonitorSystem::new(
            MonitorSystemConfig::default(),
            Arc::clone(&beliefs),
        ));
        Self {
            entities,
            beliefs,
            patterns,
            conflicts,
            derivations,
            monitor,
            trust,
        }
    }
    
    /// Get a reference to the entity store.
    pub fn entity_store(&self) -> &Arc<dyn EntityStore> {
        &self.entities
    }
    
    /// Get a reference to the belief store.
    pub fn belief_store(&self) -> &Arc<dyn BeliefStore> {
        &self.beliefs
    }
    
    /// Get a reference to the pattern store.
    pub fn pattern_store(&self) -> &Arc<dyn PatternStore> {
        &self.patterns
    }
    
    /// Get a reference to the conflict store.
    pub fn conflict_store(&self) -> &Arc<dyn ConflictStore> {
        &self.conflicts
    }
    
    /// Get a reference to the derivation store.
    pub fn derivation_store(&self) -> &Arc<dyn DerivationStore> {
        &self.derivations
    }
    
    /// Get a reference to the monitor system.
    pub fn monitor_system(&self) -> &Arc<MonitorSystem> {
        &self.monitor
    }

    /// Access the configured trust model.
    pub fn trust_model(&self) -> &Arc<dyn TrustModel> {
        &self.trust
    }

    /// Construct a meta-knowledge analyzer.
    pub fn meta_analyzer(&self) -> MetaAnalyzer {
        MetaAnalyzer::new(Arc::clone(&self.entities), Arc::clone(&self.beliefs))
    }

    fn trust_weight(&self, source: &crate::source::Source, domain: Option<&str>) -> f32 {
        self.trust.assess(source, domain).weight()
    }

    fn trusted_confidence(&self, belief: &Belief, domain: Option<&str>) -> f32 {
        belief.confidence.value().clamp(0.0, 1.0) * self.trust_weight(&belief.source, domain)
    }

    fn decide_with_trust(
        &self,
        policy: &ConflictResolutionPolicy,
        beliefs: &[Belief],
        domain: Option<&str>,
    ) -> PolicyDecision {
        if beliefs.is_empty() {
            return PolicyDecision::Unresolved;
        }

        match policy {
            ConflictResolutionPolicy::ExplicitConflict => PolicyDecision::Unresolved,
            ConflictResolutionPolicy::LatestWins => {
                let mut best = &beliefs[0];
                for b in &beliefs[1..] {
                    if b.tx_time > best.tx_time {
                        best = b;
                    } else if b.tx_time == best.tx_time {
                        let tc = self.trusted_confidence(b, domain);
                        let bc = self.trusted_confidence(best, domain);
                        if tc > bc || (tc == bc && b.id.to_string() < best.id.to_string()) {
                            best = b;
                        }
                    }
                }
                PolicyDecision::Selected(best.id)
            }
            ConflictResolutionPolicy::HighestConfidence => {
                let mut best = &beliefs[0];
                let mut best_score = self.trusted_confidence(best, domain);
                for b in &beliefs[1..] {
                    let score = self.trusted_confidence(b, domain);
                    if score > best_score {
                        best = b;
                        best_score = score;
                    } else if score == best_score {
                        if b.tx_time > best.tx_time
                            || (b.tx_time == best.tx_time && b.id.to_string() < best.id.to_string())
                        {
                            best = b;
                            best_score = score;
                        }
                    }
                }
                PolicyDecision::Selected(best.id)
            }
            ConflictResolutionPolicy::SourcePriority { priority } => {
                let priority = priority.as_slice();
                let rank = |b: &Belief| {
                    let sid = b.source.source_id();
                    priority
                        .iter()
                        .position(|p| *p == sid)
                        .unwrap_or(usize::MAX)
                };

                let mut best = &beliefs[0];
                let mut best_rank = rank(best);
                let mut best_score = self.trusted_confidence(best, domain);

                for b in &beliefs[1..] {
                    let r = rank(b);
                    let score = self.trusted_confidence(b, domain);
                    if r < best_rank {
                        best = b;
                        best_rank = r;
                        best_score = score;
                    } else if r == best_rank {
                        if score > best_score {
                            best = b;
                            best_score = score;
                        } else if score == best_score {
                            if b.tx_time > best.tx_time
                                || (b.tx_time == best.tx_time && b.id.to_string() < best.id.to_string())
                            {
                                best = b;
                                best_score = score;
                            }
                        }
                    }
                }

                PolicyDecision::Selected(best.id)
            }
        }
    }

    /// Execute a KyroQL IR request.
    pub fn execute(&self, ir: KyroIR) -> KyroResult<EngineResponse> {
        // Defensive validation for deserialized IR.
        // Builders already validate, but server/embedded execution must not trust inputs.
        ir.operation.validate().map_err(KyroError::from)?;

        match ir.operation {
            Operation::Assert(payload) => self.execute_assert(ir.timestamp, payload.consistency_mode, payload.entity_id, payload.predicate, payload.value, payload.confidence, payload.source, payload.valid_time, payload.embedding),
            Operation::Resolve(payload) => self.execute_resolve(payload),
            Operation::Simulate(payload) => self.execute_simulate(payload),
            Operation::Monitor(payload) => self.execute_monitor(payload),
            Operation::Derive(payload) => self.execute_derive(ir.timestamp, payload),
            Operation::Retract(payload) => self.execute_retract(ir.timestamp, payload),
            Operation::DefinePattern(payload) => self.execute_define_pattern(payload),
        }
    }

    fn execute_derive(&self, tx_time: DateTime<Utc>, payload: DerivePayload) -> KyroResult<EngineResponse> {
        let rule = payload.rule.ok_or_else(|| KyroError::Validation(ValidationError::MissingField {
            field: "rule".to_string(),
        }))?;

        let sources = payload.sources.ok_or_else(|| KyroError::Validation(ValidationError::MissingField {
            field: "sources".to_string(),
        }))?;

        if sources.is_empty() {
            return Err(KyroError::Validation(ValidationError::MissingField {
                field: "sources".to_string(),
            }));
        }

        // Deduplicate sources while preserving first-seen order.
        let mut seen = std::collections::HashSet::<BeliefId>::with_capacity(sources.len());
        let mut premise_ids = Vec::with_capacity(sources.len());
        for id in sources {
            if seen.insert(id) {
                premise_ids.push(id);
            }
        }

        if let Some(derived) = payload.derived_belief_id {
            if premise_ids.iter().any(|p| *p == derived) {
                return Err(KyroError::Execution(ExecutionError::InvalidDerivation {
                    reason: "derived_belief_id must not appear in premise_ids".to_string(),
                }));
            }

            let exists = self
                .beliefs
                .get(derived)
                .map_err(Self::storage_err)?
                .is_some();
            if !exists {
                return Err(KyroError::Execution(ExecutionError::BeliefNotFound { id: derived }));
            }
        }

        for premise in &premise_ids {
            let exists = self
                .beliefs
                .get(*premise)
                .map_err(Self::storage_err)?
                .is_some();
            if !exists {
                return Err(KyroError::Execution(ExecutionError::BeliefNotFound { id: *premise }));
            }
        }

        let steps = payload.inference_steps.unwrap_or_default();

        let record = DerivationRecord::new(
            tx_time,
            payload.derived_belief_id,
            premise_ids,
            rule,
            steps,
            payload.confidence,
            payload.justification,
            payload.metadata,
        )
        .map_err(KyroError::from)?;

        let id = record.id;
        self.derivations.insert(record).map_err(Self::storage_err)?;

        Ok(EngineResponse::Derive { derivation_id: id })
    }

    fn execute_monitor(&self, payload: MonitorPayload) -> KyroResult<EngineResponse> {
        let threshold = payload.threshold.unwrap_or(Value::Null);

        let triggers = self.monitor.triggers_from_threshold_value(
            &threshold,
            payload.entity_filter.as_deref(),
            payload.predicates.as_deref(),
            payload.pattern_filter.as_deref(),
        )?;

        let registration = self.monitor.register(triggers, payload.expires_at)?;
        Ok(EngineResponse::Monitor { registration })
    }

    fn execute_simulate(&self, payload: SimulatePayload) -> KyroResult<EngineResponse> {
        let constraints = match payload.constraints {
            None => SimulateConstraints::default(),
            Some(Value::Null) => SimulateConstraints::default(),
            Some(Value::Structured(v)) => serde_json::from_value::<SimulateConstraints>(v)
                .map_err(|e| KyroError::Validation(ValidationError::InvalidSimulationConstraints {
                    reason: format!("invalid constraints object: {e}"),
                }))?,
            Some(other) => {
                return Err(KyroError::Validation(ValidationError::InvalidSimulationConstraints {
                    reason: format!(
                        "constraints must be Value::Structured (JSON object), got {other:?}"
                    ),
                }));
            }
        };

        constraints.validate().map_err(KyroError::from)?;

        if let Some(entities) = payload.entities.as_ref() {
            for &id in entities {
                self.ensure_entity_exists(id)?;
            }
        }

        let base = SimulationBaseStores {
            entities: Arc::clone(&self.entities),
            beliefs: Arc::clone(&self.beliefs),
            patterns: Arc::clone(&self.patterns),
            conflicts: Arc::clone(&self.conflicts),
        };

        let ctx = SimulationContext::new(base, constraints)?;
        Ok(EngineResponse::Simulate {
            simulation: Arc::new(ctx),
        })
    }

    fn storage_err(err: StorageError) -> KyroError {
        KyroError::Execution(ExecutionError::Storage {
            message: err.to_string(),
        })
    }

    fn ensure_entity_exists(&self, id: EntityId) -> KyroResult<()> {
        match self.entities.get(id).map_err(Self::storage_err)? {
            Some(_) => Ok(()),
            None => Err(KyroError::Execution(ExecutionError::EntityNotFound { id })),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_assert(
        &self,
        tx_time: DateTime<Utc>,
        mode: ConsistencyMode,
        entity_id: EntityId,
        predicate: String,
        value: Value,
        confidence: Confidence,
        source: crate::source::Source,
        valid_time: TimeRange,
        embedding: Option<Vec<f32>>,
    ) -> KyroResult<EngineResponse> {
        self.ensure_entity_exists(entity_id)?;

        // Deterministic embedding generation.
        // If an embedding is not provided, generate one from the entity name + predicate + value.
        let embedding = match embedding {
            Some(v) => Some(v),
            None => {
                let entity = self
                    .entities
                    .get(entity_id)
                    .map_err(Self::storage_err)?
                    .ok_or(KyroError::Execution(ExecutionError::EntityNotFound { id: entity_id }))?;
                let text = format!("{} {} {}", entity.canonical_name, predicate.trim(), value);
                Some(crate::embedding::lexical_embedding(&text))
            }
        };

        let predicate = predicate.trim().to_string();
        if predicate.is_empty() {
            return Err(ValidationError::EmptyPredicate.into());
        }

        let mut belief = Belief {
            id: BeliefId::new(),
            subject: entity_id,
            predicate: predicate.clone(),
            value: value.clone(),
            confidence: confidence.clone(),
            source,
            valid_time,
            tx_time,
            reason: None,
            consistency_status: ConsistencyStatus::Provisional,
            supersedes: None,
            superseded_by: None,
            embedding,
        };

        let belief_id = belief.id;

        if mode.is_force() {
            self.beliefs.insert(belief).map_err(Self::storage_err)?;

            self.monitor.observe_assert(AssertObservation {
                tx_time,
                belief_id,
                entity_id,
                predicate: predicate.clone(),
                value: value.clone(),
                confidence: confidence.value(),
                conflict_types: Vec::new(),
            });

            return Ok(EngineResponse::Assert {
                belief_id,
                conflict_ids: Vec::new(),
            });
        }

        let conflicts = self.detect_conflicts(&belief, tx_time)?;

        if mode.is_strict() && !conflicts.is_empty() {
            return Err(KyroError::Execution(ExecutionError::ConflictsDetected {
                conflicts: conflicts
                    .iter()
                    .map(|c| c.conflict_type.to_string())
                    .collect(),
            }));
        }

        // When checks pass, mark the belief as verified.
        if conflicts.is_empty() {
            belief.consistency_status = ConsistencyStatus::Verified;
            self.beliefs.insert(belief).map_err(Self::storage_err)?;

            self.monitor.observe_assert(AssertObservation {
                tx_time,
                belief_id,
                entity_id,
                predicate: predicate.clone(),
                value: value.clone(),
                confidence: confidence.value(),
                conflict_types: Vec::new(),
            });

            return Ok(EngineResponse::Assert {
                belief_id,
                conflict_ids: Vec::new(),
            });
        }

        // Eventual mode records conflicts and writes contested belief.
        // Insert conflicts before the belief so the belief never points at missing conflicts.
        let mut conflict_ids: Vec<ConflictId> = Vec::new();
        if mode.is_eventual() {
            for conflict in &conflicts {
                self.conflicts
                    .insert(conflict.clone())
                    .map_err(Self::storage_err)?;
                conflict_ids.push(conflict.id);
            }
        }

        belief.consistency_status = ConsistencyStatus::Contested {
            conflict_ids: conflict_ids.clone(),
        };
        self.beliefs.insert(belief).map_err(Self::storage_err)?;

        let conflict_types: Vec<crate::conflict::ConflictType> =
            conflicts.iter().map(|c| c.conflict_type.clone()).collect();

        self.monitor.observe_assert(AssertObservation {
            tx_time,
            belief_id,
            entity_id,
            predicate: predicate.clone(),
            value: value.clone(),
            confidence: confidence.value(),
            conflict_types,
        });

        Ok(EngineResponse::Assert {
            belief_id,
            conflict_ids,
        })
    }

    fn execute_define_pattern(&self, payload: DefinePatternPayload) -> KyroResult<EngineResponse> {
        let name = payload.name.trim();
        if name.is_empty() {
            return Err(ValidationError::MissingField {
                field: "name".to_string(),
            }
            .into());
        }

        let mut pattern = Pattern::new(name, payload.rule, payload.confidence);
        pattern.description = payload.description;
        pattern.valid_time = payload.valid_time;
        pattern.active = true;

        self.patterns.insert(pattern.clone()).map_err(Self::storage_err)?;

        Ok(EngineResponse::DefinePattern {
            pattern_id: pattern.id,
        })
    }

    fn execute_retract(&self, tx_time: DateTime<Utc>, payload: RetractPayload) -> KyroResult<EngineResponse> {
        let Some(old) = self.beliefs.get(payload.belief_id).map_err(Self::storage_err)? else {
            return Err(KyroError::Execution(ExecutionError::BeliefNotFound {
                id: payload.belief_id,
            }));
        };

        // Create a retraction belief that supersedes the old one.
        let retraction = Belief {
            id: BeliefId::new(),
            subject: old.subject,
            predicate: old.predicate,
            value: Value::Null,
            confidence: Confidence::from_agent(1.0, "system").map_err(KyroError::from)?,
            source: payload.authorized_by,
            valid_time: TimeRange::starting_at(tx_time),
            tx_time,
            reason: payload.reason.clone(),
            consistency_status: ConsistencyStatus::Verified,
            supersedes: Some(old.id),
            superseded_by: None,
            embedding: None,
        };

        self.beliefs.insert(retraction.clone()).map_err(Self::storage_err)?;
        self.beliefs
            .supersede(old.id, retraction.id)
            .map_err(Self::storage_err)?;

        Ok(EngineResponse::Retract {
            retraction_belief_id: retraction.id,
        })
    }

    fn execute_resolve(&self, payload: ResolvePayload) -> KyroResult<EngineResponse> {
        let as_of = payload.as_of.unwrap_or_else(Utc::now);
        let min_conf = payload.min_confidence.unwrap_or(0.0).clamp(0.0, 1.0);
        let policy = payload
            .conflict_policy
            .clone()
            .unwrap_or_else(ConflictResolutionPolicy::default);
        let mut trust_domain = payload.trust_domain.as_deref();

        // Conservative entity resolution from query.
        // We only auto-resolve if:
        // - entity_id was not provided
        // - query looks like an entity name (short, no '?')
        // - fuzzy search yields exactly one candidate
        let mut entity_id = payload.entity_id;
        if entity_id.is_none() {
            if let Some(q) = payload.query.as_deref() {
                let q = q.trim();
                let looks_like_name = !q.is_empty()
                    && q.len() <= 80
                    && !q.contains('?')
                    && q.split_whitespace().count() <= 6;
                if looks_like_name {
                    let candidates = self
                        .entities
                        .find_by_name_fuzzy(q, 2)
                        .map_err(Self::storage_err)?;
                    if candidates.len() == 1 {
                        entity_id = Some(candidates[0].id);
                    }
                }
            }
        }

        let mut frame = BeliefFrame::empty();
        frame.time_window = TimeRange::instant(as_of);
        frame.query_assumptions.as_of_time = as_of;
        frame.query_assumptions.min_confidence = payload.min_confidence;
        frame.query_assumptions.conflict_policy = policy.clone();
        frame.query_assumptions.trust_model = self.trust.name().to_string();

        // Semantic path (top-k embedding retrieval) if a query embedding is present.
        if let Some(query_embedding) = payload.query_embedding.as_deref() {
            let mut matches = self
                .beliefs
                .find_by_embedding(query_embedding, payload.limit * 4, Some(min_conf))
                .map_err(Self::storage_err)?;

            // Apply AS_OF validity.
            matches.retain(|(b, _)| b.is_valid_at(as_of));

            // Apply optional filters.
            if let Some(eid) = entity_id {
                matches.retain(|(b, _)| b.subject == eid);
                self.ensure_entity_exists(eid)?;
            }

            let predicate_filter = payload
                .predicate
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty());
            let trust_scope = trust_domain.or(predicate_filter);
            if let Some(pred) = predicate_filter {
                matches.retain(|(b, _)| b.predicate == pred);
            }

            // If nothing matched, report gaps.
            if matches.is_empty() {
                if payload.include_gaps {
                    if let Some(eid) = entity_id {
                        if let Some(pred) = predicate_filter {
                            frame
                                .gaps
                                .push(KnowledgeGap::new(
                                    crate::frame::GapType::NoDataFound,
                                    format!("No semantically relevant beliefs found for '{pred}'"),
                                )
                                .with_missing_entity(eid)
                                .with_missing_predicate(pred.to_string()));
                        } else {
                            frame
                                .gaps
                                .push(KnowledgeGap::new(
                                    crate::frame::GapType::NoDataFound,
                                    "No semantically relevant beliefs found",
                                )
                                .with_missing_entity(eid));
                        }
                    } else {
                        frame.gaps.push(
                            KnowledgeGap::missing_entity(
                                "Semantic search returned no beliefs; provide entity_id or refine query",
                            )
                            .with_suggested_query(
                                "Provide entity_id and/or predicate, or refine query text",
                            ),
                        );
                    }
                }
                return Ok(EngineResponse::Resolve { frame });
            }

            // Sort by similarity (descending), then by confidence.
            matches.sort_by(|(a, sa), (b, sb)| {
                sb.total_cmp(sa)
                    .then_with(|| {
                        let ba = self.trusted_confidence(a, trust_scope);
                        let bb = self.trusted_confidence(b, trust_scope);
                        bb.total_cmp(&ba)
                    })
                    .then_with(|| b.tx_time.cmp(&a.tx_time))
                    .then_with(|| b.id.to_string().cmp(&a.id.to_string()))
            });
            matches.truncate(payload.limit);

            // Convert to beliefs while keeping relevance.
            let best_score = matches.first().map(|(_, s)| *s).unwrap_or(0.0).clamp(0.0, 1.0);

            let mut beliefs: Vec<Belief> = matches.iter().map(|(b, _)| b.clone()).collect();
            beliefs.sort_by(|a, b| {
                let ca = self.trusted_confidence(a, trust_scope);
                let cb = self.trusted_confidence(b, trust_scope);
                cb.total_cmp(&ca)
            });

            // Conflict resolution for semantic results is still per-predicate/per-entity.
            // If filters don't constrain to a single predicate, we conservatively do not pick.
            let distinct_predicates: std::collections::HashSet<&str> =
                beliefs.iter().map(|b| b.predicate.as_str()).collect();
            if distinct_predicates.len() != 1 {
                if payload.include_gaps {
                    frame.gaps.push(KnowledgeGap::new(
                        crate::frame::GapType::InsufficientEvidence,
                        "Semantic RESOLVE matched multiple predicates; specify predicate to synthesize an answer",
                    ));
                }

                // Still attach evidence with relevance weights.
                for (b, score) in matches {
                    let trusted_conf = self.trusted_confidence(&b, trust_scope);
                    frame.supporting_evidence.push(Evidence::new(
                        b.id,
                        b.predicate.clone(),
                        b.source.clone(),
                        trusted_conf,
                        score.clamp(0.0, 1.0),
                    ));
                }
                return Ok(EngineResponse::Resolve { frame });
            }

            // Now we can treat it like the strict path for one predicate.

            let mut distinct_values: Vec<Value> = Vec::new();
            for b in &beliefs {
                if !distinct_values.iter().any(|v| v == &b.value) {
                    distinct_values.push(b.value.clone());
                }
            }

            let (winner_id, decision) = if distinct_values.len() <= 1 {
                (beliefs[0].id, PolicyDecision::Selected(beliefs[0].id))
            } else {
                let decision = self.decide_with_trust(&policy, &beliefs, trust_scope);
                match decision {
                    PolicyDecision::Selected(id) => (id, decision),
                    PolicyDecision::Unresolved => {
                        if payload.include_gaps {
                            frame.gaps.push(KnowledgeGap::new(
                                crate::frame::GapType::InsufficientEvidence,
                                "Competing beliefs exist; no resolution policy selected",
                            ));
                        }
                        frame.debug_summary = Some(
                            "multiple competing beliefs found and conflict policy did not select a winner".to_string(),
                        );
                        (beliefs[0].id, decision)
                    }
                }
            };

            let winner = beliefs
                .iter()
                .find(|b| b.id == winner_id)
                .unwrap_or(&beliefs[0]);

            let claim = RankedClaim::new(
                winner.clone(),
                self.trusted_confidence(winner, trust_scope),
                best_score,
            );

            // Attach evidence with relevance weights.
            for (b, score) in matches {
                let trusted_conf = self.trusted_confidence(&b, trust_scope);
                if b.value == winner.value {
                    frame.supporting_evidence.push(Evidence::new(
                        b.id,
                        b.predicate.clone(),
                        b.source.clone(),
                        trusted_conf,
                        score.clamp(0.0, 1.0),
                    ));
                } else if payload.include_counter_evidence {
                    frame.counter_evidence.push(Evidence::new(
                        b.id,
                        b.predicate.clone(),
                        b.source.clone(),
                        trusted_conf,
                        score.clamp(0.0, 1.0),
                    ));
                }

                let conflicts = self
                    .conflicts
                    .find_by_belief(b.id)
                    .map_err(Self::storage_err)?;
                for c in conflicts {
                    if c.is_open() {
                        frame.conflicts.push(c);
                    }
                }
            }

            if !matches!(decision, PolicyDecision::Unresolved) {
                frame.best_supported_claim = Some(claim);
            }

            return Ok(EngineResponse::Resolve { frame });
        }

        let predicate = payload.predicate.as_deref().map(str::trim).filter(|p| !p.is_empty());

        // If we cannot resolve the entity, we cannot query stores meaningfully.
        let Some(entity_id) = entity_id else {
            if payload.include_gaps {
                frame.gaps.push(KnowledgeGap::missing_entity(
                    "resolve requires an entity_id (or a query that resolves to exactly one entity)",
                ));
            }
            frame.debug_summary = Some(
                "resolve requires an entity_id (or a query that resolves to exactly one entity)".to_string(),
            );
            return Ok(EngineResponse::Resolve { frame });
        };

        self.ensure_entity_exists(entity_id)?;

        // If predicate is missing, return a structured gap rather than hard error.
        let Some(predicate) = predicate else {
            if payload.include_gaps {
                // If the entity has no beliefs at all, report that; otherwise we still need a predicate.
                let count = self.beliefs.count_by_entity(entity_id).map_err(Self::storage_err)?;
                if count == 0 {
                    frame.gaps.push(
                        KnowledgeGap::new(
                            crate::frame::GapType::NoDataFound,
                            "Entity has no beliefs to resolve",
                        )
                        .with_missing_entity(entity_id),
                    );
                } else {
                    frame.gaps.push(
                        KnowledgeGap::new(
                            crate::frame::GapType::InsufficientEvidence,
                            "resolve requires a predicate to answer this query",
                        )
                        .with_missing_entity(entity_id),
                    );
                }
            }

            frame.debug_summary = Some(
                "resolve requires a predicate when using the current storage APIs".to_string(),
            );
            return Ok(EngineResponse::Resolve { frame });
        };

        if trust_domain.is_none() {
            trust_domain = Some(predicate);
        }

        let all = self
            .beliefs
            .find_as_of(entity_id, predicate, as_of)
            .map_err(Self::storage_err)?;

        let max_conf = all
            .iter()
            .map(|b| b.confidence.value())
            .fold(0.0f32, f32::max);
        let mut beliefs: Vec<Belief> = all
            .into_iter()
            .filter(|b| b.confidence.value() >= min_conf)
            .collect();

        let trust_scope = trust_domain;
        beliefs.sort_by(|a, b| {
            let ca = self.trusted_confidence(a, trust_scope);
            let cb = self.trusted_confidence(b, trust_scope);
            cb.total_cmp(&ca)
        });
        beliefs.truncate(payload.limit);

        if beliefs.is_empty() {
            if payload.include_gaps {
                if max_conf > 0.0 && max_conf < min_conf {
                    frame.gaps.push(
                        KnowledgeGap::new(
                            crate::frame::GapType::LowConfidenceOnly,
                            format!(
                                "Data exists but maximum confidence ({max_conf:.3}) is below min_confidence ({min_conf:.3})",
                            ),
                        )
                        .with_missing_entity(entity_id)
                        .with_missing_predicate(predicate),
                    );
                } else {
                    frame.gaps.push(
                        KnowledgeGap::new(
                            crate::frame::GapType::NoDataFound,
                            format!("No data found for predicate '{predicate}'"),
                        )
                        .with_missing_entity(entity_id)
                        .with_missing_predicate(predicate),
                    );
                }
            }
            return Ok(EngineResponse::Resolve { frame });
        }

        // Resolve competing beliefs if necessary.

        // Detect whether we have multiple distinct values.
        // `Value` is not Hash/Eq, so compute uniqueness via equality.
        let mut distinct_values: Vec<Value> = Vec::new();
        for b in &beliefs {
            if !distinct_values.iter().any(|v| v == &b.value) {
                distinct_values.push(b.value.clone());
            }
        }

        let (winner_id, decision) = if distinct_values.len() <= 1 {
            // No conflict; treat the best-ranked belief as selected.
            (beliefs[0].id, PolicyDecision::Selected(beliefs[0].id))
        } else {
            let decision = self.decide_with_trust(&policy, &beliefs, trust_scope);
            match decision {
                PolicyDecision::Selected(id) => (id, decision),
                PolicyDecision::Unresolved => {
                    if payload.include_gaps {
                        frame.gaps.push(
                            KnowledgeGap::new(
                                crate::frame::GapType::InsufficientEvidence,
                                "Competing beliefs exist; no resolution policy selected",
                            )
                            .with_missing_entity(entity_id)
                            .with_missing_predicate(predicate),
                        );
                    }
                    frame.debug_summary = Some(
                        "multiple competing beliefs found and conflict policy did not select a winner".to_string(),
                    );
                    // Still attach evidence + conflicts, but omit best_supported_claim.
                    (beliefs[0].id, decision)
                }
            }
        };

        let winner = beliefs
            .iter()
            .find(|b| b.id == winner_id)
            .unwrap_or(&beliefs[0]);

        let claim = RankedClaim::new(winner.clone(), self.trusted_confidence(winner, trust_scope), 1.0);

        for b in &beliefs {
            if b.value == winner.value {
                frame.supporting_evidence.push(Evidence::new(
                    b.id,
                    b.predicate.clone(),
                    b.source.clone(),
                    self.trusted_confidence(b, trust_scope),
                    1.0,
                ));
            } else if payload.include_counter_evidence {
                frame.counter_evidence.push(Evidence::new(
                    b.id,
                    b.predicate.clone(),
                    b.source.clone(),
                    self.trusted_confidence(b, trust_scope),
                    1.0,
                ));
            }

            // Attach open conflicts.
            let conflicts = self
                .conflicts
                .find_by_belief(b.id)
                .map_err(Self::storage_err)?;
            for c in conflicts {
                if c.is_open() {
                    frame.conflicts.push(c);
                }
            }
        }

        // Only set the answer if the policy selected a winner (or there was no conflict).
        if !matches!(decision, PolicyDecision::Unresolved) {
            frame.best_supported_claim = Some(claim);
        }

        Ok(EngineResponse::Resolve { frame })
    }

    fn detect_conflicts(&self, belief: &Belief, as_of: DateTime<Utc>) -> KyroResult<Vec<Conflict>> {
        let mut conflicts = Vec::new();

        // Value contradiction detection: other active beliefs with different value.
        let existing = self
            .beliefs
            .find_as_of(belief.subject, &belief.predicate, as_of)
            .map_err(Self::storage_err)?;
        for other in existing {
            if other.id == belief.id {
                continue;
            }
            // Both beliefs are already filtered by `find_as_of` at `as_of`.
            if other.value != belief.value {
                conflicts.push(Conflict::value_contradiction(
                    vec![other.id, belief.id],
                    belief.subject,
                    &belief.predicate,
                ));
            }
        }

        // Pattern checks.
        let patterns = self
            .patterns
            .find_by_predicate(&belief.predicate)
            .map_err(Self::storage_err)?;

        for pattern in patterns {
            if !pattern.active {
                continue;
            }
            if !pattern.valid_time.contains(as_of) {
                continue;
            }

            if let Some(reason) = check_pattern(&pattern.rule, belief, &self.beliefs, as_of)? {
                conflicts.push(Conflict::pattern_violation(
                    vec![belief.id],
                    belief.subject,
                    pattern.id.to_string(),
                    pattern.name,
                ));

                // Encode more detail in metadata for debugging.
                // Avoid large payloads; keep it simple.
                if let Some(last) = conflicts.last_mut() {
                    last.metadata = serde_json::json!({"reason": reason});
                }
            }
        }

        Ok(conflicts)
    }
}

fn check_pattern(
    rule: &PatternRule,
    belief: &Belief,
    belief_store: &Arc<dyn BeliefStore>,
    as_of: DateTime<Utc>,
) -> KyroResult<Option<String>> {
    match rule {
        PatternRule::Range { min, max, .. } => {
            let Some(v) = belief.value.as_float() else {
                return Ok(Some(format!(
                    "range rule requires numeric value, got {}",
                    belief.value.type_name()
                )));
            };

            if let Some(min) = min {
                if v < *min {
                    return Ok(Some(format!("value {v} is below min {min}")));
                }
            }
            if let Some(max) = max {
                if v > *max {
                    return Ok(Some(format!("value {v} is above max {max}")));
                }
            }
            Ok(None)
        }
        PatternRule::Unique { .. } => {
            let existing = belief_store
                .find_as_of(belief.subject, &belief.predicate, as_of)
                .map_err(|e| KyroError::Execution(ExecutionError::Storage {
                    message: e.to_string(),
                }))?;

            let active_count = existing
                .into_iter()
                .filter(|b| b.id != belief.id && b.is_valid_at(as_of))
                .count();

            if active_count > 0 {
                Ok(Some("unique rule violated (another active belief exists)".to_string()))
            } else {
                Ok(None)
            }
        }
        PatternRule::Cardinality { min, max, .. } => {
            let existing = belief_store
                .find_as_of(belief.subject, &belief.predicate, as_of)
                .map_err(|e| KyroError::Execution(ExecutionError::Storage {
                    message: e.to_string(),
                }))?;

            let count = existing
                .into_iter()
                .filter(|b| b.id != belief.id && b.is_valid_at(as_of))
                .count()
                + 1;

            if count < *min {
                Ok(Some(format!("cardinality {count} < min {min}")))
            } else if count > *max {
                Ok(Some(format!("cardinality {count} > max {max}")))
            } else {
                Ok(None)
            }
        }
        PatternRule::Enumerated { allowed_values, .. } => {
            let Some(s) = belief.value.as_string() else {
                return Ok(Some(format!(
                    "enumerated rule requires string value, got {}",
                    belief.value.type_name()
                )));
            };
            if allowed_values.iter().any(|v| v == s) {
                Ok(None)
            } else {
                Ok(Some(format!("'{s}' not in allowed values")))
            }
        }
        PatternRule::Regex { pattern, .. } => {
            let Some(s) = belief.value.as_string() else {
                return Ok(Some(format!(
                    "regex rule requires string value, got {}",
                    belief.value.type_name()
                )));
            };
            let re = cached_regex(pattern)?;

            if re.is_match(s) {
                Ok(None)
            } else {
                Ok(Some(format!("'{s}' does not match /{pattern}/")))
            }
        }
        PatternRule::Monotonic { direction, .. } => {
            let mut existing = belief_store
                .find_as_of(belief.subject, &belief.predicate, as_of)
                .map_err(|e| KyroError::Execution(ExecutionError::Storage {
                    message: e.to_string(),
                }))?;
            existing.sort_by(|a, b| b.tx_time.cmp(&a.tx_time));

            let Some(prev) = existing
                .into_iter()
                .find(|b| b.id != belief.id && b.is_valid_at(as_of))
            else {
                return Ok(None);
            };

            let Some(prev_v) = prev.value.as_float() else {
                return Ok(Some("monotonic rule requires numeric values".to_string()));
            };
            let Some(new_v) = belief.value.as_float() else {
                return Ok(Some("monotonic rule requires numeric values".to_string()));
            };

            match direction {
                crate::pattern::MonotonicDirection::Increasing => {
                    if new_v < prev_v {
                        Ok(Some(format!("value {new_v} decreased from {prev_v}")))
                    } else {
                        Ok(None)
                    }
                }
                crate::pattern::MonotonicDirection::Decreasing => {
                    if new_v > prev_v {
                        Ok(Some(format!("value {new_v} increased from {prev_v}")))
                    } else {
                        Ok(None)
                    }
                }
            }
        }
        PatternRule::Implication { if_predicate, then_predicate } => {
            if belief.predicate != if_predicate.trim() {
                return Ok(None);
            }
            if belief.value.as_bool() != Some(true) {
                return Ok(None);
            }
            let then = belief_store
                .find_as_of(belief.subject, then_predicate.trim(), as_of)
                .map_err(|e| KyroError::Execution(ExecutionError::Storage {
                    message: e.to_string(),
                }))?;

            let satisfied = then
                .iter()
                .any(|b| b.is_valid_at(as_of) && b.value.as_bool() == Some(true));
            if satisfied {
                Ok(None)
            } else {
                Ok(Some(format!("'{then_predicate}' is not true when '{if_predicate}' is true")))
            }
        }
        PatternRule::MutuallyExclusive { predicates } => {
            if !predicates.iter().any(|p| p.trim() == belief.predicate) {
                return Ok(None);
            }
            if belief.value.as_bool() != Some(true) {
                return Ok(None);
            }
            for p in predicates {
                let p = p.trim();
                if p == belief.predicate {
                    continue;
                }
                let others = belief_store
                    .find_as_of(belief.subject, p, as_of)
                    .map_err(|e| KyroError::Execution(ExecutionError::Storage {
                        message: e.to_string(),
                    }))?;
                if others
                    .iter()
                    .any(|b| b.is_valid_at(as_of) && b.value.as_bool() == Some(true))
                {
                    return Ok(Some(format!(
                        "'{p}' is true but predicates are mutually exclusive"
                    )));
                }
            }
            Ok(None)
        }
        PatternRule::Custom { .. } => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use uuid::Uuid;

    use crate::entity::{Entity, EntityType};
    use crate::inference::ConflictResolutionPolicy;
    use crate::ir::AssertPayload;
    use crate::source::Source;
    use crate::storage::memory::InMemoryStores;
    use crate::trust::{SimpleTrustModel, TrustModel};

    fn engine() -> (KyroEngine, EntityId) {
        let stores = InMemoryStores::new();
        let entities = Arc::new(stores.entities);
        let beliefs = Arc::new(stores.beliefs);
        let patterns = Arc::new(stores.patterns);
        let conflicts = Arc::new(stores.conflicts);
        let derivations = Arc::new(stores.derivations);

        let eng = KyroEngine::new(
            entities.clone(),
            beliefs.clone(),
            patterns.clone(),
            conflicts.clone(),
            derivations.clone(),
        );

        let entity = Entity::new("LK-99", EntityType::Concept);
        let id = entity.id;
        entities.insert(entity).unwrap();

        (eng, id)
    }

    fn engine_with_backing_stores(
    ) -> (
        KyroEngine,
        EntityId,
        Arc<crate::storage::memory::InMemoryBeliefStore>,
        Arc<crate::storage::memory::InMemoryDerivationStore>,
    ) {
        let stores = InMemoryStores::new();
        let entities = Arc::new(stores.entities);
        let beliefs = Arc::new(stores.beliefs);
        let patterns = Arc::new(stores.patterns);
        let conflicts = Arc::new(stores.conflicts);
        let derivations = Arc::new(stores.derivations);

        let eng = KyroEngine::new(
            entities.clone(),
            beliefs.clone(),
            patterns.clone(),
            conflicts.clone(),
            derivations.clone(),
        );

        let entity = Entity::new("LK-99", EntityType::Concept);
        let id = entity.id;
        entities.insert(entity).unwrap();

        (eng, id, beliefs, derivations)
    }

    fn engine_with_trust_model(trust: Arc<dyn TrustModel>) -> (KyroEngine, EntityId) {
        let stores = InMemoryStores::new();
        let entities = Arc::new(stores.entities);
        let beliefs = Arc::new(stores.beliefs);
        let patterns = Arc::new(stores.patterns);
        let conflicts = Arc::new(stores.conflicts);
        let derivations = Arc::new(stores.derivations);

        let eng = KyroEngine::with_trust_model(
            entities.clone(),
            beliefs.clone(),
            patterns.clone(),
            conflicts.clone(),
            derivations.clone(),
            trust,
        );

        let entity = Entity::new("LK-99", EntityType::Concept);
        let id = entity.id;
        entities.insert(entity).unwrap();

        (eng, id)
    }

    #[test]
    fn derive_persists_record_and_indexes_by_premise_and_derived() {
        let (eng, id, _beliefs, derivations) = engine_with_backing_stores();

        let p1 = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id: id,
            predicate: "premise_a".to_string(),
            value: Value::Bool(true),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: Source::agent("a", Option::<String>::None),
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Force,
            embedding: None,
        }));
        let EngineResponse::Assert { belief_id: b1, .. } = eng.execute(p1).unwrap() else {
            panic!("expected assert");
        };

        let p2 = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id: id,
            predicate: "premise_b".to_string(),
            value: Value::Bool(true),
            confidence: Confidence::from_agent(0.8, "a").unwrap(),
            source: Source::agent("a", Option::<String>::None),
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Force,
            embedding: None,
        }));
        let EngineResponse::Assert { belief_id: b2, .. } = eng.execute(p2).unwrap() else {
            panic!("expected assert");
        };

        let derived_assert = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id: id,
            predicate: "conclusion".to_string(),
            value: Value::Bool(true),
            confidence: Confidence::from_agent(0.7, "a").unwrap(),
            source: Source::derived(vec![b1, b2], "modus_ponens"),
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Force,
            embedding: None,
        }));
        let EngineResponse::Assert {
            belief_id: derived_id,
            ..
        } = eng.execute(derived_assert).unwrap() else {
            panic!("expected assert");
        };

        let derive_ir = KyroIR::new(Operation::Derive(DerivePayload {
            rule: Some("modus_ponens".to_string()),
            derived_belief_id: Some(derived_id),
            sources: Some(vec![b1, b2]),
            inference_steps: Some(vec!["if A then B".to_string()]),
            confidence: Some(0.7),
            justification: Some("A is true; therefore B".to_string()),
            metadata: Some(serde_json::json!({"engine": "test"})),
        }));

        let EngineResponse::Derive { derivation_id } = eng.execute(derive_ir).unwrap() else {
            panic!("expected derive");
        };

        let stored = derivations.get(derivation_id).unwrap().unwrap();
        assert_eq!(stored.derived_belief_id, Some(derived_id));
        assert_eq!(stored.premise_ids, vec![b1, b2]);
        assert_eq!(stored.rule, "modus_ponens");

        let by_premise = derivations.find_by_premise(b1).unwrap();
        assert!(by_premise.iter().any(|r| r.id == derivation_id));

        let by_derived = derivations.find_by_derived_belief(derived_id).unwrap();
        assert!(by_derived.iter().any(|r| r.id == derivation_id));
    }

    #[test]
    fn assert_then_resolve_returns_answer() {
        let (eng, id) = engine();

        let ir = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id: id,
            predicate: "temperature".to_string(),
            value: Value::Float(25.0),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: Source::agent("a", Option::<String>::None),
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Force,
            embedding: None,
        }));

        let resp = eng.execute(ir).unwrap();
        let EngineResponse::Assert { .. } = resp else { panic!("expected assert"); };

        let resolve = KyroIR::new(Operation::Resolve(ResolvePayload {
            entity_id: Some(id),
            predicate: Some("temperature".to_string()),
            ..ResolvePayload::default()
        }));

        let EngineResponse::Resolve { frame } = eng.execute(resolve).unwrap() else { panic!("expected resolve"); };
        assert!(frame.has_answer());
        assert_eq!(frame.best_supported_claim.unwrap().belief.value, Value::Float(25.0));
    }

    #[test]
    fn eventual_mode_records_value_contradiction_conflict() {
        let (eng, id) = engine();

        let first = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id: id,
            predicate: "is_superconductor".to_string(),
            value: Value::Bool(false),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: Source::agent("a", Option::<String>::None),
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Force,
            embedding: None,
        }));
        eng.execute(first).unwrap();

        let second = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id: id,
            predicate: "is_superconductor".to_string(),
            value: Value::Bool(true),
            confidence: Confidence::from_agent(0.8, "b").unwrap(),
            source: Source::agent("b", Option::<String>::None),
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Eventual,
            embedding: None,
        }));

        let EngineResponse::Assert { conflict_ids, .. } = eng.execute(second).unwrap() else { panic!("expected assert"); };
        assert!(!conflict_ids.is_empty());
    }

    #[test]
    fn strict_mode_rejects_value_contradictions() {
        let (eng, id) = engine();

        let first = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id: id,
            predicate: "is_superconductor".to_string(),
            value: Value::Bool(false),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: Source::agent("a", Option::<String>::None),
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Force,
            embedding: None,
        }));
        eng.execute(first).unwrap();

        let strict = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id: id,
            predicate: "is_superconductor".to_string(),
            value: Value::Bool(true),
            confidence: Confidence::from_agent(0.8, "b").unwrap(),
            source: Source::agent("b", Option::<String>::None),
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Strict,
            embedding: None,
        }));

        let err = eng.execute(strict).unwrap_err();
        let KyroError::Execution(ExecutionError::ConflictsDetected { conflicts }) = err else {
            panic!("expected ConflictsDetected, got {err:?}");
        };
        assert!(!conflicts.is_empty());
    }

    #[test]
    fn mutually_exclusive_pattern_is_checked_for_all_predicates() {
        let (eng, id) = engine();

        let define = KyroIR::new(Operation::DefinePattern(DefinePatternPayload {
            name: "mut_ex".to_string(),
            description: None,
            rule: PatternRule::MutuallyExclusive {
                predicates: vec!["p1".to_string(), "p2".to_string()],
            },
            confidence: Confidence::from_agent(0.8, "a").unwrap(),
            valid_time: TimeRange::forever(),
        }));
        eng.execute(define).unwrap();

        let a1 = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id: id,
            predicate: "p1".to_string(),
            value: Value::Bool(true),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: Source::agent("a", Option::<String>::None),
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Force,
            embedding: None,
        }));
        eng.execute(a1).unwrap();

        let a2 = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id: id,
            predicate: "p2".to_string(),
            value: Value::Bool(true),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: Source::agent("a", Option::<String>::None),
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Eventual,
            embedding: None,
        }));
        let EngineResponse::Assert { conflict_ids, .. } = eng.execute(a2).unwrap() else {
            panic!("expected assert");
        };
        assert!(!conflict_ids.is_empty());
    }

    #[test]
    fn strict_mode_rejects_range_pattern_violation() {
        let (eng, id) = engine();

        let define = KyroIR::new(Operation::DefinePattern(DefinePatternPayload {
            name: "temp_range".to_string(),
            description: None,
            rule: PatternRule::Range {
                predicate: "temperature".to_string(),
                min: Some(0.0),
                max: Some(100.0),
            },
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            valid_time: TimeRange::forever(),
        }));
        eng.execute(define).unwrap();

        let bad = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id: id,
            predicate: "temperature".to_string(),
            value: Value::Float(-5.0),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: Source::agent("a", Option::<String>::None),
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Strict,
            embedding: None,
        }));

        let err = eng.execute(bad).unwrap_err();
        let KyroError::Execution(ExecutionError::ConflictsDetected { conflicts }) = err else {
            panic!("expected ConflictsDetected, got {err:?}");
        };
        assert!(conflicts.iter().any(|c| c.starts_with("pattern_violation")));
    }

    #[test]
    fn eventual_mode_records_regex_pattern_violation() {
        let (eng, id) = engine();

        let define = KyroIR::new(Operation::DefinePattern(DefinePatternPayload {
            name: "email_format".to_string(),
            description: None,
            rule: PatternRule::Regex {
                predicate: "email".to_string(),
                pattern: r"^[^@]+@[^@]+\.[^@]+$".to_string(),
            },
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            valid_time: TimeRange::forever(),
        }));
        eng.execute(define).unwrap();

        let bad = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id: id,
            predicate: "email".to_string(),
            value: Value::String("not-an-email".to_string()),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: Source::agent("a", Option::<String>::None),
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Eventual,
            embedding: None,
        }));

        let EngineResponse::Assert { conflict_ids, .. } = eng.execute(bad).unwrap() else {
            panic!("expected assert");
        };
        assert!(!conflict_ids.is_empty());
    }

    #[test]
    fn strict_mode_rejects_unique_violation() {
        let (eng, id) = engine();

        let define = KyroIR::new(Operation::DefinePattern(DefinePatternPayload {
            name: "unique_ssn".to_string(),
            description: None,
            rule: PatternRule::Unique {
                predicate: "ssn".to_string(),
            },
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            valid_time: TimeRange::forever(),
        }));
        eng.execute(define).unwrap();

        let first = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id: id,
            predicate: "ssn".to_string(),
            value: Value::String("123-45-6789".to_string()),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: Source::agent("a", Option::<String>::None),
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Force,
            embedding: None,
        }));
        eng.execute(first).unwrap();

        let second = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id: id,
            predicate: "ssn".to_string(),
            value: Value::String("123-45-6789".to_string()),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: Source::agent("a", Option::<String>::None),
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Strict,
            embedding: None,
        }));

        let err = eng.execute(second).unwrap_err();
        let KyroError::Execution(ExecutionError::ConflictsDetected { conflicts }) = err else {
            panic!("expected ConflictsDetected, got {err:?}");
        };
        assert!(conflicts.iter().any(|c| c.starts_with("pattern_violation")));
    }

    #[test]
    fn retract_closes_old_belief_and_persists_retraction_state() {
        use chrono::Duration;

        let (eng, id, belief_store, _derivations) = engine_with_backing_stores();

        let t0 = Utc::now();
        let t1 = t0 + Duration::seconds(5);
        let t2 = t0 + Duration::seconds(10);

        let assert_ir = KyroIR {
            version: KyroIR::CURRENT_VERSION.to_string(),
            request_id: Uuid::new_v4(),
            timestamp: t1,
            operation: Operation::Assert(crate::ir::AssertPayload {
                entity_id: id,
                predicate: "status".to_string(),
                value: Value::String("active".to_string()),
                confidence: Confidence::from_agent(0.9, "a").unwrap(),
                source: Source::agent("a", Option::<String>::None),
                valid_time: TimeRange::starting_at(t0),
                consistency_mode: ConsistencyMode::Force,
                embedding: None,
            }),
        };

        let EngineResponse::Assert { belief_id, .. } = eng.execute(assert_ir).unwrap() else {
            panic!("expected assert");
        };

        let retract_ir = KyroIR {
            version: KyroIR::CURRENT_VERSION.to_string(),
            request_id: Uuid::new_v4(),
            timestamp: t2,
            operation: Operation::Retract(RetractPayload {
                belief_id,
                reason: Some("no longer true".to_string()),
                authorized_by: Source::agent("system", Option::<String>::None),
            }),
        };

        let EngineResponse::Retract {
            retraction_belief_id,
        } = eng.execute(retract_ir).unwrap()
        else {
            panic!("expected retract");
        };

        // Old belief is closed at (or before) the retraction tx_time.
        let old = belief_store.get(belief_id).unwrap().unwrap();
        assert_eq!(old.superseded_by, Some(retraction_belief_id));
        assert!(old.valid_time.to().is_some());
        assert!(old.valid_time.to().unwrap() <= t2);

        // As-of before retract sees the original value.
        let resolve_before = KyroIR {
            version: KyroIR::CURRENT_VERSION.to_string(),
            request_id: Uuid::new_v4(),
            timestamp: t2,
            operation: Operation::Resolve(ResolvePayload {
                entity_id: Some(id),
                predicate: Some("status".to_string()),
                as_of: Some(t1 + Duration::seconds(1)),
                ..ResolvePayload::default()
            }),
        };

        let EngineResponse::Resolve { frame } = eng.execute(resolve_before).unwrap() else {
            panic!("expected resolve");
        };
        assert_eq!(
            frame.best_supported_claim.unwrap().belief.value,
            Value::String("active".to_string())
        );

        // As-of after retract sees the retraction state (Null).
        let resolve_after = KyroIR {
            version: KyroIR::CURRENT_VERSION.to_string(),
            request_id: Uuid::new_v4(),
            timestamp: t2,
            operation: Operation::Resolve(ResolvePayload {
                entity_id: Some(id),
                predicate: Some("status".to_string()),
                as_of: Some(t2 + Duration::seconds(1)),
                ..ResolvePayload::default()
            }),
        };

        let EngineResponse::Resolve { frame } = eng.execute(resolve_after).unwrap() else {
            panic!("expected resolve");
        };
        assert_eq!(frame.best_supported_claim.unwrap().belief.value, Value::Null);
    }

    #[test]
    fn resolve_explicit_conflict_omits_answer() {
        let (eng, id) = engine();

        eng.execute(KyroIR::new(Operation::Assert(AssertPayload {
            entity_id: id,
            predicate: "status".to_string(),
            value: Value::String("on".to_string()),
            confidence: Confidence::from_agent(0.7, "a").unwrap(),
            source: Source::agent("a", Option::<String>::None),
            valid_time: TimeRange::forever(),
            consistency_mode: ConsistencyMode::Eventual,
            embedding: None,
        })))
        .unwrap();

        eng.execute(KyroIR::new(Operation::Assert(AssertPayload {
            entity_id: id,
            predicate: "status".to_string(),
            value: Value::String("off".to_string()),
            confidence: Confidence::from_agent(0.8, "b").unwrap(),
            source: Source::agent("b", Option::<String>::None),
            valid_time: TimeRange::forever(),
            consistency_mode: ConsistencyMode::Eventual,
            embedding: None,
        })))
        .unwrap();

        let resolve = KyroIR::new(Operation::Resolve(ResolvePayload {
            entity_id: Some(id),
            predicate: Some("status".to_string()),
            conflict_policy: Some(ConflictResolutionPolicy::ExplicitConflict),
            ..ResolvePayload::default()
        }));

        let EngineResponse::Resolve { frame } = eng.execute(resolve).unwrap() else {
            panic!("expected resolve");
        };

        assert!(frame.best_supported_claim.is_none());
        assert!(frame.has_conflicts());
        assert!(frame.has_gaps());
        assert_eq!(frame.query_assumptions.trust_model, "simple_trust");
    }

    #[test]
    fn resolve_latest_wins_selects_newest_tx_time() {
        let (eng, id, belief_store, _derivations) = engine_with_backing_stores();

        let t0 = Utc::now();
        let old = Belief {
            id: BeliefId::new(),
            subject: id,
            predicate: "status".to_string(),
            value: Value::String("old".to_string()),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: Source::agent("a", Option::<String>::None),
            valid_time: TimeRange::forever(),
            tx_time: t0,
            reason: None,
            consistency_status: ConsistencyStatus::Verified,
            supersedes: None,
            superseded_by: None,
            embedding: None,
        };

        let new = Belief {
            id: BeliefId::new(),
            subject: id,
            predicate: "status".to_string(),
            value: Value::String("new".to_string()),
            confidence: Confidence::from_agent(0.1, "b").unwrap(),
            source: Source::agent("b", Option::<String>::None),
            valid_time: TimeRange::forever(),
            tx_time: t0 + chrono::Duration::seconds(5),
            reason: None,
            consistency_status: ConsistencyStatus::Verified,
            supersedes: None,
            superseded_by: None,
            embedding: None,
        };

        belief_store.insert(old).unwrap();
        belief_store.insert(new).unwrap();

        let resolve = KyroIR::new(Operation::Resolve(ResolvePayload {
            entity_id: Some(id),
            predicate: Some("status".to_string()),
            as_of: Some(t0 + chrono::Duration::seconds(10)),
            conflict_policy: Some(ConflictResolutionPolicy::LatestWins),
            ..ResolvePayload::default()
        }));

        let EngineResponse::Resolve { frame } = eng.execute(resolve).unwrap() else {
            panic!("expected resolve");
        };

        assert_eq!(
            frame.best_supported_claim.unwrap().belief.value,
            Value::String("new".to_string())
        );
        assert_eq!(frame.query_assumptions.trust_model, "simple_trust");
    }

    #[test]
    fn resolve_trust_domain_defaults_to_predicate_and_affects_ranking() {
        let model = Arc::new(SimpleTrustModel::new());
        let source_a = Source::agent("a", Option::<String>::None);
        let source_b = Source::agent("b", Option::<String>::None);

        // In the "status" domain, downweight A and leave B at 1.0.
        model.set_domain("status", source_a.source_id(), 0.0);
        model.set_domain("status", source_b.source_id(), 1.0);

        let (eng, id) = engine_with_trust_model(model);

        // Without trust weighting, A would win (0.9 > 0.2).
        // With trust weighting in the default scope (predicate "status"), B should win.
        eng.execute(KyroIR::new(Operation::Assert(AssertPayload {
            entity_id: id,
            predicate: "status".to_string(),
            value: Value::String("off".to_string()),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: source_a.clone(),
            valid_time: TimeRange::forever(),
            consistency_mode: ConsistencyMode::Eventual,
            embedding: None,
        })))
        .unwrap();

        eng.execute(KyroIR::new(Operation::Assert(AssertPayload {
            entity_id: id,
            predicate: "status".to_string(),
            value: Value::String("on".to_string()),
            confidence: Confidence::from_agent(0.2, "b").unwrap(),
            source: source_b.clone(),
            valid_time: TimeRange::forever(),
            consistency_mode: ConsistencyMode::Eventual,
            embedding: None,
        })))
        .unwrap();

        let resolve = KyroIR::new(Operation::Resolve(ResolvePayload {
            entity_id: Some(id),
            predicate: Some("status".to_string()),
            // trust_domain intentionally omitted: should default to predicate.
            ..ResolvePayload::default()
        }));

        let EngineResponse::Resolve { frame } = eng.execute(resolve).unwrap() else {
            panic!("expected resolve");
        };

        assert_eq!(
            frame.best_supported_claim.unwrap().belief.value,
            Value::String("on".to_string())
        );
    }

    #[test]
    fn resolve_trust_domain_override_changes_scope() {
        let model = Arc::new(SimpleTrustModel::new());
        let source_a = Source::agent("a", Option::<String>::None);
        let source_b = Source::agent("b", Option::<String>::None);

        // Domain overrides apply only when that domain is selected.
        model.set_domain("status", source_a.source_id(), 0.0);
        model.set_domain("status", source_b.source_id(), 1.0);

        let (eng, id) = engine_with_trust_model(model);

        eng.execute(KyroIR::new(Operation::Assert(AssertPayload {
            entity_id: id,
            predicate: "status".to_string(),
            value: Value::String("off".to_string()),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: source_a.clone(),
            valid_time: TimeRange::forever(),
            consistency_mode: ConsistencyMode::Eventual,
            embedding: None,
        })))
        .unwrap();

        eng.execute(KyroIR::new(Operation::Assert(AssertPayload {
            entity_id: id,
            predicate: "status".to_string(),
            value: Value::String("on".to_string()),
            confidence: Confidence::from_agent(0.2, "b").unwrap(),
            source: source_b.clone(),
            valid_time: TimeRange::forever(),
            consistency_mode: ConsistencyMode::Eventual,
            embedding: None,
        })))
        .unwrap();

        // Force a different trust domain that has no overrides => both weights default to 1.0.
        let resolve = KyroIR::new(Operation::Resolve(ResolvePayload {
            entity_id: Some(id),
            predicate: Some("status".to_string()),
            trust_domain: Some("other".to_string()),
            ..ResolvePayload::default()
        }));

        let EngineResponse::Resolve { frame } = eng.execute(resolve).unwrap() else {
            panic!("expected resolve");
        };

        assert_eq!(
            frame.best_supported_claim.unwrap().belief.value,
            Value::String("off".to_string())
        );
    }
}
