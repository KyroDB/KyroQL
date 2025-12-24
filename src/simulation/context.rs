//! Simulation context with hard timeout and deterministic teardown.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::belief::Belief;
use crate::confidence::BeliefId;
use crate::engine::{EngineResponse, KyroEngine};
use crate::error::{ExecutionError, KyroError, KyroResult};
use crate::frame::BeliefFrame;
use crate::ir::{KyroIR, Operation, ResolvePayload};
use crate::storage::StorageError;

use crate::entity::EntityId;

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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    created_at: Instant,
    deadline: Instant,
    hypothetical_count: AtomicUsize,
    is_dropped: AtomicBool,

    // Overlay state (implemented in later steps).
    pub(crate) delta_store: DeltaStore,
    pub(crate) delta_index: DeltaVectorIndex,
}

impl fmt::Debug for SimulationContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SimulationContext")
            .field("id", &self.id)
            .field("constraints", &self.constraints)
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
        constraints.validate().map_err(KyroError::from)?;

        let created_at = Instant::now();
        let timeout = Duration::from_millis(constraints.max_duration_ms);
        let deadline = created_at
            .checked_add(timeout)
            .ok_or_else(|| KyroError::Validation(crate::error::ValidationError::InvalidSimulationConstraints {
                reason: "max_duration_ms overflow".to_string(),
            }))?;

        Ok(Self {
            id: SimulationId::new(),
            constraints,
            created_at,
            deadline,
            hypothetical_count: AtomicUsize::new(0),
            is_dropped: AtomicBool::new(false),
            delta_store: DeltaStore::new(base, constraints),
            delta_index: DeltaVectorIndex::new(),
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
        // Derived from max_affected_entities * max_depth.
        let max_ops = self
            .constraints
            .max_affected_entities
            .saturating_mul(self.constraints.max_depth)
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
    use std::sync::Arc;

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
}
