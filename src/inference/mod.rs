//! Inference layer primitives.
//!
//! Conflict-resolution policies for reproducible inference.
//! For now we only implement selection policies (no belief merging).

mod policies;
mod resolver;

pub use policies::ConflictResolutionPolicy;
pub use resolver::{apply_conflict_policy, PolicyDecision};
