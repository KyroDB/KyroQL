//! Entity types and identity management.
//!
//! The Entity layer is the prerequisite for everything in KyroQL.
//! Without stable entity IDs, beliefs cannot be linked, contradictions
//! cannot be detected, and temporal queries are meaningless.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Globally unique, stable entity identifier.
///
/// Once created, an `EntityId` never changes. This provides the stable
/// identity anchor that beliefs reference.
///
/// # Examples
///
/// ```
/// use kyroql::EntityId;
///
/// let id = EntityId::new();
/// assert!(!id.is_nil());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(Uuid);

impl EntityId {
    /// Creates a new random entity ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Creates an entity ID from an existing UUID.
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Returns the underlying UUID.
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    /// Returns true if this is a nil (all zeros) UUID.
    #[must_use]
    pub fn is_nil(&self) -> bool {
        self.0.is_nil()
    }

    /// Creates a nil entity ID (for testing or sentinel values).
    #[must_use]
    pub const fn nil() -> Self {
        Self(Uuid::nil())
    }
}

impl Default for EntityId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for EntityId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl From<EntityId> for Uuid {
    fn from(id: EntityId) -> Self {
        id.0
    }
}

/// Classification of entity types.
///
/// Entity types help organize beliefs and can be used for
/// pattern matching and constraint enforcement.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    /// A human person
    Person,
    /// A company, institution, or group
    Organization,
    /// An abstract concept or idea
    Concept,
    /// A temporal event
    Event,
    /// A geographic location
    Location,
    /// A physical or digital artifact (code, documents, objects)
    Artifact,
    /// A scientific hypothesis or theory
    Hypothesis,
    /// A custom entity type
    Custom(String),
}

impl fmt::Display for EntityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Person => write!(f, "person"),
            Self::Organization => write!(f, "organization"),
            Self::Concept => write!(f, "concept"),
            Self::Event => write!(f, "event"),
            Self::Location => write!(f, "location"),
            Self::Artifact => write!(f, "artifact"),
            Self::Hypothesis => write!(f, "hypothesis"),
            Self::Custom(name) => write!(f, "custom:{name}"),
        }
    }
}

/// The anchor of identity in KyroQL.
///
/// All beliefs attach to entities via [`EntityId`]. An entity represents
/// a stable identity that can have multiple beliefs associated with it.
///
/// # Examples
///
/// ```
/// use kyroql::{Entity, EntityType};
///
/// let entity = Entity::new("LK-99", EntityType::Concept);
/// assert_eq!(entity.canonical_name, "LK-99");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// Globally unique identifier
    pub id: EntityId,

    pub canonical_name: String,

    #[serde(default)]
    pub aliases: Vec<String>,

    pub entity_type: EntityType,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,

    pub version: u64,

    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl Entity {
    /// Creates a new entity with the given name and type.
    ///
    /// # Examples
    ///
    /// ```
    /// use kyroql::{Entity, EntityType};
    ///
    /// let entity = Entity::new("Albert Einstein", EntityType::Person);
    /// assert_eq!(entity.version, 1);
    /// ```
    #[must_use]
    pub fn new(name: impl Into<String>, entity_type: EntityType) -> Self {
        let now = Utc::now();
        Self {
            id: EntityId::new(),
            canonical_name: name.into(),
            aliases: Vec::new(),
            entity_type,
            created_at: now,
            updated_at: now,
            embedding: None,
            version: 1,
            metadata: serde_json::Value::Null,
        }
    }

    /// Creates a new entity with a specific ID.
    ///
    /// This is useful when you need to control the entity ID,
    /// such as during data migration or testing.
    #[must_use]
    pub fn with_id(id: EntityId, name: impl Into<String>, entity_type: EntityType) -> Self {
        let now = Utc::now();
        Self {
            id,
            canonical_name: name.into(),
            aliases: Vec::new(),
            entity_type,
            created_at: now,
            updated_at: now,
            embedding: None,
            version: 1,
            metadata: serde_json::Value::Null,
        }
    }

    /// Adds an alias to this entity.
    pub fn add_alias(&mut self, alias: impl Into<String>) {
        let alias = alias.into();
        if !self.aliases.contains(&alias) {
            self.aliases.push(alias);
            self.touch();
        }
    }

    /// Sets the embedding vector for semantic matching.
    pub fn set_embedding(&mut self, embedding: Vec<f32>) {
        self.embedding = Some(embedding);
        self.touch();
    }

    /// Updates the `updated_at` timestamp and increments the version.
    fn touch(&mut self) {
        self.updated_at = Utc::now();
        self.version += 1;
    }

    /// Returns true if this entity has an embedding.
    #[must_use]
    pub fn has_embedding(&self) -> bool {
        self.embedding.is_some()
    }

    /// Returns the number of aliases.
    #[must_use]
    pub fn alias_count(&self) -> usize {
        self.aliases.len()
    }
}

impl PartialEq for Entity {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Entity {}

impl std::hash::Hash for Entity {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entity_id_creation() {
        let id1 = EntityId::new();
        let id2 = EntityId::new();
        assert_ne!(id1, id2);
        assert!(!id1.is_nil());
    }

    #[test]
    fn test_entity_id_nil() {
        let nil = EntityId::nil();
        assert!(nil.is_nil());
    }

    #[test]
    fn test_entity_id_from_uuid() {
        let uuid = Uuid::new_v4();
        let id = EntityId::from_uuid(uuid);
        assert_eq!(id.as_uuid(), &uuid);
    }

    #[test]
    fn test_entity_id_display() {
        let id = EntityId::new();
        let display = format!("{id}");
        assert!(!display.is_empty());
        assert!(display.contains('-')); // UUID format
    }

    #[test]
    fn test_entity_creation() {
        let entity = Entity::new("Test Entity", EntityType::Concept);
        assert_eq!(entity.canonical_name, "Test Entity");
        assert_eq!(entity.entity_type, EntityType::Concept);
        assert_eq!(entity.version, 1);
        assert!(entity.aliases.is_empty());
    }

    #[test]
    fn test_entity_with_id() {
        let id = EntityId::new();
        let entity = Entity::with_id(id, "Test", EntityType::Person);
        assert_eq!(entity.id, id);
    }

    #[test]
    fn test_entity_add_alias() {
        let mut entity = Entity::new("Einstein", EntityType::Person);
        entity.add_alias("Albert Einstein");
        entity.add_alias("A. Einstein");

        assert_eq!(entity.alias_count(), 2);
        assert!(entity.aliases.contains(&"Albert Einstein".to_string()));
        assert_eq!(entity.version, 3); // Initial + 2 alias additions
    }

    #[test]
    fn test_entity_add_duplicate_alias() {
        let mut entity = Entity::new("Einstein", EntityType::Person);
        entity.add_alias("Albert");
        let version_after_first = entity.version;
        entity.add_alias("Albert"); // Duplicate

        assert_eq!(entity.alias_count(), 1);
        assert_eq!(entity.version, version_after_first); // No change
    }

    #[test]
    fn test_entity_set_embedding() {
        let mut entity = Entity::new("Test", EntityType::Concept);
        assert!(!entity.has_embedding());

        entity.set_embedding(vec![0.1, 0.2, 0.3]);
        assert!(entity.has_embedding());
        assert_eq!(entity.embedding.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn test_entity_equality() {
        let id = EntityId::new();
        let entity1 = Entity::with_id(id, "Test", EntityType::Concept);
        let mut entity2 = Entity::with_id(id, "Different Name", EntityType::Person);
        entity2.version = 100;

        // Entities are equal if they have the same ID
        assert_eq!(entity1, entity2);
    }

    #[test]
    fn test_entity_type_display() {
        assert_eq!(format!("{}", EntityType::Person), "person");
        assert_eq!(format!("{}", EntityType::Concept), "concept");
        assert_eq!(
            format!("{}", EntityType::Custom("my_type".to_string())),
            "custom:my_type"
        );
    }

    #[test]
    fn test_entity_serialization() {
        let entity = Entity::new("Test", EntityType::Concept);
        let json = serde_json::to_string(&entity).unwrap();
        let deserialized: Entity = serde_json::from_str(&json).unwrap();
        assert_eq!(entity.id, deserialized.id);
        assert_eq!(entity.canonical_name, deserialized.canonical_name);
    }
}
