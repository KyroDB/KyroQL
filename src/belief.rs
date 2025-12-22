//! Belief types—the atomic unit of knowledge in KyroQL.
//!
//! A Belief is not just data; it is a claim about reality with
//! explicit confidence, provenance, and temporal validity.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::confidence::{BeliefId, Confidence};
use crate::entity::EntityId;
use crate::source::Source;
use crate::time::TimeRange;
use crate::value::Value;
use crate::error::ValidationError;

use crate::conflict::ConflictId;

/// Consistency status of a belief within the knowledge base.
///
/// This enum tracks the *consistency* of a belief, not its lifecycle.
/// Lifecycle states (e.g., supersession) are tracked separately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ConsistencyStatus {
    /// Checked against patterns and other beliefs, passed all checks.
    Verified,
    /// Accepted but not yet checked for consistency.
    Provisional,
    /// Conflicts with other beliefs or violates patterns.
    Contested {
        /// IDs of the conflicts this belief is involved in.
        conflict_ids: Vec<ConflictId>,
    },
}

impl ConsistencyStatus {
    /// Returns true if this status indicates the belief is contested.
    #[must_use]
    pub fn is_contested(&self) -> bool {
        matches!(self, Self::Contested { .. })
    }

    /// Returns the conflict IDs if contested, empty slice otherwise.
    #[must_use]
    pub fn conflict_ids(&self) -> &[ConflictId] {
        match self {
            Self::Contested { conflict_ids } => conflict_ids,
            _ => &[],
        }
    }
}

impl Default for ConsistencyStatus {
    fn default() -> Self {
        Self::Provisional
    }
}

impl fmt::Display for ConsistencyStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Verified => write!(f, "verified"),
            Self::Provisional => write!(f, "provisional"),
            Self::Contested { conflict_ids } => {
                write!(f, "contested({} conflicts)", conflict_ids.len())
            }
        }
    }
}

/// The atomic unit of knowledge in KyroQL.
///
/// A Belief is not just data; it is a claim about reality with:
/// - Explicit confidence (with calibration semantics)
/// - Full provenance (where did this come from?)
/// - Bitemporal validity (valid_time + transaction_time)
/// - Consistency tracking (conflicts, supersession)
///
/// # Examples
///
/// ```
/// use kyroql::{Belief, EntityId, Confidence, Source, Value, TimeRange};
///
/// let belief = Belief::builder()
///     .subject(EntityId::new())
///     .predicate("is_superconductor")
///     .value(false)
///     .confidence(Confidence::from_agent(0.95, "researcher-1").unwrap())
///     .source(Source::paper("2307.12008", "LK-99 Analysis"))
///     .build()
///     .unwrap();
///
/// assert_eq!(belief.predicate, "is_superconductor");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Belief {
    /// Unique identifier for this belief.
    pub id: BeliefId,
    /// The entity this belief is about.
    pub subject: EntityId,
    /// The attribute/relationship being asserted.
    pub predicate: String,
    /// The value being asserted.
    pub value: Value,
    /// Confidence in this belief.
    pub confidence: Confidence,
    /// Provenance of this belief.
    pub source: Source,
    /// When this belief is/was valid in the real world.
    pub valid_time: TimeRange,
    /// When this belief was recorded in the system.
    pub tx_time: DateTime<Utc>,
    /// Current consistency status.
    pub consistency_status: ConsistencyStatus,
    /// ID of the belief this one supersedes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<BeliefId>,
    /// ID of the belief that superseded this one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<BeliefId>,
    /// Optional embedding for semantic search.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}

impl Belief {
    /// Creates a new builder for constructing a Belief.
    pub fn builder() -> BeliefBuilder {
        BeliefBuilder::new()
    }

    /// Returns true if this belief is currently active (not superseded).
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.superseded_by.is_none()
    }

    /// Returns true if this belief is currently active and temporally valid now.
    #[must_use]
    pub fn is_valid_now(&self) -> bool {
        let now = Utc::now();
        self.is_valid_at(now)
    }

    /// Returns true if this belief is currently active and temporally valid at a specific time.
    #[must_use]
    pub fn is_valid_at(&self, time: DateTime<Utc>) -> bool {
        self.is_active() && self.valid_time.contains(time) && time >= self.tx_time
    }

    /// Returns true if superseded.
    pub fn is_superseded(&self) -> bool {
        self.superseded_by.is_some()
    }

    /// Returns true if contested (has conflicts).
    pub fn is_contested(&self) -> bool {
        self.consistency_status.is_contested()
    }

    /// Returns true if this belief has an embedding.
    #[must_use]
    pub fn has_embedding(&self) -> bool {
        self.embedding.is_some()
    }

    /// Sets the embedding for this belief.
    pub fn set_embedding(&mut self, embedding: Vec<f32>) {
        self.embedding = Some(embedding);
    }

    /// Marks this belief as superseded by another.
    pub fn mark_superseded(&mut self, by: BeliefId) {
        self.superseded_by = Some(by);
    }

    /// Marks this belief as contested with the given conflicts.
    pub fn mark_contested(&mut self, conflict_ids: Vec<ConflictId>) {
        self.consistency_status = ConsistencyStatus::Contested { conflict_ids };
    }

    /// Marks this belief as verified (passed all consistency checks).
    pub fn mark_verified(&mut self) {
        self.consistency_status = ConsistencyStatus::Verified;
    }

}

impl PartialEq for Belief {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Belief {}

impl std::hash::Hash for Belief {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Builder for creating Belief instances.
///
/// Ensures all required fields are set before building.
#[derive(Debug, Default)]
pub struct BeliefBuilder {
    id: Option<BeliefId>,
    subject: Option<EntityId>,
    predicate: Option<String>,
    value: Option<Value>,
    confidence: Option<Confidence>,
    source: Option<Source>,
    valid_time: Option<TimeRange>,
    supersedes: Option<BeliefId>,
    embedding: Option<Vec<f32>>,
}

impl BeliefBuilder {
    /// Creates a new belief builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the belief ID (optional, will be generated if not set).
    #[must_use]
    pub fn id(mut self, id: BeliefId) -> Self {
        self.id = Some(id);
        self
    }

    /// Sets the subject entity.
    #[must_use]
    pub fn subject(mut self, subject: EntityId) -> Self {
        self.subject = Some(subject);
        self
    }

    /// Sets the predicate.
    #[must_use]
    pub fn predicate(mut self, predicate: impl Into<String>) -> Self {
        self.predicate = Some(predicate.into());
        self
    }

    /// Sets the value.
    #[must_use]
    pub fn value(mut self, value: impl Into<Value>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Sets the confidence.
    #[must_use]
    pub fn confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }

    /// Sets the source.
    #[must_use]
    pub fn source(mut self, source: Source) -> Self {
        self.source = Some(source);
        self
    }

    /// Sets the valid time range.
    #[must_use]
    pub fn valid_time(mut self, valid_time: TimeRange) -> Self {
        self.valid_time = Some(valid_time);
        self
    }

    /// Sets the belief this one supersedes.
    #[must_use]
    pub fn supersedes(mut self, supersedes: BeliefId) -> Self {
        self.supersedes = Some(supersedes);
        self
    }

    /// Sets the embedding.
    #[must_use]
    pub fn embedding(mut self, embedding: Vec<f32>) -> Self {
        self.embedding = Some(embedding);
        self
    }

    /// Builds the Belief.
    /// Returns `ValidationError` if required fields are missing or invalid.
    pub fn build(self) -> Result<Belief, ValidationError> {
        let subject = self.subject.ok_or(ValidationError::MissingField {
            field: "subject".to_string(),
        })?;

        let predicate = self.predicate.ok_or(ValidationError::MissingField {
            field: "predicate".to_string(),
        })?;

        if predicate.trim().is_empty() {
            return Err(ValidationError::EmptyPredicate);
        }

        let value = self.value.ok_or(ValidationError::MissingField {
            field: "value".to_string(),
        })?;

        let confidence = self.confidence.ok_or(ValidationError::MissingField {
            field: "confidence".to_string(),
        })?;

        let source = self.source.unwrap_or_default();
        let valid_time = self.valid_time.unwrap_or_else(TimeRange::from_now);

        Ok(Belief {
            id: self.id.unwrap_or_else(BeliefId::new),
            subject,
            predicate,
            value,
            confidence,
            source,
            valid_time,
            tx_time: Utc::now(),
            consistency_status: ConsistencyStatus::Provisional,
            supersedes: self.supersedes,
            superseded_by: None,
            embedding: self.embedding,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_belief() -> Belief {
        Belief::builder()
            .subject(EntityId::new())
            .predicate("test_predicate")
            .value(true)
            .confidence(Confidence::from_agent(0.9, "test").unwrap())
            .build()
            .unwrap()
    }

    #[test]
    fn test_belief_builder_success() {
        let belief = make_test_belief();
        assert_eq!(belief.predicate, "test_predicate");
        assert_eq!(belief.value, Value::Bool(true));
        assert!(belief.is_active());
    }

    #[test]
    fn test_belief_builder_missing_subject() {
        let result = Belief::builder()
            .predicate("test")
            .value(true)
            .confidence(Confidence::from_agent(0.5, "test").unwrap())
            .build();

        assert!(result.is_err());
        if let Err(ValidationError::MissingField { field }) = result {
            assert_eq!(field, "subject");
        } else {
            panic!("Expected MissingField error");
        }
    }

    #[test]
    fn test_belief_builder_missing_predicate() {
        let result = Belief::builder()
            .subject(EntityId::new())
            .value(true)
            .confidence(Confidence::from_agent(0.5, "test").unwrap())
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_belief_builder_empty_predicate() {
        let result = Belief::builder()
            .subject(EntityId::new())
            .predicate("")
            .value(true)
            .confidence(Confidence::from_agent(0.5, "test").unwrap())
            .build();

        assert!(matches!(result, Err(ValidationError::EmptyPredicate)));
    }

    #[test]
    fn test_belief_builder_whitespace_predicate() {
        let result = Belief::builder()
            .subject(EntityId::new())
            .predicate("   ")
            .value(true)
            .confidence(Confidence::from_agent(0.5, "test").unwrap())
            .build();

        assert!(matches!(result, Err(ValidationError::EmptyPredicate)));
    }

    #[test]
    fn test_belief_builder_with_all_fields() {
        let subject = EntityId::new();
        let supersedes = BeliefId::new();
        let embedding = vec![0.1, 0.2, 0.3];

        let belief = Belief::builder()
            .subject(subject)
            .predicate("test")
            .value(42)
            .confidence(Confidence::from_agent(0.9, "agent").unwrap())
            .source(Source::agent("test-agent", Some("v1")))
            .valid_time(TimeRange::from_now())
            .supersedes(supersedes)
            .embedding(embedding.clone())
            .build()
            .unwrap();

        assert_eq!(belief.subject, subject);
        assert_eq!(belief.supersedes, Some(supersedes));
        assert!(belief.has_embedding());
        assert_eq!(belief.embedding.as_ref().unwrap(), &embedding);
    }

    #[test]
    fn test_belief_is_active() {
        let mut belief = make_test_belief();
        assert!(belief.is_active());

        belief.mark_superseded(BeliefId::new());
        assert!(!belief.is_active());
    }

    #[test]
    fn test_belief_mark_contested() {
        let mut belief = make_test_belief();
        assert!(!belief.is_contested());

        belief.mark_contested(vec![]);
        assert!(belief.is_contested());
        assert!(belief.is_active()); // Contested beliefs are still active
    }

    #[test]
    fn test_belief_mark_verified() {
        let mut belief = make_test_belief();
        belief.mark_verified();
        assert_eq!(belief.consistency_status, ConsistencyStatus::Verified);
    }

    #[test]
    fn test_belief_is_valid_now() {
        let belief = make_test_belief();
        assert!(belief.is_valid_now());
    }

    #[test]
    fn test_belief_equality() {
        let belief1 = make_test_belief();
        let belief2 = Belief::builder()
            .id(belief1.id)
            .subject(EntityId::new())
            .predicate("different")
            .value("different")
            .confidence(Confidence::from_agent(0.1, "test").unwrap())
            .build()
            .unwrap();

        // Same ID = equal
        assert_eq!(belief1, belief2);
    }

    #[test]
    fn test_belief_set_embedding() {
        let mut belief = make_test_belief();
        assert!(!belief.has_embedding());

        belief.set_embedding(vec![1.0, 2.0, 3.0]);
        assert!(belief.has_embedding());
    }

    #[test]
    fn test_consistency_status_display() {
        assert_eq!(format!("{}", ConsistencyStatus::Verified), "verified");
        assert_eq!(format!("{}", ConsistencyStatus::Provisional), "provisional");
        assert_eq!(
            format!("{}", ConsistencyStatus::Contested { conflict_ids: vec![] }),
            "contested(0 conflicts)"
        );
    }

    #[test]
    fn test_consistency_status_serde_shape() {
        let verified = serde_json::to_value(ConsistencyStatus::Verified).unwrap();
        assert_eq!(verified, serde_json::json!({"status": "verified"}));

        let provisional = serde_json::to_value(ConsistencyStatus::Provisional).unwrap();
        assert_eq!(provisional, serde_json::json!({"status": "provisional"}));

        let conflict_id = ConflictId::new();
        let contested = serde_json::to_value(ConsistencyStatus::Contested {
            conflict_ids: vec![conflict_id],
        })
        .unwrap();

        assert_eq!(contested["status"], serde_json::json!("contested"));
        assert!(contested["conflict_ids"].is_array());
        assert_eq!(contested["conflict_ids"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_belief_serialization() {
        let belief = make_test_belief();
        let json = serde_json::to_string(&belief).unwrap();
        let deserialized: Belief = serde_json::from_str(&json).unwrap();
        assert_eq!(belief.id, deserialized.id);
        assert_eq!(belief.predicate, deserialized.predicate);
    }
}
