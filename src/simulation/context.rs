//! Simulation context with hard timeout and deterministic teardown.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::belief::Belief;
use crate::confidence::BeliefId;
use crate::derivation::{DerivationId};
use crate::engine::{EngineResponse, KyroEngine};
use crate::error::{ExecutionError, KyroError, KyroResult};
use crate::frame::BeliefFrame;
use crate::ir::{ConsistencyMode, DerivePayload, KyroIR, Operation, ResolvePayload};
use crate::storage::StorageError;
use crate::storage::DerivationStore;

use crate::entity::EntityId;
use crate::conflict::ConflictId;

use super::constraints::SimulateConstraints;
use super::delta_index::DeltaVectorIndex;
use super::delta_store::DeltaStore;
use super::SimulationBaseStores;

fn storage_err(err: StorageError) -> KyroError {
    KyroError::Execution(ExecutionError::Storage {
        message: err.to_string(),
    })
}

/// Summary of changes within a simulation overlay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationImpact {
    /// Unique entities affected by the simulation.
    pub affected_entities: Vec<EntityId>,
    /// Total number of beliefs inserted into the overlay.
    pub inserted_beliefs: usize,

    /// IDs of beliefs inserted into the overlay.
    pub inserted_belief_ids: Vec<BeliefId>,

    /// Supersede pairs recorded in the overlay (old_id -> new_id).
    pub supersedes: Vec<(BeliefId, BeliefId)>,
}

/// Result of committing a simulation overlay into base storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationCommitResult {
    /// How many overlay beliefs were committed.
    pub committed_beliefs: usize,
    /// Mapping from hypothetical belief IDs (overlay) to committed belief IDs (base).
    pub belief_id_map: Vec<(BeliefId, BeliefId)>,
    /// All conflict IDs produced during commit (may be empty in strict/force modes).
    pub conflict_ids: Vec<ConflictId>,
}

/// Stable identifier for a simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SimulationId(Uuid);

impl SimulationId {
    /// Create a new random simulation ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SimulationId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SimulationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// In-memory simulation context.
///
/// This type is intentionally conservative:
/// - Enforces a hard wall-clock timeout.
/// - Provides deterministic teardown via `Drop`.
/// - Tracks resource usage counters for constraint enforcement.
pub struct SimulationContext {
    /// Simulation identity.
    pub id: SimulationId,
    constraints: SimulateConstraints,
    nesting_level: usize,
    remaining_depth: usize,
    created_at: Instant,
    deadline: Instant,
    hypothetical_count: AtomicUsize,
    is_dropped: AtomicBool,
    is_committed: AtomicBool,

    // Only root simulations retain a writable commit base.
    // Nested simulations are layered on top of parent overlays and must not be able to
    // mutate the root substrate via a commit.
    commit_base: Option<SimulationBaseStores>,

    // Overlay state (implemented in later steps).
    pub(crate) delta_store: DeltaStore,
    pub(crate) delta_index: DeltaVectorIndex,

    // Derivations recorded inside this simulation.
    // This store is intentionally simulation-local (no base mutation).
    derivations: std::sync::Arc<dyn DerivationStore>,
}

impl fmt::Debug for SimulationContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SimulationContext")
            .field("id", &self.id)
            .field("constraints", &self.constraints)
            .field("nesting_level", &self.nesting_level)
            .field("remaining_depth", &self.remaining_depth)
            .field("created_at", &self.created_at)
            .field("deadline", &self.deadline)
            .field(
                "hypothetical_count",
                &self.hypothetical_count.load(Ordering::Relaxed),
            )
            .field("is_dropped", &self.is_dropped.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl SimulationContext {
    /// Create a new simulation context.
    pub fn new(base: SimulationBaseStores, constraints: SimulateConstraints) -> KyroResult<Self> {
        Self::new_internal(base, constraints, 0, None, true)
    }

    fn new_internal(
        base: SimulationBaseStores,
        constraints: SimulateConstraints,
        nesting_level: usize,
        deadline_cap: Option<Instant>,
        retain_commit_base: bool,
    ) -> KyroResult<Self> {
        constraints.validate().map_err(KyroError::from)?;

        let created_at = Instant::now();
        let timeout = Duration::from_millis(constraints.max_duration_ms);
        let computed_deadline = created_at
            .checked_add(timeout)
            .ok_or_else(|| KyroError::Validation(crate::error::ValidationError::InvalidSimulationConstraints {
                reason: "max_duration_ms overflow".to_string(),
            }))?;

        let deadline = match deadline_cap {
            None => computed_deadline,
            Some(cap) => if computed_deadline < cap { computed_deadline } else { cap },
        };

        let remaining_depth = constraints.max_depth.saturating_sub(nesting_level);

        let commit_base = if retain_commit_base {
            Some(base.clone())
        } else {
            None
        };

        Ok(Self {
            id: SimulationId::new(),
            constraints,
            nesting_level,
            remaining_depth,
            created_at,
            deadline,
            hypothetical_count: AtomicUsize::new(0),
            is_dropped: AtomicBool::new(false),
            is_committed: AtomicBool::new(false),
            commit_base,
            delta_store: DeltaStore::new(base, constraints),
            delta_index: DeltaVectorIndex::new(),
            derivations: std::sync::Arc::new(crate::storage::InMemoryDerivationStore::default()),
        })
    }

    /// Spawn a child simulation layered on top of this simulation.
    ///
    /// The child sees this simulation's hypotheticals (and base state), but any
    /// child writes are isolated to the child's own overlay and cannot mutate the
    /// parent overlay.
    pub fn spawn_child(&self) -> KyroResult<Self> {
        self.ensure_not_expired()?;

        if self.remaining_depth < 1 {
            // nesting_level = max_depth - remaining_depth
            // actual_value reports the attempted nesting level (current + 1).
            let nesting_level = self
                .constraints
                .max_depth
                .saturating_sub(self.remaining_depth);
            return Err(KyroError::Execution(ExecutionError::SimulationLimitExceeded {
                limit_type: "nesting_depth".to_string(),
                max_value: self.constraints.max_depth as u64,
                actual_value: nesting_level.saturating_add(1) as u64,
            }));
        }

        let base = SimulationBaseStores {
            entities: self.delta_store.entities(),
            beliefs: self.delta_store.beliefs(),
            patterns: self.delta_store.patterns(),
            conflicts: self.delta_store.conflicts(),
        };

        Self::new_internal(base, self.constraints, self.nesting_level + 1, Some(self.deadline), false)
    }

    /// Commit this simulation's overlay into base storage.
    ///
    /// Semantics (explicit):
    /// - Commit is only allowed for a root simulation (non-nested).
    /// - Commit uses the provided `engine` to execute ASSERTs so that pattern/conflict checks
    ///   and MONITOR observations behave identically to normal writes.
    /// - Hypothetical belief IDs are not preserved: the engine assigns fresh `BeliefId`s.
    ///   A mapping is returned.
    /// - The commit is not transactional across multiple beliefs (the current storage traits
    ///   do not support rollback). The method preflights invariants to reduce partial-commit risk.
    ///   If a write fails mid-stream, a `SimulationPartialCommit` error is returned containing the
    ///   belief ID mapping and conflict IDs accumulated so far so callers can reconcile/roll forward
    ///   or compensate.
    pub fn commit_overlay(&self, engine: &KyroEngine, mode: ConsistencyMode) -> KyroResult<SimulationCommitResult> {
        self.ensure_not_expired()?;

        if self.nesting_level != 0 || self.commit_base.is_none() {
            return Err(KyroError::Execution(ExecutionError::SimulationCommitNotAllowed {
                reason: "commit_overlay is only supported for root simulations".to_string(),
            }));
        }

        if self.is_committed.swap(true, Ordering::AcqRel) {
            return Err(KyroError::Execution(ExecutionError::SimulationCommitNotAllowed {
                reason: "simulation overlay was already committed".to_string(),
            }));
        }

        let base = self.commit_base.as_ref().expect("checked above");
        let (overlay_beliefs, supersedes) = self.delta_store.overlay_snapshot().map_err(storage_err)?;

        if overlay_beliefs.is_empty() {
            return Ok(SimulationCommitResult {
                committed_beliefs: 0,
                belief_id_map: Vec::new(),
                conflict_ids: Vec::new(),
            });
        }

        // Preflight: ensure all subjects exist in base and supersede pairs reference known IDs.
        for belief in &overlay_beliefs {
            if base.entities.get(belief.subject).map_err(storage_err)?.is_none() {
                return Err(KyroError::Execution(ExecutionError::EntityNotFound {
                    id: belief.subject,
                }));
            }
        }

        let overlay_ids: std::collections::HashSet<BeliefId> =
            overlay_beliefs.iter().map(|b| b.id).collect();

        for (old_id, new_id) in &supersedes {
            if !overlay_ids.contains(new_id) {
                return Err(KyroError::Execution(ExecutionError::SimulationCommitNotAllowed {
                    reason: format!("supersede references non-overlay new_id={new_id}"),
                }));
            }

            if !overlay_ids.contains(old_id) {
                // old_id must exist in base if it is not an overlay belief.
                if base.beliefs.get(*old_id).map_err(storage_err)?.is_none() {
                    return Err(KyroError::Execution(ExecutionError::BeliefNotFound { id: *old_id }));
                }
            }
        }

        let tx_time = chrono::Utc::now();

        let mut belief_id_map: Vec<(BeliefId, BeliefId)> = Vec::with_capacity(overlay_beliefs.len());
        let mut conflict_ids: Vec<ConflictId> = Vec::new();

        for belief in overlay_beliefs {
            let old_id = belief.id;

            let payload = crate::ir::AssertPayload {
                entity_id: belief.subject,
                predicate: belief.predicate,
                value: belief.value,
                confidence: belief.confidence,
                source: belief.source,
                valid_time: belief.valid_time,
                consistency_mode: mode,
                embedding: belief.embedding,
            };

            let ir = KyroIR {
                version: KyroIR::CURRENT_VERSION.to_string(),
                request_id: uuid::Uuid::new_v4(),
                timestamp: tx_time,
                operation: Operation::Assert(payload),
            };

            let resp = match engine.execute(ir) {
                Ok(resp) => resp,
                Err(err) => {
                    return Err(KyroError::Execution(ExecutionError::SimulationPartialCommit {
                        committed_len: belief_id_map.len(),
                        committed: belief_id_map,
                        conflict_ids,
                        failed_belief_id: old_id,
                        cause: Box::new(err),
                    }));
                }
            };
            let EngineResponse::Assert { belief_id: new_id, conflict_ids: ids } = resp else {
                return Err(KyroError::Execution(ExecutionError::InvalidOperation {
                    expected: "engine_response.assert".to_string(),
                    actual: format!("{resp:?}"),
                }));
            };

            belief_id_map.push((old_id, new_id));
            conflict_ids.extend(ids);
        }

        // Apply supersede relations in base storage.
        // Note: the engine assigns fresh IDs, so we remap pairs using the returned mapping.
        let map: std::collections::HashMap<BeliefId, BeliefId> = belief_id_map.iter().copied().collect();
        for (old_id, new_id) in supersedes {
            let mapped_new = map
                .get(&new_id)
                .copied()
                .ok_or_else(|| KyroError::Execution(ExecutionError::SimulationCommitNotAllowed {
                    reason: format!("missing committed mapping for new_id={new_id}"),
                }))?;

            let mapped_old = map.get(&old_id).copied().unwrap_or(old_id);
            base.beliefs.supersede(mapped_old, mapped_new).map_err(storage_err)?;
        }

        Ok(SimulationCommitResult {
            committed_beliefs: belief_id_map.len(),
            belief_id_map,
            conflict_ids,
        })
    }

    /// Returns the constraints for this simulation.
    #[must_use]
    pub const fn constraints(&self) -> SimulateConstraints {
        self.constraints
    }

    /// Returns the time elapsed since creation.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.created_at.elapsed()
    }

    /// Enforce timeout.
    pub fn ensure_not_expired(&self) -> KyroResult<()> {
        if self.is_dropped.load(Ordering::Acquire) {
            return Err(KyroError::Execution(ExecutionError::SimulationNotFound {
                id: self.id.to_string(),
            }));
        }

        if Instant::now() > self.deadline {
            return Err(KyroError::Execution(ExecutionError::Timeout {
                duration_ms: self.constraints.max_duration_ms,
            }));
        }

        Ok(())
    }

    /// Increment the hypothetical operation count and enforce basic limits.
    pub fn register_hypothetical(&self) -> KyroResult<()> {
        self.ensure_not_expired()?;

        // This is a coarse counter; entity-level impact is enforced by DeltaStore later.
        let current = self.hypothetical_count.fetch_add(1, Ordering::AcqRel) + 1;

        // Cap the total number of hypotheticals as a conservative proxy.
        // Derived from max_affected_entities * remaining_depth.
        let max_ops = self
            .constraints
            .max_affected_entities
            .saturating_mul(self.remaining_depth)
            .max(1);

        if current > max_ops {
            return Err(KyroError::Execution(ExecutionError::SimulationLimitExceeded {
                limit_type: "hypothetical_count".to_string(),
                max_value: max_ops as u64,
                actual_value: current as u64,
            }));
        }

        Ok(())
    }

    /// Assert a hypothetical belief into the simulation overlay.
    ///
    /// This will never mutate the base stores.
    pub fn assert_hypothetical(&self, belief: Belief) -> KyroResult<BeliefId> {
        self.register_hypothetical()?;

        // Enforce that the subject exists in base storage.
        let exists = self
            .delta_store
            .entities()
            .get(belief.subject)
            .map_err(storage_err)?
            .is_some();
        if !exists {
            return Err(KyroError::Execution(ExecutionError::EntityNotFound {
                id: belief.subject,
            }));
        }

        let id = belief.id;
        self.delta_store
            .beliefs()
            .insert(belief)
            .map_err(storage_err)?;
        Ok(id)
    }

    /// Return an impact summary for the current overlay state.
    pub fn query_impact(&self) -> KyroResult<SimulationImpact> {
        self.ensure_not_expired()?;
        let (affected_entities, inserted_belief_ids, supersedes) = self
            .delta_store
            .impact_details()
            .map_err(storage_err)?;
        Ok(SimulationImpact {
            affected_entities,
            inserted_beliefs: inserted_belief_ids.len(),
            inserted_belief_ids,
            supersedes,
        })
    }

    /// Execute a RESOLVE operation against the base+delta overlay.
    ///
    /// This routes reads through the simulation's `DeltaStore`, ensuring
    /// hypotheticals are visible while the base stores remain unmodified.
    pub fn resolve_payload(&self, payload: ResolvePayload) -> KyroResult<BeliefFrame> {
        self.resolve_ir(KyroIR::new(Operation::Resolve(payload)))
    }

    /// Execute a RESOLVE IR against the base+delta overlay.
    pub fn resolve_ir(&self, ir: KyroIR) -> KyroResult<BeliefFrame> {
        self.ensure_not_expired()?;

        let KyroIR {
            operation,
            timestamp,
            request_id,
            version,
        } = ir;

        let Operation::Resolve(payload) = operation else {
            return Err(KyroError::Execution(ExecutionError::InvalidOperation {
                expected: "resolve".to_string(),
                actual: format!("{operation:?}"),
            }));
        };

        let engine = KyroEngine::new(
            self.delta_store.entities(),
            self.delta_store.beliefs(),
            self.delta_store.patterns(),
            self.delta_store.conflicts(),
            std::sync::Arc::clone(&self.derivations),
        );

        // Preserve request metadata for tracing.
        let ir = KyroIR {
            version,
            request_id,
            timestamp,
            operation: Operation::Resolve(payload),
        };

        match engine.execute(ir)? {
            EngineResponse::Resolve { frame } => Ok(frame),
            other => Err(KyroError::Execution(ExecutionError::InvalidOperation {
                expected: "engine_response.resolve".to_string(),
                actual: format!("{other:?}"),
            })),
        }
    }

    /// Execute a DERIVE operation inside the simulation.
    ///
    /// This records a derivation chain (premises + rule + optional steps) in the simulation-local
    /// derivation store. It does not mutate base storage.
    pub fn derive_payload(&self, payload: DerivePayload) -> KyroResult<DerivationId> {
        self.derive_ir(KyroIR::new(Operation::Derive(payload)))
    }

    /// Execute a DERIVE IR inside the simulation.
    pub fn derive_ir(&self, ir: KyroIR) -> KyroResult<DerivationId> {
        self.ensure_not_expired()?;

        let KyroIR {
            operation,
            timestamp,
            request_id,
            version,
        } = ir;

        let Operation::Derive(payload) = operation else {
            return Err(KyroError::Execution(ExecutionError::InvalidOperation {
                expected: "derive".to_string(),
                actual: format!("{operation:?}"),
            }));
        };

        let engine = KyroEngine::new(
            self.delta_store.entities(),
            self.delta_store.beliefs(),
            self.delta_store.patterns(),
            self.delta_store.conflicts(),
            std::sync::Arc::clone(&self.derivations),
        );

        let ir = KyroIR {
            version,
            request_id,
            timestamp,
            operation: Operation::Derive(payload),
        };

        match engine.execute(ir)? {
            EngineResponse::Derive { derivation_id } => Ok(derivation_id),
            other => Err(KyroError::Execution(ExecutionError::InvalidOperation {
                expected: "engine_response.derive".to_string(),
                actual: format!("{other:?}"),
            })),
        }
    }

    /// Read a derivation record recorded inside this simulation.
    pub fn get_derivation(&self, id: DerivationId) -> KyroResult<Option<crate::derivation::DerivationRecord>> {
        self.ensure_not_expired()?;
        self.derivations.get(id).map_err(storage_err)
    }

    /// Find derivations that cite a given premise belief ID.
    pub fn find_derivations_by_premise(
        &self,
        premise_id: BeliefId,
    ) -> KyroResult<Vec<crate::derivation::DerivationRecord>> {
        self.ensure_not_expired()?;
        self.derivations.find_by_premise(premise_id).map_err(storage_err)
    }

    /// Find derivations that are attached to a given derived belief ID.
    pub fn find_derivations_by_derived_belief(
        &self,
        derived_belief_id: BeliefId,
    ) -> KyroResult<Vec<crate::derivation::DerivationRecord>> {
        self.ensure_not_expired()?;
        self.derivations
            .find_by_derived_belief(derived_belief_id)
            .map_err(storage_err)
    }
}

impl Drop for SimulationContext {
    fn drop(&mut self) {
        self.is_dropped.store(true, Ordering::Release);
        self.delta_store.clear();
        self.delta_index.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::sync::Arc;

    use crate::belief::ConsistencyStatus;
    use crate::confidence::Confidence;
    use crate::entity::{Entity, EntityType};
    use crate::source::Source;
    use crate::storage::{BeliefStore, ConflictStore, EntityStore, PatternStore};
    use crate::time::TimeRange;
    use crate::value::Value;

    #[test]
    fn commit_overlay_inserts_into_base_and_returns_mapping() {
        let stores = crate::storage::InMemoryStores::default();

        let entity = Entity::new("e", EntityType::Concept);
        let entity_id = entity.id;
        stores.entities.insert(entity).unwrap();

        let entities: Arc<dyn EntityStore> = Arc::new(stores.entities);
        let beliefs: Arc<dyn BeliefStore> = Arc::new(stores.beliefs);
        let patterns: Arc<dyn PatternStore> = Arc::new(stores.patterns);
        let conflicts: Arc<dyn ConflictStore> = Arc::new(stores.conflicts);
        let derivations: Arc<dyn crate::storage::DerivationStore> =
            Arc::new(stores.derivations);

        let engine = KyroEngine::new(
            Arc::clone(&entities),
            Arc::clone(&beliefs),
            Arc::clone(&patterns),
            Arc::clone(&conflicts),
            Arc::clone(&derivations),
        );

        let base = SimulationBaseStores {
            entities: Arc::clone(&entities),
            beliefs: Arc::clone(&beliefs),
            patterns: Arc::clone(&patterns),
            conflicts: Arc::clone(&conflicts),
        };

        let ctx = SimulationContext::new(
            base,
            SimulateConstraints {
                max_affected_entities: 10,
                max_depth: 1,
                max_duration_ms: 500,
            },
        )
        .unwrap();

        let t0 = Utc::now();
        let hypo = Belief {
            id: BeliefId::new(),
            subject: entity_id,
            predicate: "p".to_string(),
            value: Value::Int(7),
            confidence: Confidence::from_agent(0.9, "sim").unwrap(),
            source: Source::Unknown { description: None },
            valid_time: TimeRange::starting_at(t0),
            tx_time: t0,
            reason: None,
            consistency_status: ConsistencyStatus::Provisional,
            supersedes: None,
            superseded_by: None,
            embedding: None,
        };

        ctx.assert_hypothetical(hypo.clone()).unwrap();

        let before = beliefs
            .find_by_entity_predicate(entity_id, "p")
            .unwrap()
            .len();
        assert_eq!(before, 0);

        let res = ctx.commit_overlay(&engine, ConsistencyMode::Eventual).unwrap();
        assert_eq!(res.committed_beliefs, 1);
        assert_eq!(res.belief_id_map.len(), 1);
        assert_eq!(res.belief_id_map[0].0, hypo.id);

        let after = beliefs
            .find_by_entity_predicate(entity_id, "p")
            .unwrap()
            .len();
        assert_eq!(after, 1);
    }

    #[test]
    fn commit_overlay_applies_supersede_pairs_using_committed_ids() {
        let stores = crate::storage::InMemoryStores::default();

        let entity = Entity::new("e", EntityType::Concept);
        let entity_id = entity.id;
        stores.entities.insert(entity).unwrap();

        let entities: Arc<dyn EntityStore> = Arc::new(stores.entities);
        let beliefs: Arc<dyn BeliefStore> = Arc::new(stores.beliefs);
        let patterns: Arc<dyn PatternStore> = Arc::new(stores.patterns);
        let conflicts: Arc<dyn ConflictStore> = Arc::new(stores.conflicts);
        let derivations: Arc<dyn crate::storage::DerivationStore> =
            Arc::new(stores.derivations);

        let engine = KyroEngine::new(
            Arc::clone(&entities),
            Arc::clone(&beliefs),
            Arc::clone(&patterns),
            Arc::clone(&conflicts),
            Arc::clone(&derivations),
        );

        // Seed a base belief to supersede.
        let seed = KyroIR::new(Operation::Assert(crate::ir::AssertPayload {
            entity_id,
            predicate: "p".to_string(),
            value: Value::Int(1),
            confidence: Confidence::from_agent(0.8, "seed").unwrap(),
            source: Source::Unknown { description: None },
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Eventual,
            embedding: None,
        }));
        let EngineResponse::Assert { belief_id: old_id, .. } = engine.execute(seed).unwrap() else {
            panic!("expected assert");
        };

        let base = SimulationBaseStores {
            entities: Arc::clone(&entities),
            beliefs: Arc::clone(&beliefs),
            patterns: Arc::clone(&patterns),
            conflicts: Arc::clone(&conflicts),
        };

        let ctx = SimulationContext::new(
            base,
            SimulateConstraints {
                max_affected_entities: 10,
                max_depth: 1,
                max_duration_ms: 500,
            },
        )
        .unwrap();

        let t0 = Utc::now();
        let hypo = Belief {
            id: BeliefId::new(),
            subject: entity_id,
            predicate: "p".to_string(),
            value: Value::Int(2),
            confidence: Confidence::from_agent(0.9, "sim").unwrap(),
            source: Source::Unknown { description: None },
            valid_time: TimeRange::starting_at(t0),
            tx_time: t0,
            reason: None,
            consistency_status: ConsistencyStatus::Provisional,
            supersedes: None,
            superseded_by: None,
            embedding: None,
        };

        ctx.assert_hypothetical(hypo.clone()).unwrap();

        // Record a supersede edge in the overlay: old(base) -> new(overlay).
        ctx.delta_store
            .beliefs()
            .supersede(old_id, hypo.id)
            .unwrap();

        let res = ctx.commit_overlay(&engine, ConsistencyMode::Eventual).unwrap();
        let committed_new = res
            .belief_id_map
            .iter()
            .find(|(h, _)| *h == hypo.id)
            .map(|(_, c)| *c)
            .expect("mapping must exist");

        let updated_old = beliefs.get(old_id).unwrap().unwrap();
        assert_eq!(updated_old.superseded_by, Some(committed_new));
    }

    #[test]
    fn context_enforces_timeout() {
        let stores = crate::storage::InMemoryStores::default();
        let base = SimulationBaseStores {
            entities: Arc::new(stores.entities),
            beliefs: Arc::new(stores.beliefs),
            patterns: Arc::new(stores.patterns),
            conflicts: Arc::new(stores.conflicts),
        };

        let ctx = SimulationContext::new(
            base,
            SimulateConstraints {
            max_affected_entities: 10,
            max_depth: 1,
            max_duration_ms: 1,
            },
        )
        .unwrap();

        // Busy-wait only in test with tiny duration.
        while ctx.ensure_not_expired().is_ok() {}

        let err = ctx.ensure_not_expired().unwrap_err();
        let KyroError::Execution(ExecutionError::Timeout { .. }) = err else {
            panic!("expected timeout, got {err:?}");
        };
    }

    #[test]
    fn context_register_hypothetical_enforces_count_limit() {
        let stores = crate::storage::InMemoryStores::default();
        let base = SimulationBaseStores {
            entities: Arc::new(stores.entities),
            beliefs: Arc::new(stores.beliefs),
            patterns: Arc::new(stores.patterns),
            conflicts: Arc::new(stores.conflicts),
        };

        let ctx = SimulationContext::new(
            base,
            SimulateConstraints {
            max_affected_entities: 2,
            max_depth: 2,
            max_duration_ms: 500,
            },
        )
        .unwrap();

        // max_ops = 4
        for _ in 0..4 {
            ctx.register_hypothetical().unwrap();
        }

        let err = ctx.register_hypothetical().unwrap_err();
        let KyroError::Execution(ExecutionError::SimulationLimitExceeded { .. }) = err else {
            panic!("expected SimulationLimitExceeded, got {err:?}");
        };
    }

    #[test]
    fn child_sees_parent_hypotheticals_but_writes_do_not_leak_up() {
        let stores = crate::storage::InMemoryStores::default();
        let entity = Entity::new("e", EntityType::Concept);
        let entity_id = entity.id;
        stores.entities.insert(entity).unwrap();

        let base = SimulationBaseStores {
            entities: Arc::new(stores.entities),
            beliefs: Arc::new(stores.beliefs),
            patterns: Arc::new(stores.patterns),
            conflicts: Arc::new(stores.conflicts),
        };

        let parent = SimulationContext::new(
            base,
            SimulateConstraints {
                max_affected_entities: 10,
                max_depth: 3,
                max_duration_ms: 500,
            },
        )
        .unwrap();

        let t0 = Utc::now();
        let b_parent = Belief {
            id: BeliefId::new(),
            subject: entity_id,
            predicate: "p".to_string(),
            value: Value::Int(1),
            confidence: Confidence::from_agent(0.9, "sim").unwrap(),
            source: Source::Unknown { description: None },
            valid_time: TimeRange::starting_at(t0),
            tx_time: t0,
            reason: None,
            consistency_status: ConsistencyStatus::Provisional,
            supersedes: None,
            superseded_by: None,
            embedding: None,
        };

        parent.assert_hypothetical(b_parent.clone()).unwrap();
        assert!(parent.delta_store.beliefs().get(b_parent.id).unwrap().is_some());

        let child = parent.spawn_child().unwrap();

        // Child can read parent's hypotheticals through its base view.
        assert!(child.delta_store.beliefs().get(b_parent.id).unwrap().is_some());

        // Child writes remain isolated to the child.
        let t1 = t0 + chrono::Duration::milliseconds(1);
        let b_child = Belief {
            id: BeliefId::new(),
            subject: entity_id,
            predicate: "q".to_string(),
            value: Value::Bool(true),
            confidence: Confidence::from_agent(0.8, "sim").unwrap(),
            source: Source::Unknown { description: None },
            valid_time: TimeRange::starting_at(t1),
            tx_time: t1,
            reason: None,
            consistency_status: ConsistencyStatus::Provisional,
            supersedes: None,
            superseded_by: None,
            embedding: None,
        };

        child.assert_hypothetical(b_child.clone()).unwrap();
        assert!(child.delta_store.beliefs().get(b_child.id).unwrap().is_some());
        assert!(parent.delta_store.beliefs().get(b_child.id).unwrap().is_none());
    }

    #[test]
    fn child_op_budget_shrinks_with_depth() {
        let stores = crate::storage::InMemoryStores::default();
        let entity = Entity::new("e", EntityType::Concept);
        stores.entities.insert(entity).unwrap();

        let base = SimulationBaseStores {
            entities: Arc::new(stores.entities),
            beliefs: Arc::new(stores.beliefs),
            patterns: Arc::new(stores.patterns),
            conflicts: Arc::new(stores.conflicts),
        };

        // max_ops(parent)=1*2=2, max_ops(child)=1*1=1
        let parent = SimulationContext::new(
            base,
            SimulateConstraints {
                max_affected_entities: 1,
                max_depth: 2,
                max_duration_ms: 500,
            },
        )
        .unwrap();

        let child = parent.spawn_child().unwrap();
        child.register_hypothetical().unwrap();

        let err = child.register_hypothetical().unwrap_err();
        let KyroError::Execution(ExecutionError::SimulationLimitExceeded { limit_type, .. }) = err else {
            panic!("expected SimulationLimitExceeded, got {err:?}");
        };
        assert_eq!(limit_type, "hypothetical_count");
    }

    #[test]
    fn child_deadline_is_capped_by_parent_deadline() {
        let stores = crate::storage::InMemoryStores::default();
        let entity = Entity::new("e", EntityType::Concept);
        stores.entities.insert(entity).unwrap();

        let base = SimulationBaseStores {
            entities: Arc::new(stores.entities),
            beliefs: Arc::new(stores.beliefs),
            patterns: Arc::new(stores.patterns),
            conflicts: Arc::new(stores.conflicts),
        };

        let parent = SimulationContext::new(
            base,
            SimulateConstraints {
                max_affected_entities: 10,
                max_depth: 3,
                max_duration_ms: 20,
            },
        )
        .unwrap();

        let child = parent.spawn_child().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(40));

        let parent_err = parent.ensure_not_expired().unwrap_err();
        assert!(matches!(parent_err, KyroError::Execution(ExecutionError::Timeout { .. })));

        let child_err = child.ensure_not_expired().unwrap_err();
        assert!(matches!(child_err, KyroError::Execution(ExecutionError::Timeout { .. })));
    }
}
