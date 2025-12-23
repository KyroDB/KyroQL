//! Entity storage re-exports.
//!
//! KyroQL's storage contracts live under `crate::storage`; this module exists to
//! match the documented file layout and provide a focused import surface.

pub use crate::storage::{EntityStore, InMemoryEntityStore, StorageError};
