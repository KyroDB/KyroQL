//! Inference layer primitives.
//!
//! Phase 2 starts by making conflict-resolution *explicit* and reproducible.
//! For now we only implement selection policies (no belief merging).

mod policies;
mod resolver;

pub use policies::ConflictResolutionPolicy;
pub use resolver::{apply_conflict_policy, PolicyDecision};
