//! Structured response types for RESOLVE.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::belief::Belief;
use crate::confidence::BeliefId;
use crate::conflict::Conflict;
use crate::entity::EntityId;
use crate::inference::ConflictResolutionPolicy;
use crate::source::Source;
use crate::time::TimeRange;

/// A ranked claim with separate confidence and relevance scores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedClaim {
    pub belief: Belief,

    /// Epistemic confidence: Is this claim true? (0.0 - 1.0)
    pub epistemic_confidence: f32,

    /// Retrieval relevance: Is this relevant to the query? (0.0 - 1.0)
    pub retrieval_relevance: f32,

    /// Combined score (arithmetic mean of epistemic confidence and retrieval relevance)
    pub combined_score: f32,
}

impl RankedClaim {
    #[must_use]
    pub fn new(belief: Belief, epistemic_confidence: f32, retrieval_relevance: f32) -> Self {
        let epistemic_confidence = epistemic_confidence.clamp(0.0, 1.0);
        let retrieval_relevance = retrieval_relevance.clamp(0.0, 1.0);
        let combined_score = (epistemic_confidence + retrieval_relevance) * 0.5;

        Self {
            belief,
            epistemic_confidence,
            retrieval_relevance,
            combined_score,
        }
    }
}

/// A piece of evidence supporting or contradicting a claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub belief_id: BeliefId,
    pub summary: String,
    pub source: Source,
    pub confidence: f32,
    pub relevance: f32,
}

impl Evidence {
    #[must_use]
    pub fn new(
        belief_id: BeliefId,
        summary: impl Into<String>,
        source: Source,
        confidence: f32,
        relevance: f32,
    ) -> Self {
        Self {
            belief_id,
            summary: summary.into(),
            source,
            confidence: confidence.clamp(0.0, 1.0),
            relevance: relevance.clamp(0.0, 1.0),
        }
    }
}

/// Types of knowledge gaps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapType {
    NoDataFound,
    LowConfidenceOnly,
    ExpiredData,
    MissingEntity,
    InsufficientEvidence,
}

/// A detected gap in knowledge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeGap {
    pub gap_type: GapType,
    pub description: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_query: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_entity: Option<EntityId>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_predicate: Option<String>,
}

impl KnowledgeGap {
    #[must_use]
    pub fn new(gap_type: GapType, description: impl Into<String>) -> Self {
        Self {
            gap_type,
            description: description.into(),
            suggested_query: None,
            missing_entity: None,
            missing_predicate: None,
        }
    }

    #[must_use]
    pub fn missing_entity(description: impl Into<String>) -> Self {
        Self {
            gap_type: GapType::MissingEntity,
            description: description.into(),
            suggested_query: None,
            missing_entity: None,
            missing_predicate: None,
        }
    }

    #[must_use]
    pub fn with_suggested_query(mut self, query: impl Into<String>) -> Self {
        self.suggested_query = Some(query.into());
        self
    }

    #[must_use]
    pub fn with_missing_entity(mut self, entity_id: EntityId) -> Self {
        self.missing_entity = Some(entity_id);
        self
    }

    #[must_use]
    pub fn with_missing_predicate(mut self, predicate: impl Into<String>) -> Self {
        self.missing_predicate = Some(predicate.into());
        self
    }
}

/// Assumptions made during query execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryAssumptions {
    pub conflict_policy: ConflictResolutionPolicy,
    pub min_confidence: Option<f32>,
    pub trust_model: String,
    pub as_of_time: DateTime<Utc>,
}

impl Default for QueryAssumptions {
    fn default() -> Self {
        Self {
            conflict_policy: ConflictResolutionPolicy::default(),
            min_confidence: None,
            trust_model: "default".to_string(),
            as_of_time: Utc::now(),
        }
    }
}

/// The structured response type for RESOLVE operations.
/// Contains answer, evidence, conflicts, and gaps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeliefFrame {
    /// Primary answer (structured, not prose)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_supported_claim: Option<RankedClaim>,

    /// Supporting evidence
    pub supporting_evidence: Vec<Evidence>,

    /// Counter-evidence
    pub counter_evidence: Vec<Evidence>,

    /// Detected conflicts
    pub conflicts: Vec<Conflict>,

    /// Knowledge gaps
    pub gaps: Vec<KnowledgeGap>,

    /// Time window for query
    pub time_window: TimeRange,

    /// Assumptions made during execution
    pub query_assumptions: QueryAssumptions,

    /// For debugging only (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_summary: Option<String>,
}

impl BeliefFrame {
    /// Create an empty belief frame.
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
            debug_summary: None,
        }
    }

    #[must_use]
    pub fn has_answer(&self) -> bool {
        self.best_supported_claim.is_some()
    }

    #[must_use]
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }

    #[must_use]
    pub fn has_gaps(&self) -> bool {
        !self.gaps.is_empty()
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

    use crate::confidence::Confidence;
    use crate::source::Source;
    use crate::value::Value;

    #[test]
    fn ranked_claim_combined_score_is_mean() {
        let belief = crate::belief::Belief::builder()
            .subject(EntityId::new())
            .predicate("p")
            .value(Value::Bool(true))
            .confidence(Confidence::from_agent(0.8, "test").unwrap())
            .source(Source::agent("test", Option::<String>::None))
            .valid_time(TimeRange::from_now())
            .build()
            .unwrap();

        let claim = RankedClaim::new(belief, 0.8, 0.4);
        assert!((claim.combined_score - 0.6).abs() < 1e-6);
    }

    #[test]
    fn belief_frame_roundtrip_json() {
        let belief = crate::belief::Belief::builder()
            .subject(EntityId::new())
            .predicate("p")
            .value(Value::Int(1))
            .confidence(Confidence::from_agent(0.9, "test").unwrap())
            .source(Source::agent("test", Option::<String>::None))
            .valid_time(TimeRange::from_now())
            .build()
            .unwrap();

        let mut frame = BeliefFrame::empty();
        frame.best_supported_claim = Some(RankedClaim::new(belief.clone(), 0.9, 1.0));
        frame.supporting_evidence.push(Evidence::new(
            belief.id,
            "p",
            belief.source.clone(),
            belief.confidence.value(),
            1.0,
        ));

        let json = serde_json::to_string(&frame).unwrap();
        let back: BeliefFrame = serde_json::from_str(&json).unwrap();
        assert!(back.has_answer());
        assert_eq!(back.supporting_evidence.len(), 1);
    }
}
