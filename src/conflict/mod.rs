//! Conflict modules.
//!
//! Conflicts are explicit objects tracking contradictions and their resolution.

pub mod detector;
pub mod store;
pub mod types;

pub use types::{
    Conflict, ConflictId, ConflictResolution, ConflictStatus, ConflictType,
};
