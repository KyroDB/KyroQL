//! Abstract storage traits for KyroQL.
//!
//! These traits define the contract that storage backends must implement.
//! By using traits, we enable:
//! - In-memory backends for testing and embedded use
//! - Persistent backends for production
//! - Distributed backends for scale

use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::belief::Belief;
use crate::confidence::BeliefId;
use crate::conflict::{Conflict, ConflictId};
use crate::derivation::{DerivationId, DerivationRecord};
use crate::entity::{Entity, EntityId};
use crate::pattern::{Pattern, PatternId};
use crate::time::TimeRange;

/// Errors that can occur during storage operations.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Entity not found.
    #[error("Entity not found: {0}")]
    EntityNotFound(EntityId),

    /// Belief not found.
    #[error("Belief not found: {0}")]
    BeliefNotFound(BeliefId),

    /// Conflict not found.
    #[error("Conflict not found: {0}")]
    ConflictNotFound(ConflictId),

    /// Pattern not found.
    #[error("Pattern not found: {0}")]
    PatternNotFound(PatternId),

    /// Key already exists.
    #[error("Duplicate key: {0}")]
    DuplicateKey(String),

    /// Backend error.
    #[error("Storage backend error: {0}")]
    BackendError(String),

    /// Serialization failed.
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Connection failed.
    #[error("Connection error: {0}")]
    ConnectionError(String),
}

/// Storage trait for Entity operations.
///
/// # Safety Considerations
/// - All mutations should be atomic where possible
/// - Implementations should handle concurrent access safely
pub trait EntityStore: Send + Sync {
    /// Insert a new entity. Returns error if ID already exists.
    fn insert(&self, entity: Entity) -> Result<(), StorageError>;

    /// Get an entity by ID.
    fn get(&self, id: EntityId) -> Result<Option<Entity>, StorageError>;

    /// Update an existing entity. Returns error if not found.
    fn update(&self, entity: Entity) -> Result<(), StorageError>;

    /// Delete an entity by ID. Returns error if not found.
    fn delete(&self, id: EntityId) -> Result<(), StorageError>;

    /// Find entities by canonical name (exact match).
    fn find_by_name(&self, name: &str) -> Result<Vec<Entity>, StorageError>;

    /// Find entities by name (fuzzy/prefix match).
    fn find_by_name_fuzzy(&self, query: &str, limit: usize) -> Result<Vec<Entity>, StorageError>;

    /// Find entities by embedding similarity (requires vector index).
    fn find_by_embedding(
        &self,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(Entity, f32)>, StorageError>;

    /// Merge `secondary` into `primary`, returning the merged entity.
    ///
    /// After a successful merge:
    /// - Calling `get(secondary_id)` returns the merged primary entity
    /// - Historical versions of the secondary remain accessible via `get_at_version(secondary_id, version)`
    ///   and `list_versions(secondary_id)`
    ///
    /// # Errors
    /// - `EntityNotFound`: If either `primary` or `secondary` does not exist
    /// - `BackendError`: If `primary == secondary` or other merge conflicts occur
    fn merge(&self, primary: EntityId, secondary: EntityId) -> Result<Entity, StorageError>;

    /// Retrieve the entity snapshot for an exact version.
    ///
    /// Versions start at 1 on insert and increment on every update/merge. Implementations should
    /// return `Ok(None)` when the entity is missing or the requested version is not recorded. Use
    /// `list_versions` to discover available versions.
    fn get_at_version(&self, id: EntityId, version: u64) -> Result<Option<Entity>, StorageError>;

    /// List all stored versions for an entity (ascending by version).
    fn list_versions(&self, id: EntityId) -> Result<Vec<Entity>, StorageError>;
}

/// Storage trait for Belief operations.
///
/// # Bitemporal Semantics
/// - `valid_time`: When the belief is/was true in the world
/// - `tx_time`: When the belief was recorded in the system
pub trait BeliefStore: Send + Sync {
    /// Insert a new belief. Returns error if ID already exists.
    fn insert(&self, belief: Belief) -> Result<(), StorageError>;

    /// Get a belief by ID.
    fn get(&self, id: BeliefId) -> Result<Option<Belief>, StorageError>;

    /// Mark a belief as superseded by another.
    fn supersede(&self, old_id: BeliefId, new_id: BeliefId) -> Result<(), StorageError>;

    /// Find all beliefs for an entity (any predicate).
    fn find_by_entity(&self, entity_id: EntityId) -> Result<Vec<Belief>, StorageError>;

    /// Find beliefs by entity and predicate.
    fn find_by_entity_predicate(
        &self,
        entity_id: EntityId,
        predicate: &str,
    ) -> Result<Vec<Belief>, StorageError>;

    /// Find beliefs valid at a specific time (AS OF query).
    fn find_as_of(
        &self,
        entity_id: EntityId,
        predicate: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Vec<Belief>, StorageError>;

    /// Find beliefs within a time range.
    fn find_by_time_range(&self, range: &TimeRange) -> Result<Vec<Belief>, StorageError>;

    /// Find beliefs by embedding similarity (semantic search).
    fn find_by_embedding(
        &self,
        embedding: &[f32],
        limit: usize,
        min_confidence: Option<f32>,
    ) -> Result<Vec<(Belief, f32)>, StorageError>;

    /// Count beliefs for an entity.
    fn count_by_entity(&self, entity_id: EntityId) -> Result<usize, StorageError>;
}

/// Storage trait for Conflict operations.
pub trait ConflictStore: Send + Sync {
    /// Insert a new conflict.
    fn insert(&self, conflict: Conflict) -> Result<(), StorageError>;

    /// Get a conflict by ID.
    fn get(&self, id: ConflictId) -> Result<Option<Conflict>, StorageError>;

    /// Update conflict status/resolution.
    fn update(&self, conflict: Conflict) -> Result<(), StorageError>;

    /// Find conflicts involving a specific belief.
    fn find_by_belief(&self, belief_id: BeliefId) -> Result<Vec<Conflict>, StorageError>;

    /// Find all open (unresolved) conflicts.
    fn find_open(&self) -> Result<Vec<Conflict>, StorageError>;
}

/// Storage trait for Pattern operations.
pub trait PatternStore: Send + Sync {
    /// Insert a new pattern.
    fn insert(&self, pattern: Pattern) -> Result<(), StorageError>;

    /// Get a pattern by ID.
    fn get(&self, id: PatternId) -> Result<Option<Pattern>, StorageError>;

    /// Update an existing pattern.
    fn update(&self, pattern: Pattern) -> Result<(), StorageError>;

    /// Delete a pattern.
    fn delete(&self, id: PatternId) -> Result<(), StorageError>;

    /// Find patterns that apply to a specific predicate.
    fn find_by_predicate(&self, predicate: &str) -> Result<Vec<Pattern>, StorageError>;

    /// Find all active patterns.
    fn find_active(&self) -> Result<Vec<Pattern>, StorageError>;
}

/// Storage trait for derivation records.
///
/// Derivations provide an audit trail linking premise beliefs to a derived belief.
pub trait DerivationStore: Send + Sync {
    /// Insert a new derivation record. Returns error if ID already exists.
    fn insert(&self, record: DerivationRecord) -> Result<(), StorageError>;

    /// Get a derivation record by ID.
    fn get(&self, id: DerivationId) -> Result<Option<DerivationRecord>, StorageError>;

    /// Find derivations that cite a given premise belief.
    fn find_by_premise(&self, premise_id: BeliefId) -> Result<Vec<DerivationRecord>, StorageError>;

    /// Find derivations that produced a given derived belief.
    fn find_by_derived_belief(
        &self,
        derived_belief_id: BeliefId,
    ) -> Result<Vec<DerivationRecord>, StorageError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-time test: ensure traits are object-safe
    fn _assert_entity_store_object_safe(_: &dyn EntityStore) {}
    fn _assert_belief_store_object_safe(_: &dyn BeliefStore) {}
    fn _assert_conflict_store_object_safe(_: &dyn ConflictStore) {}
    fn _assert_pattern_store_object_safe(_: &dyn PatternStore) {}
    fn _assert_derivation_store_object_safe(_: &dyn DerivationStore) {}

    #[test]
    fn test_storage_error_display() {
        let err = StorageError::EntityNotFound(EntityId::new());
        assert!(err.to_string().contains("Entity not found"));

        let err = StorageError::BackendError("connection refused".to_string());
        assert!(err.to_string().contains("connection refused"));
    }
}
