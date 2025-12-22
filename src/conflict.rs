//! Conflict types for tracking contradictions.
//!
//! Conflicts in KyroQL are explicit objects, not hidden errors.
//! When beliefs contradict, we create a Conflict record that
//! tracks the contradiction and its resolution.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::confidence::BeliefId;
use crate::entity::EntityId;

/// Unique identifier for a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConflictId(Uuid);

impl ConflictId {
    /// Creates a new random conflict ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ConflictId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ConflictId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The type of conflict between beliefs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictType {
    /// Same predicate has incompatible values.
    ValueContradiction {
        /// The predicate with conflicting values.
        predicate: String,
    },

    /// Beliefs have inconsistent temporal relationships.
    TemporalInconsistency {
        /// Description of the inconsistency.
        reason: String,
    },

    /// Multiple sources disagree.
    SourceDisagreement {
        /// Number of disagreeing sources.
        source_count: usize,
    },

    /// A defined pattern/constraint was violated.
    PatternViolation {
        /// ID of the violated pattern.
        pattern_id: String,
        /// Name of the violated pattern.
        pattern_name: String,
    },

    /// Logical contradiction (e.g., A and not-A).
    LogicalContradiction {
        /// Type of contradiction.
        contradiction_type: String,
    },

    /// Custom conflict type.
    Custom {
        /// Name of the custom type.
        name: String,
        /// Reason for the conflict.
        reason: String,
    },
}

impl fmt::Display for ConflictType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ValueContradiction { predicate } => {
                write!(f, "value_contradiction({predicate})")
            }
            Self::TemporalInconsistency { reason } => {
                write!(f, "temporal_inconsistency({reason})")
            }
            Self::SourceDisagreement { source_count } => {
                write!(f, "source_disagreement({source_count} sources)")
            }
            Self::PatternViolation { pattern_name, .. } => {
                write!(f, "pattern_violation({pattern_name})")
            }
            Self::LogicalContradiction {
                contradiction_type,
            } => {
                write!(f, "logical_contradiction({contradiction_type})")
            }
            Self::Custom { name, .. } => write!(f, "custom({name})"),
        }
    }
}

/// The status of a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStatus {
    /// Conflict is open and unresolved.
    Open,

    /// Conflict is being analyzed.
    Analyzing,

    /// Conflict has been resolved.
    Resolved,

    /// Conflict was dismissed (deemed not a real conflict).
    Dismissed,
}

impl Default for ConflictStatus {
    fn default() -> Self {
        Self::Open
    }
}

impl fmt::Display for ConflictStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open => write!(f, "open"),
            Self::Analyzing => write!(f, "analyzing"),
            Self::Resolved => write!(f, "resolved"),
            Self::Dismissed => write!(f, "dismissed"),
        }
    }
}

/// How a conflict was resolved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum ConflictResolution {
    /// Chose the belief with highest confidence.
    HigherConfidence {
        /// The winning belief.
        chosen_belief_id: BeliefId,
        /// Its confidence value.
        confidence: f32,
    },
    /// Chose the most recently asserted belief.
    MoreRecent {
        /// The winning belief.
        chosen_belief_id: BeliefId,
    },
    /// Chose based on source trust hierarchy.
    SourcePriority {
        /// The winning belief.
        chosen_belief_id: BeliefId,
        /// Priority rank of the source.
        source_priority: u32,
    },
    /// Merged beliefs into a consensus.
    Consensus {
        /// The newly created merged belief.
        merged_belief_id: BeliefId,
    },
    /// Resolved via human/agent review.
    ManualReview {
        /// The chosen belief (if any).
        chosen_belief_id: Option<BeliefId>,
        /// Who performed the review.
        reviewer_id: String,
        /// Review notes.
        notes: String,
    },
    /// All conflicting beliefs were retracted.
    AllRetracted,
    /// Conflict was accepted (coexistence allowed).
    Accepted {
        /// Reason for acceptance.
        reason: String,
    },
}

impl fmt::Display for ConflictResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HigherConfidence { confidence, .. } => {
                write!(f, "higher_confidence({confidence:.2})")
            }
            Self::MoreRecent { .. } => write!(f, "more_recent"),
            Self::SourcePriority { source_priority, .. } => {
                write!(f, "source_priority({source_priority})")
            }
            Self::Consensus { .. } => write!(f, "consensus"),
            Self::ManualReview { reviewer_id, .. } => {
                write!(f, "manual_review({reviewer_id})")
            }
            Self::AllRetracted => write!(f, "all_retracted"),
            Self::Accepted { reason } => write!(f, "accepted({reason})"),
        }
    }
}

/// A conflict between beliefs.
///
/// Conflicts are first-class objects in KyroQL. When beliefs contradict,
/// we don't silently drop data—we create a Conflict record that tracks
/// the contradiction and how (or if) it was resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    /// Unique identifier for this conflict.
    pub id: ConflictId,

    /// The conflicting belief IDs.
    pub belief_ids: Vec<BeliefId>,

    /// The entity involved in the conflict.
    pub entity_id: EntityId,

    /// The type of conflict.
    pub conflict_type: ConflictType,

    /// When the conflict was detected.
    pub detected_at: DateTime<Utc>,

    /// Current status.
    pub status: ConflictStatus,

    /// How it was resolved (if resolved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<ConflictResolution>,

    /// When it was resolved (if resolved).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,

    /// Severity score (0.0 to 1.0, higher is more severe).
    pub severity: f32,

    /// Arbitrary metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl Conflict {
    /// Creates a new conflict.
    #[must_use]
    pub fn new(
        belief_ids: Vec<BeliefId>,
        entity_id: EntityId,
        conflict_type: ConflictType,
    ) -> Self {
        Self {
            id: ConflictId::new(),
            belief_ids,
            entity_id,
            conflict_type,
            detected_at: Utc::now(),
            status: ConflictStatus::Open,
            resolution: None,
            resolved_at: None,
            severity: 0.5, // Default to medium severity
            metadata: serde_json::Value::Null,
        }
    }

    /// Creates a value contradiction conflict.
    #[must_use]
    pub fn value_contradiction(
        belief_ids: Vec<BeliefId>,
        entity_id: EntityId,
        predicate: impl Into<String>,
    ) -> Self {
        Self::new(
            belief_ids,
            entity_id,
            ConflictType::ValueContradiction {
                predicate: predicate.into(),
            },
        )
    }

    /// Creates a pattern violation conflict.
    #[must_use]
    pub fn pattern_violation(
        belief_ids: Vec<BeliefId>,
        entity_id: EntityId,
        pattern_id: impl Into<String>,
        pattern_name: impl Into<String>,
    ) -> Self {
        Self::new(
            belief_ids,
            entity_id,
            ConflictType::PatternViolation {
                pattern_id: pattern_id.into(),
                pattern_name: pattern_name.into(),
            },
        )
    }

    /// Returns true if the conflict is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.status == ConflictStatus::Open
    }

    /// Returns true if the conflict is resolved.
    #[must_use]
    pub fn is_resolved(&self) -> bool {
        self.status == ConflictStatus::Resolved
    }

    /// Sets the severity.
    pub fn with_severity(mut self, severity: f32) -> Self {
        self.severity = severity.clamp(0.0, 1.0);
        self
    }

    /// Resolves the conflict.
    pub fn resolve(&mut self, resolution: ConflictResolution) {
        self.status = ConflictStatus::Resolved;
        self.resolution = Some(resolution);
        self.resolved_at = Some(Utc::now());
    }

    /// Dismisses the conflict.
    pub fn dismiss(&mut self) {
        self.status = ConflictStatus::Dismissed;
        self.resolved_at = Some(Utc::now());
    }

    /// Returns the number of conflicting beliefs.
    #[must_use]
    pub fn belief_count(&self) -> usize {
        self.belief_ids.len()
    }
}

impl PartialEq for Conflict {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Conflict {}

impl std::hash::Hash for Conflict {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conflict_id() {
        let id1 = ConflictId::new();
        let id2 = ConflictId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_conflict_creation() {
        let beliefs = vec![BeliefId::new(), BeliefId::new()];
        let entity = EntityId::new();
        let conflict = Conflict::value_contradiction(
            beliefs.clone(),
            entity,
            "temperature",
        );

        assert_eq!(conflict.belief_count(), 2);
        assert!(conflict.is_open());
        assert!(!conflict.is_resolved());
    }

    #[test]
    fn test_conflict_pattern_violation() {
        let beliefs = vec![BeliefId::new()];
        let entity = EntityId::new();
        let conflict = Conflict::pattern_violation(
            beliefs,
            entity,
            "pattern-123",
            "temperature_range",
        );

        if let ConflictType::PatternViolation {
            pattern_id,
            pattern_name,
        } = &conflict.conflict_type
        {
            assert_eq!(pattern_id, "pattern-123");
            assert_eq!(pattern_name, "temperature_range");
        } else {
            panic!("Expected PatternViolation");
        }
    }

    #[test]
    fn test_conflict_resolve() {
        let beliefs = vec![BeliefId::new(), BeliefId::new()];
        let entity = EntityId::new();
        let mut conflict = Conflict::value_contradiction(beliefs.clone(), entity, "test");

        assert!(conflict.is_open());

        conflict.resolve(ConflictResolution::HigherConfidence {
            chosen_belief_id: beliefs[0],
            confidence: 0.95,
        });

        assert!(conflict.is_resolved());
        assert!(conflict.resolution.is_some());
        assert!(conflict.resolved_at.is_some());
    }

    #[test]
    fn test_conflict_dismiss() {
        let beliefs = vec![BeliefId::new()];
        let entity = EntityId::new();
        let mut conflict = Conflict::value_contradiction(beliefs, entity, "test");

        conflict.dismiss();

        assert_eq!(conflict.status, ConflictStatus::Dismissed);
        assert!(conflict.resolved_at.is_some());
    }

    #[test]
    fn test_conflict_severity() {
        let beliefs = vec![BeliefId::new()];
        let entity = EntityId::new();
        let conflict = Conflict::value_contradiction(beliefs, entity, "test")
            .with_severity(0.9);

        assert!((conflict.severity - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn test_conflict_severity_clamping() {
        let beliefs = vec![BeliefId::new()];
        let entity = EntityId::new();
        let conflict = Conflict::value_contradiction(beliefs, entity, "test")
            .with_severity(1.5);

        assert!((conflict.severity - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_conflict_type_display() {
        let ct = ConflictType::ValueContradiction {
            predicate: "temp".to_string(),
        };
        assert!(format!("{ct}").contains("value_contradiction"));
        assert!(format!("{ct}").contains("temp"));
    }

    #[test]
    fn test_conflict_status_display() {
        assert_eq!(format!("{}", ConflictStatus::Open), "open");
        assert_eq!(format!("{}", ConflictStatus::Resolved), "resolved");
    }

    #[test]
    fn test_conflict_resolution_display() {
        let res = ConflictResolution::HigherConfidence {
            chosen_belief_id: BeliefId::new(),
            confidence: 0.95,
        };
        assert!(format!("{res}").contains("higher_confidence"));
    }

    #[test]
    fn test_conflict_serialization() {
        let beliefs = vec![BeliefId::new()];
        let entity = EntityId::new();
        let conflict = Conflict::value_contradiction(beliefs, entity, "test");

        let json = serde_json::to_string(&conflict).unwrap();
        let deserialized: Conflict = serde_json::from_str(&json).unwrap();
        assert_eq!(conflict.id, deserialized.id);
    }
}
