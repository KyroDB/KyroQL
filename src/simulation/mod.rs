//! Simulation support for counterfactual reasoning.
//!
//! Vision requirement: simulations must be isolated from base storage and run
//! on the Reflection execution path.

pub mod constraints;
pub mod context;

// Implemented in later steps (Phase 3.1 / 3.2).
pub mod delta_index;
pub mod delta_store;

pub use constraints::SimulateConstraints;
pub use context::{SimulationContext, SimulationId, SimulationImpact};

use std::sync::Arc;

use crate::storage::{BeliefStore, ConflictStore, EntityStore, PatternStore};

/// Base storage handles used as the substrate for a simulation.
///
/// The simulation layer must treat these stores as immutable; writes are applied only
/// to the delta overlay.
#[derive(Clone)]
pub struct SimulationBaseStores {
	/// Base entity store.
	pub entities: Arc<dyn EntityStore>,
	/// Base belief store.
	pub beliefs: Arc<dyn BeliefStore>,
	/// Base pattern store.
	pub patterns: Arc<dyn PatternStore>,
	/// Base conflict store.
	pub conflicts: Arc<dyn ConflictStore>,
}
