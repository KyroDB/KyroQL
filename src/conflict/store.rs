//! Conflict storage re-exports.
//!
//! Storage contracts live under `crate::storage`; this module exists to match
//! the documented file layout.

pub use crate::storage::{ConflictStore, InMemoryConflictStore, StorageError};
