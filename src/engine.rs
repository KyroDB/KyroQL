//! Execution engine for KyroQL IR.
//!
//! This module provides a synchronous executor that applies operations (`KyroIR`) against
//! pluggable storage backends.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::{OnceLock, RwLock};

use chrono::{DateTime, Utc};

use crate::belief::{Belief, ConsistencyStatus};
use crate::confidence::{BeliefId, Confidence};
use crate::conflict::{Conflict, ConflictId};
use crate::entity::{EntityId};
use crate::error::{ExecutionError, KyroError, KyroResult, ValidationError};
use crate::frame::{BeliefFrame, Evidence, KnowledgeGap, RankedClaim};
use crate::ir::{ConsistencyMode, DefinePatternPayload, KyroIR, Operation, ResolvePayload, RetractPayload};
use crate::pattern::{Pattern, PatternId, PatternRule};
use crate::storage::{BeliefStore, ConflictStore, EntityStore, PatternStore, StorageError};
use crate::time::TimeRange;
use crate::value::Value;

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
#[derive(Debug, Clone, PartialEq)]
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
}

/// KyroQL execution engine.
#[derive(Clone)]
pub struct KyroEngine {
    entities: Arc<dyn EntityStore>,
    beliefs: Arc<dyn BeliefStore>,
    patterns: Arc<dyn PatternStore>,
    conflicts: Arc<dyn ConflictStore>,
}

impl KyroEngine {
    /// Create a new engine using the given stores.
    #[must_use]
    pub fn new(
        entities: Arc<dyn EntityStore>,
        beliefs: Arc<dyn BeliefStore>,
        patterns: Arc<dyn PatternStore>,
        conflicts: Arc<dyn ConflictStore>,
    ) -> Self {
        Self {
            entities,
            beliefs,
            patterns,
            conflicts,
        }
    }

    /// Execute a KyroQL IR request.
    pub fn execute(&self, ir: KyroIR) -> KyroResult<EngineResponse> {
        match ir.operation {
            Operation::Assert(payload) => self.execute_assert(ir.timestamp, payload.consistency_mode, payload.entity_id, payload.predicate, payload.value, payload.confidence, payload.source, payload.valid_time, payload.embedding),
            Operation::Resolve(payload) => self.execute_resolve(payload),
            Operation::Retract(payload) => self.execute_retract(ir.timestamp, payload),
            Operation::DefinePattern(payload) => self.execute_define_pattern(payload),
        }
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
        let entity_id = payload.entity_id.ok_or_else(|| {
            KyroError::Execution(ExecutionError::Index {
                message: "resolve currently requires entity_id".to_string(),
            })
        })?;
        let predicate = payload.predicate.as_deref().ok_or_else(|| {
            KyroError::Execution(ExecutionError::Index {
                message: "resolve currently requires predicate".to_string(),
            })
        })?;

        self.ensure_entity_exists(entity_id)?;

        let as_of = payload.as_of.unwrap_or_else(Utc::now);
        let all = self
            .beliefs
            .find_as_of(entity_id, predicate, as_of)
            .map_err(Self::storage_err)?;

        let max_conf = all
            .iter()
            .map(|b| b.confidence.value())
            .fold(0.0f32, f32::max);

        let min_conf = payload.min_confidence.unwrap_or(0.0).clamp(0.0, 1.0);
        let mut beliefs: Vec<Belief> = all
            .into_iter()
            .filter(|b| b.confidence.value() >= min_conf)
            .collect();

        beliefs.sort_by(|a, b| b.confidence.value().total_cmp(&a.confidence.value()));
        beliefs.truncate(payload.limit);

        let mut frame = BeliefFrame::empty();
        frame.time_window = TimeRange::instant(as_of);
        frame.query_assumptions.assumed_time = Some(as_of);
        frame.query_assumptions.resolved_entity = Some(entity_id);

        if beliefs.is_empty() {
            if payload.include_gaps {
                if max_conf > 0.0 && max_conf < min_conf {
                    frame.gaps.push(KnowledgeGap::low_confidence(entity_id, max_conf));
                } else {
                    frame.gaps.push(KnowledgeGap::no_predicate(entity_id, predicate));
                }
            }
            return Ok(EngineResponse::Resolve { frame });
        }

        let best = &beliefs[0];
        frame.epistemic_confidence = best.confidence.value();
        frame.retrieval_relevance = 1.0;

        let mut claim = RankedClaim::new(best.value.clone(), best.confidence.clone(), 1);

        for b in &beliefs {
            if b.value == best.value {
                claim.supporting_belief_ids.push(b.id);
                frame.supporting_evidence.push(Evidence::supporting(
                    b.id,
                    b.value.clone(),
                    b.confidence.clone(),
                ));
            } else if payload.include_counter_evidence {
                frame.counter_evidence.push(Evidence::counter(
                    b.id,
                    b.value.clone(),
                    b.confidence.clone(),
                ));
            }

            // Attach open conflicts.
            let conflicts = self
                .conflicts
                .find_by_belief(b.id)
                .map_err(Self::storage_err)?;
            for c in conflicts {
                if c.is_open() {
                    frame.conflicts.push(c.id);
                }
            }
        }

        frame.best_supported_claim = Some(claim);

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
    use crate::source::Source;
    use crate::storage::memory::InMemoryStores;

    fn engine() -> (KyroEngine, EntityId) {
        let stores = InMemoryStores::new();
        let entities = Arc::new(stores.entities);
        let beliefs = Arc::new(stores.beliefs);
        let patterns = Arc::new(stores.patterns);
        let conflicts = Arc::new(stores.conflicts);

        let eng = KyroEngine::new(entities.clone(), beliefs.clone(), patterns.clone(), conflicts.clone());

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
    ) {
        let stores = InMemoryStores::new();
        let entities = Arc::new(stores.entities);
        let beliefs = Arc::new(stores.beliefs);
        let patterns = Arc::new(stores.patterns);
        let conflicts = Arc::new(stores.conflicts);

        let eng = KyroEngine::new(
            entities.clone(),
            beliefs.clone(),
            patterns.clone(),
            conflicts.clone(),
        );

        let entity = Entity::new("LK-99", EntityType::Concept);
        let id = entity.id;
        entities.insert(entity).unwrap();

        (eng, id, beliefs)
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
        assert_eq!(frame.best_supported_claim.unwrap().value, Value::Float(25.0));
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

        let (eng, id, belief_store) = engine_with_backing_stores();

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
        assert!(old.valid_time.to.is_some());
        assert!(old.valid_time.to.unwrap() <= t2);

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
        assert_eq!(frame.best_supported_claim.unwrap().value, Value::String("active".to_string()));

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
        assert_eq!(frame.best_supported_claim.unwrap().value, Value::Null);
    }
}
