//! Storage trait definitions for KyroQL.
//!
//! These traits define the abstract interface for storage backends.
//! Implementations will be provided in separate modules.

mod traits;
pub mod memory;

#[cfg(feature = "persistent")]
pub mod persistent;

pub use traits::{
	BeliefStore, ConflictStore, DerivationStore, EntityStore, PatternStore, StorageError,
};

pub use memory::{
	InMemoryBeliefStore, InMemoryConflictStore, InMemoryDerivationStore, InMemoryEntityStore,
	InMemoryPatternStore, InMemoryStores,
};

#[cfg(feature = "persistent")]
pub use persistent::{
	open_database, PersistentBeliefStore, PersistentConfig, PersistentConflictStore,
	PersistentDerivationStore, PersistentEntityStore, PersistentPatternStore, PersistentStores,
};
