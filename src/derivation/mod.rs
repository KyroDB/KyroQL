//! Derivation records.
//!
//! DERIVE is KyroQL's inference recorder: it stores explicit links between
//! a derived belief and the premise beliefs and rules used to produce it.
//! This enables audit trails and future re-evaluation when premises change.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::confidence::BeliefId;
use crate::error::ValidationError;

/// Stable identifier for a derivation record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DerivationId(uuid::Uuid);

impl DerivationId {
    /// Creates a new random derivation ID.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for DerivationId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for DerivationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Immutable derivation record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DerivationRecord {
    /// Derivation record identifier.
    pub id: DerivationId,

    /// Transaction time: when this derivation was recorded.
    pub tx_time: DateTime<Utc>,

    /// Optional belief produced by this derivation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_belief_id: Option<BeliefId>,

    /// Premise belief IDs.
    pub premise_ids: Vec<BeliefId>,

    /// Derivation rule identifier/name.
    pub rule: String,

    /// Optional list of inference steps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inference_steps: Vec<String>,

    /// Optional propagated confidence for the derived belief.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub propagated_confidence: Option<f32>,

    /// Optional human-readable justification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,

    /// Optional extensible metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl DerivationRecord {
    /// Conservative bound on premise list size.
    pub const MAX_PREMISES: usize = 1024;

    /// Conservative bound on number of inference steps.
    pub const MAX_STEPS: usize = 256;

    /// Conservative bound on metadata size (serialized JSON bytes).
    pub const MAX_METADATA_BYTES: usize = 64 * 1024;

    /// Construct a new derivation record with validation.
    pub fn new(
        tx_time: DateTime<Utc>,
        derived_belief_id: Option<BeliefId>,
        premise_ids: Vec<BeliefId>,
        rule: impl Into<String>,
        inference_steps: Vec<String>,
        propagated_confidence: Option<f32>,
        justification: Option<String>,
        metadata: Option<serde_json::Value>,
    ) -> Result<Self, ValidationError> {
        let rule = rule.into();
        let trimmed_rule = rule.trim();
        if trimmed_rule.is_empty() {
            return Err(ValidationError::MissingField {
                field: "rule".to_string(),
            });
        }
        if premise_ids.is_empty() {
            return Err(ValidationError::MissingField {
                field: "premise_ids".to_string(),
            });
        }
        if premise_ids.len() > Self::MAX_PREMISES {
            return Err(ValidationError::FieldTooLong {
                field: "premise_ids".to_string(),
                max_length: Self::MAX_PREMISES,
            });
        }
        if inference_steps.len() > Self::MAX_STEPS {
            return Err(ValidationError::FieldTooLong {
                field: "inference_steps".to_string(),
                max_length: Self::MAX_STEPS,
            });
        }
        if let Some(v) = propagated_confidence {
            if !(0.0..=1.0).contains(&v) || v.is_nan() {
                return Err(ValidationError::ConfidenceOutOfRange { value: v });
            }
        }

        if let Some(ref j) = justification {
            if j.trim().is_empty() {
                return Err(ValidationError::MissingField {
                    field: "justification".to_string(),
                });
            }
        }

        if let Some(ref meta) = metadata {
            let bytes = serde_json::to_vec(meta).map_err(|e| ValidationError::InvalidField {
                field: "metadata".to_string(),
                reason: format!("failed to serialize metadata: {e}"),
            })?;
            if bytes.len() > Self::MAX_METADATA_BYTES {
                return Err(ValidationError::FieldTooLong {
                    field: "metadata".to_string(),
                    max_length: Self::MAX_METADATA_BYTES,
                });
            }
        }

        Ok(Self {
            id: DerivationId::new(),
            tx_time,
            derived_belief_id,
            premise_ids,
            rule: trimmed_rule.to_string(),
            inference_steps,
            propagated_confidence,
            justification,
            metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivation_record_validation_requires_rule_and_sources() {
        let now = Utc::now();
        assert!(DerivationRecord::new(
            now,
            None,
            vec![],
            "r",
            Vec::new(),
            None,
            None,
            None
        )
        .is_err());

        assert!(DerivationRecord::new(
            now,
            None,
            vec![BeliefId::new()],
            "  ",
            Vec::new(),
            None,
            None,
            None
        )
        .is_err());

        let ok = DerivationRecord::new(
            now,
            Some(BeliefId::new()),
            vec![BeliefId::new()],
            "modus_ponens",
            vec!["step".to_string()],
            Some(0.7),
            Some("because".to_string()),
            Some(serde_json::json!({"k": "v"})),
        )
        .unwrap();
        assert_eq!(ok.rule, "modus_ponens");
        assert_eq!(ok.premise_ids.len(), 1);
    }
}
