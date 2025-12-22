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

// BeliefId is defined in confidence module - re-export not needed here

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsistencyStatus {
    Pending,
    Consistent,
    Conflicted,
    Superseded,
    Retracted,
}

impl Default for ConsistencyStatus {
    fn default() -> Self {
        Self::Pending
    }
}

impl fmt::Display for ConsistencyStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Consistent => write!(f, "consistent"),
            Self::Conflicted => write!(f, "conflicted"),
            Self::Superseded => write!(f, "superseded"),
            Self::Retracted => write!(f, "retracted"),
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
///     .confidence(Confidence::probability(0.95, "researcher-1").unwrap())
///     .source(Source::paper("2307.12008", "LK-99 Analysis"))
///     .build()
///     .unwrap();
///
/// assert_eq!(belief.predicate, "is_superconductor");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Belief {
    pub id: BeliefId,
    pub subject: EntityId,
    pub predicate: String,
    pub value: Value,
    pub confidence: Confidence,
    pub source: Source,
    pub valid_time: TimeRange,
    
    /// When this belief was recorded in the system.
    pub tx_time: DateTime<Utc>,

    pub consistency_status: ConsistencyStatus,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<BeliefId>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<BeliefId>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,

    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl Belief {
    pub fn builder() -> BeliefBuilder {
        BeliefBuilder::new()
    }

    /// Returns true if this belief is currently active (not superseded or retracted).
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(
            self.consistency_status,
            ConsistencyStatus::Pending | ConsistencyStatus::Consistent | ConsistencyStatus::Conflicted
        ) && self.superseded_by.is_none()
    }

    pub fn is_valid_now(&self) -> bool {
        self.is_active() && self.valid_time.is_active()
    }

    pub fn is_valid_at(&self, time: DateTime<Utc>) -> bool {
        self.valid_time.contains(time) && time >= self.tx_time
    }

    pub fn is_superseded(&self) -> bool {
        self.superseded_by.is_some() || self.consistency_status == ConsistencyStatus::Superseded
    }

    pub fn is_conflicted(&self) -> bool {
        self.consistency_status == ConsistencyStatus::Conflicted
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
        self.consistency_status = ConsistencyStatus::Superseded;
    }

    /// Marks this belief as conflicted.
    pub fn mark_conflicted(&mut self) {
        self.consistency_status = ConsistencyStatus::Conflicted;
    }

    /// Marks this belief as consistent.
    pub fn mark_consistent(&mut self) {
        self.consistency_status = ConsistencyStatus::Consistent;
    }

    /// Marks this belief as retracted.
    pub fn mark_retracted(&mut self) {
        self.consistency_status = ConsistencyStatus::Retracted;
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
    metadata: Option<serde_json::Value>,
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

    /// Sets the metadata.
    #[must_use]
    pub fn metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
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

        if predicate.is_empty() {
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
            consistency_status: ConsistencyStatus::Pending,
            supersedes: self.supersedes,
            superseded_by: None,
            embedding: self.embedding,
            metadata: self.metadata.unwrap_or(serde_json::Value::Null),
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
            .confidence(Confidence::probability(0.9, "test").unwrap())
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
            .confidence(Confidence::heuristic(0.5, "test").unwrap())
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
            .confidence(Confidence::heuristic(0.5, "test").unwrap())
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_belief_builder_empty_predicate() {
        let result = Belief::builder()
            .subject(EntityId::new())
            .predicate("")
            .value(true)
            .confidence(Confidence::heuristic(0.5, "test").unwrap())
            .build();

        assert!(matches!(result, Err(ValidationError::EmptyPredicate)));
    }

    #[test]
    fn test_belief_builder_with_all_fields() {
        let subject = EntityId::new();
        let supersedes = BeliefId::new();
        let embedding = vec![0.1, 0.2, 0.3];
        let metadata = serde_json::json!({"key": "value"});

        let belief = Belief::builder()
            .subject(subject)
            .predicate("test")
            .value(42)
            .confidence(Confidence::probability(0.9, "agent").unwrap())
            .source(Source::agent("test-agent", Some("v1")))
            .valid_time(TimeRange::from_now())
            .supersedes(supersedes)
            .embedding(embedding.clone())
            .metadata(metadata.clone())
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
    fn test_belief_mark_conflicted() {
        let mut belief = make_test_belief();
        assert!(!belief.is_conflicted());

        belief.mark_conflicted();
        assert!(belief.is_conflicted());
        assert!(belief.is_active()); // Conflicted beliefs are still active
    }

    #[test]
    fn test_belief_mark_retracted() {
        let mut belief = make_test_belief();
        belief.mark_retracted();
        assert!(!belief.is_active());
        assert_eq!(belief.consistency_status, ConsistencyStatus::Retracted);
    }

    #[test]
    fn test_belief_mark_consistent() {
        let mut belief = make_test_belief();
        belief.mark_consistent();
        assert_eq!(belief.consistency_status, ConsistencyStatus::Consistent);
    }

    #[test]
    fn test_belief_is_valid_now() {
        let belief = make_test_belief();
        assert!(belief.is_valid_now());
    }

    #[test]
    fn test_belief_equality() {
        let belief1 = make_test_belief();
        let mut belief2 = Belief::builder()
            .id(belief1.id)
            .subject(EntityId::new())
            .predicate("different")
            .value("different")
            .confidence(Confidence::heuristic(0.1, "test").unwrap())
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
        assert_eq!(format!("{}", ConsistencyStatus::Pending), "pending");
        assert_eq!(format!("{}", ConsistencyStatus::Consistent), "consistent");
        assert_eq!(format!("{}", ConsistencyStatus::Conflicted), "conflicted");
        assert_eq!(format!("{}", ConsistencyStatus::Superseded), "superseded");
        assert_eq!(format!("{}", ConsistencyStatus::Retracted), "retracted");
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
