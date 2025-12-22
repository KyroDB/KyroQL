//! Storage trait definitions for KyroQL.
//!
//! These traits define the abstract interface for storage backends.
//! Implementations will be provided in separate modules.

mod traits;

pub use traits::{BeliefStore, ConflictStore, EntityStore, PatternStore, StorageError};
