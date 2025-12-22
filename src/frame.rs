//! BeliefFrame—the structured response type for RESOLVE operations.
//!
//! Unlike traditional databases that return flat result sets,
//! KyroQL returns BeliefFrames that include the answer, evidence,
//! conflicts, and knowledge gaps.

use serde::{Deserialize, Serialize};

use crate::confidence::{BeliefId, Confidence};
use crate::conflict::ConflictId;
use crate::entity::EntityId;
use crate::time::TimeRange;
use crate::value::Value;

/// Type of knowledge gap.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapType {
    /// No data exists for the requested predicate.
    NoPredicate {
        /// The missing predicate.
        predicate: String,
    },

    /// Entity exists but has no beliefs.
    NoBeliefs,

    /// Data exists but confidence is too low.
    LowConfidence {
        /// Highest confidence found (0-100).
        max_confidence: u8,
    },

    /// Data exists but is outdated.
    Outdated {
        /// When the most recent data is from.
        most_recent: String,
    },

    /// Unresolved conflicts prevent a clear answer.
    ConflictedWithNoResolution {
        /// Number of conflicting beliefs.
        conflict_count: usize,
    },

    /// Expected relationship is missing.
    MissingRelationship {
        /// Expected predicate.
        predicate: String,
        /// Expected target type.
        target_type: String,
    },
}

impl std::fmt::Display for GapType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPredicate { predicate } => write!(f, "no data for '{predicate}'"),
            Self::NoBeliefs => write!(f, "entity has no beliefs"),
            Self::LowConfidence { max_confidence } => {
                write!(f, "low confidence (max: {max_confidence}%)")
            }
            Self::Outdated { most_recent } => write!(f, "outdated (last: {most_recent})"),
            Self::ConflictedWithNoResolution { conflict_count } => {
                write!(f, "{conflict_count} unresolved conflicts")
            }
            Self::MissingRelationship {
                predicate,
                target_type,
            } => {
                write!(f, "missing '{predicate}' → {target_type}")
            }
        }
    }
}

/// A detected knowledge gap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeGap {
    /// The entity with missing knowledge.
    pub entity_id: EntityId,
    /// The type of gap.
    pub gap_type: GapType,
    /// Human-readable description.
    pub description: String,

    /// Optional suggestion for filling the gap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

impl KnowledgeGap {
    /// Creates a new knowledge gap.
    #[must_use]
    pub fn new(entity_id: EntityId, gap_type: GapType, description: impl Into<String>) -> Self {
        Self {
            entity_id,
            gap_type,
            description: description.into(),
            suggestion: None,
        }
    }

    /// Adds a suggestion for filling the gap.
    #[must_use]
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// Creates a "no predicate" gap.
    #[must_use]
    pub fn no_predicate(
        entity_id: EntityId,
        predicate: impl Into<String>,
    ) -> Self {
        let predicate = predicate.into();
        Self::new(
            entity_id,
            GapType::NoPredicate {
                predicate: predicate.clone(),
            },
            format!("No data found for predicate '{predicate}'"),
        )
    }

    /// Creates a "low confidence" gap.
    #[must_use]
    pub fn low_confidence(entity_id: EntityId, max_confidence: f32) -> Self {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let pct = (max_confidence.clamp(0.0, 1.0) * 100.0) as u8;
        Self::new(
            entity_id,
            GapType::LowConfidence {
                max_confidence: pct,
            },
            format!("Data exists but confidence is low ({pct}%)"),
        )
    }
}

/// A piece of evidence supporting or countering a claim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    /// ID of the belief serving as evidence.
    pub belief_id: BeliefId,
    /// Value of the belief.
    pub value: Value,
    /// Confidence of the belief.
    pub confidence: Confidence,

    /// Whether this supports (true) or counters (false) the claim.
    pub supports: bool,

    /// Weight of this evidence in the final answer (0.0 to 1.0).
    pub weight: f32,

    /// Optional explanation of why this evidence is relevant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
}

impl Evidence {
    /// Creates supporting evidence.
    #[must_use]
    pub fn supporting(belief_id: BeliefId, value: Value, confidence: Confidence) -> Self {
        Self {
            belief_id,
            value,
            confidence,
            supports: true,
            weight: 1.0,
            explanation: None,
        }
    }

    /// Creates counter evidence.
    #[must_use]
    pub fn counter(belief_id: BeliefId, value: Value, confidence: Confidence) -> Self {
        Self {
            belief_id,
            value,
            confidence,
            supports: false,
            weight: 1.0,
            explanation: None,
        }
    }

    /// Sets the weight.
    #[must_use]
    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight.clamp(0.0, 1.0);
        self
    }

    /// Sets the explanation.
    #[must_use]
    pub fn with_explanation(mut self, explanation: impl Into<String>) -> Self {
        self.explanation = Some(explanation.into());
        self
    }
}

/// A ranked claim from the evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankedClaim {
    /// The claimed value.
    pub value: Value,
    /// Derived confidence for this claim.
    pub confidence: Confidence,
    /// IDs of beliefs supporting this claim.
    pub supporting_belief_ids: Vec<BeliefId>,
    
    /// Rank of this claim (1 = best).
    pub rank: u32,
}

impl RankedClaim {
    /// Creates a new ranked claim.
    #[must_use]
    pub fn new(value: Value, confidence: Confidence, rank: u32) -> Self {
        Self {
            value,
            confidence,
            supporting_belief_ids: Vec::new(),
            rank,
        }
    }

    /// Adds a supporting belief ID.
    #[must_use]
    pub fn with_supporting(mut self, belief_id: BeliefId) -> Self {
        self.supporting_belief_ids.push(belief_id);
        self
    }
}

/// Assumptions made during query processing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QueryAssumptions {
    /// Assumed current time (if temporal query).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assumed_time: Option<chrono::DateTime<chrono::Utc>>,

    /// Assumed entity resolved to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_entity: Option<EntityId>,

    /// Assumed trust model used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust_model: Option<String>,

    /// Other assumptions.
    #[serde(default)]
    pub other: serde_json::Value,
}

/// The structured response type for RESOLVE operations.
///
/// Unlike a flat ResultSet, a BeliefFrame provides:
/// - The best supported claim
/// - All evidence (supporting and countering)
/// - Active conflicts
/// - Knowledge gaps
/// - Query assumptions
///
/// This enables agents to reason about the quality and completeness
/// of the information, not just the raw data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeliefFrame {
    /// The highest-ranked claim (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_supported_claim: Option<RankedClaim>,

    /// Evidence supporting the claim.
    #[serde(default)]
    pub supporting_evidence: Vec<Evidence>,

    /// Evidence countering the claim.
    #[serde(default)]
    pub counter_evidence: Vec<Evidence>,

    /// Active conflicts affecting this frame.
    #[serde(default)]
    pub conflicts: Vec<ConflictId>,

    /// Identified knowledge gaps.
    #[serde(default)]
    pub gaps: Vec<KnowledgeGap>,

    /// Time window considered.
    pub time_window: TimeRange,

    /// Assumptions made during query processing.
    #[serde(default)]
    pub query_assumptions: QueryAssumptions,

    /// Overall confidence in the answer (0.0 to 1.0).
    pub epistemic_confidence: f32,

    /// Relevance to the original query (0.0 to 1.0).
    pub retrieval_relevance: f32,

    /// Trace of the reasoning process.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_trace: Option<String>,
}

impl BeliefFrame {
    /// Creates a new empty belief frame.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            best_supported_claim: None,
            supporting_evidence: Vec::new(),
            counter_evidence: Vec::new(),
            conflicts: Vec::new(),
            gaps: Vec::new(),
            time_window: TimeRange::from_now(),
            query_assumptions: QueryAssumptions::default(),
            epistemic_confidence: 0.0,
            retrieval_relevance: 0.0,
            reasoning_trace: None,
        }
    }

    /// Creates a belief frame with a single answer.
    #[must_use]
    pub fn with_answer(value: Value, confidence: Confidence) -> Self {
        let conf_value = confidence.value();
        Self {
            best_supported_claim: Some(RankedClaim::new(value, confidence, 1)),
            supporting_evidence: Vec::new(),
            counter_evidence: Vec::new(),
            conflicts: Vec::new(),
            gaps: Vec::new(),
            time_window: TimeRange::from_now(),
            query_assumptions: QueryAssumptions::default(),
            epistemic_confidence: conf_value,
            retrieval_relevance: 1.0,
            reasoning_trace: None,
        }
    }

    /// Creates a belief frame indicating no answer was found.
    #[must_use]
    pub fn not_found(entity_id: EntityId, predicate: impl Into<String>) -> Self {
        let mut frame = Self::empty();
        frame.gaps.push(KnowledgeGap::no_predicate(entity_id, predicate));
        frame
    }

    /// Returns true if this frame has an answer.
    #[must_use]
    pub fn has_answer(&self) -> bool {
        self.best_supported_claim.is_some()
    }

    /// Returns true if this frame has conflicts.
    #[must_use]
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }

    /// Returns true if this frame has gaps.
    #[must_use]
    pub fn has_gaps(&self) -> bool {
        !self.gaps.is_empty()
    }

    /// Returns the number of supporting evidence pieces.
    #[must_use]
    pub fn evidence_count(&self) -> usize {
        self.supporting_evidence.len() + self.counter_evidence.len()
    }

    /// Adds supporting evidence.
    pub fn add_supporting_evidence(&mut self, evidence: Evidence) {
        self.supporting_evidence.push(evidence);
    }

    /// Adds counter evidence.
    pub fn add_counter_evidence(&mut self, evidence: Evidence) {
        self.counter_evidence.push(evidence);
    }

    /// Adds a conflict.
    pub fn add_conflict(&mut self, conflict_id: ConflictId) {
        self.conflicts.push(conflict_id);
    }

    /// Adds a knowledge gap.
    pub fn add_gap(&mut self, gap: KnowledgeGap) {
        self.gaps.push(gap);
    }

    /// Sets the reasoning trace.
    pub fn with_reasoning_trace(mut self, trace: impl Into<String>) -> Self {
        self.reasoning_trace = Some(trace.into());
        self
    }
}

impl Default for BeliefFrame {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_belief_frame_empty() {
        let frame = BeliefFrame::empty();
        assert!(!frame.has_answer());
        assert!(!frame.has_conflicts());
        assert!(!frame.has_gaps());
        assert_eq!(frame.evidence_count(), 0);
    }

    #[test]
    fn test_belief_frame_with_answer() {
        let frame = BeliefFrame::with_answer(
            Value::Bool(true),
            Confidence::from_agent(0.9, "test").unwrap(),
        );

        assert!(frame.has_answer());
        assert!((frame.epistemic_confidence - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn test_belief_frame_not_found() {
        let entity = EntityId::new();
        let frame = BeliefFrame::not_found(entity, "temperature");

        assert!(!frame.has_answer());
        assert!(frame.has_gaps());
        assert_eq!(frame.gaps.len(), 1);
    }

    #[test]
    fn test_belief_frame_add_evidence() {
        let mut frame = BeliefFrame::empty();

        frame.add_supporting_evidence(Evidence::supporting(
            BeliefId::new(),
            Value::Float(25.0),
            Confidence::from_agent(0.8, "test").unwrap(),
        ));

        frame.add_counter_evidence(Evidence::counter(
            BeliefId::new(),
            Value::Float(30.0),
            Confidence::from_agent(0.6, "test").unwrap(),
        ));

        assert_eq!(frame.evidence_count(), 2);
        assert_eq!(frame.supporting_evidence.len(), 1);
        assert_eq!(frame.counter_evidence.len(), 1);
    }

    #[test]
    fn test_belief_frame_add_conflict() {
        let mut frame = BeliefFrame::empty();
        assert!(!frame.has_conflicts());

        frame.add_conflict(ConflictId::new());
        assert!(frame.has_conflicts());
    }

    #[test]
    fn test_belief_frame_add_gap() {
        let mut frame = BeliefFrame::empty();
        assert!(!frame.has_gaps());

        frame.add_gap(KnowledgeGap::no_predicate(EntityId::new(), "test"));
        assert!(frame.has_gaps());
    }

    #[test]
    fn test_belief_frame_reasoning_trace() {
        let frame = BeliefFrame::empty()
            .with_reasoning_trace("Step 1 → Step 2 → Answer");

        assert!(frame.reasoning_trace.is_some());
        assert!(frame.reasoning_trace.as_ref().unwrap().contains("Step 1"));
    }

    #[test]
    fn test_knowledge_gap_no_predicate() {
        let gap = KnowledgeGap::no_predicate(EntityId::new(), "temperature");
        assert!(gap.description.contains("temperature"));
    }

    #[test]
    fn test_knowledge_gap_low_confidence() {
        let gap = KnowledgeGap::low_confidence(EntityId::new(), 0.3);
        assert!(gap.description.contains("30%"));
    }

    #[test]
    fn test_knowledge_gap_with_suggestion() {
        let gap = KnowledgeGap::no_predicate(EntityId::new(), "temp")
            .with_suggestion("Try querying the sensor API");

        assert!(gap.suggestion.is_some());
    }

    #[test]
    fn test_gap_type_display() {
        let gap = GapType::NoPredicate {
            predicate: "test".to_string(),
        };
        assert!(format!("{gap}").contains("test"));
    }

    #[test]
    fn test_evidence_supporting() {
        let evidence = Evidence::supporting(
            BeliefId::new(),
            Value::Float(25.0),
            Confidence::from_agent(0.9, "test").unwrap(),
        );

        assert!(evidence.supports);
        assert!((evidence.weight - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_evidence_counter() {
        let evidence = Evidence::counter(
            BeliefId::new(),
            Value::Float(30.0),
            Confidence::from_agent(0.7, "test").unwrap(),
        );

        assert!(!evidence.supports);
    }

    #[test]
    fn test_evidence_with_weight() {
        let evidence = Evidence::supporting(
            BeliefId::new(),
            Value::Bool(true),
            Confidence::from_agent(0.5, "test").unwrap(),
        )
        .with_weight(0.7);

        assert!((evidence.weight - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn test_ranked_claim() {
        let claim = RankedClaim::new(
            Value::String("answer".into()),
            Confidence::from_agent(0.9, "test").unwrap(),
            1,
        )
        .with_supporting(BeliefId::new())
        .with_supporting(BeliefId::new());

        assert_eq!(claim.rank, 1);
        assert_eq!(claim.supporting_belief_ids.len(), 2);
    }

    #[test]
    fn test_belief_frame_serialization() {
        let frame = BeliefFrame::with_answer(
            Value::Int(42),
            Confidence::from_agent(0.9, "test").unwrap(),
        );

        let json = serde_json::to_string(&frame).unwrap();
        let deserialized: BeliefFrame = serde_json::from_str(&json).unwrap();

        assert!(deserialized.has_answer());
        assert!((deserialized.epistemic_confidence - 0.9).abs() < f32::EPSILON);
    }
}
