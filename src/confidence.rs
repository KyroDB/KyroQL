//! Confidence types with calibration semantics.
//!
//! Confidence in KyroQL is not just a number—it must have explicit
//! calibration semantics that explain how to interpret the value.
//! Without calibration, confidence is meaningless.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::ValidationError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BeliefId(uuid::Uuid);

impl BeliefId {
    /// Creates a new random belief ID.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for BeliefId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for BeliefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(uuid::Uuid);

impl SourceId {
    /// Creates a new random source ID.
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for SourceId {
    fn default() -> Self {
        Self::new()
    }
}

/// How to interpret the confidence value.
///
/// This is critical: confidence without calibration is meaningless.
/// A value of 0.8 means different things depending on the calibration mode.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationMode {
    /// Historically, ~X% of claims with this confidence are true.
    Probability,

    /// Uncalibrated score. Prefer `Probability` where possible.
    Heuristic,

    /// Normalized model log-probability.
    ModelLogprob,

    /// Computed from source reliability scores.
    SourceWeighted,
}

impl Default for CalibrationMode {
    fn default() -> Self {
        Self::Heuristic
    }
}

impl fmt::Display for CalibrationMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Probability => write!(f, "probability"),
            Self::Heuristic => write!(f, "heuristic"),
            Self::ModelLogprob => write!(f, "model_logprob"),
            Self::SourceWeighted => write!(f, "source_weighted"),
        }
    }
}

/// Provenance of the confidence assignment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConfidenceSource {
    AssertedByAgent {
        agent_id: String,
    },

    ComputedByModel {
        model_id: String,
        model_version: String,
    },

    AggregatedFromSources {
        source_ids: Vec<SourceId>,
        aggregation_method: String,
    },

    DerivedFromPremises {
        premise_ids: Vec<BeliefId>,
        propagation_rule: String,
    },

    Unknown,
}

impl Default for ConfidenceSource {
    fn default() -> Self {
        Self::Unknown
    }
}

/// Formalized uncertainty.
///
/// Confidence values must always have calibration and provenance.
/// This ensures that confidence is interpretable and traceable.
///
/// # Examples
///
/// ```
/// use kyroql::{Confidence, CalibrationMode, ConfidenceSource};
///
/// // Create a calibrated probability confidence
/// let conf = Confidence::probability(0.95, "my-agent").unwrap();
/// assert_eq!(conf.value(), 0.95);
/// assert_eq!(conf.calibration, CalibrationMode::Probability);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Confidence {
    /// The confidence value (0.0 to 1.0, inclusive).
    value: f32,

    /// How to interpret this value.
    pub calibration: CalibrationMode,

    /// Who/what assigned this confidence.
    pub source: ConfidenceSource,
}

impl Confidence {
    /// Minimum valid confidence value.
    pub const MIN_VALUE: f32 = 0.0;

    /// Maximum valid confidence value.
    pub const MAX_VALUE: f32 = 1.0;

    /// Creates a new confidence with validation.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError::ConfidenceOutOfRange` if the value is not in [0.0, 1.0].
    pub fn new(
        value: f32,
        calibration: CalibrationMode,
        source: ConfidenceSource,
    ) -> Result<Self, ValidationError> {
        Self::validate_value(value)?;
        Ok(Self {
            value,
            calibration,
            source,
        })
    }

    /// Creates a calibrated probability confidence asserted by an agent.
    /// Returns `ValidationError` if value not in [0.0, 1.0].
    pub fn probability(value: f32, agent_id: impl Into<String>) -> Result<Self, ValidationError> {
        Self::new(
            value,
            CalibrationMode::Probability,
            ConfidenceSource::AssertedByAgent {
                agent_id: agent_id.into(),
            },
        )
    }

    /// Creates a heuristic confidence.
    pub fn heuristic(value: f32, agent_id: impl Into<String>) -> Result<Self, ValidationError> {
        Self::new(
            value,
            CalibrationMode::Heuristic,
            ConfidenceSource::AssertedByAgent {
                agent_id: agent_id.into(),
            },
        )
    }

    /// Creates a confidence derived from model output.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError::ConfidenceOutOfRange` if the value is not in [0.0, 1.0].
    pub fn from_model(
        value: f32,
        model_id: impl Into<String>,
        model_version: impl Into<String>,
    ) -> Result<Self, ValidationError> {
        Self::new(
            value,
            CalibrationMode::ModelLogprob,
            ConfidenceSource::ComputedByModel {
                model_id: model_id.into(),
                model_version: model_version.into(),
            },
        )
    }

    /// Creates an unknown/default confidence.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError::ConfidenceOutOfRange` if the value is not in [0.0, 1.0].
    pub fn unknown(value: f32) -> Result<Self, ValidationError> {
        Self::new(value, CalibrationMode::Heuristic, ConfidenceSource::Unknown)
    }

    /// Creates a zero confidence (complete uncertainty).
    #[must_use]
    pub fn zero() -> Self {
        Self {
            value: 0.0,
            calibration: CalibrationMode::Heuristic,
            source: ConfidenceSource::Unknown,
        }
    }

    /// Creates a full confidence (complete certainty).
    #[must_use]
    pub fn one() -> Self {
        Self {
            value: 1.0,
            calibration: CalibrationMode::Heuristic,
            source: ConfidenceSource::Unknown,
        }
    }

    pub const fn value(&self) -> f32 {
        self.value
    }

    pub fn is_high(&self) -> bool {
        self.value >= 0.8
    }

    pub fn is_medium(&self) -> bool {
        self.value >= 0.5 && self.value < 0.8
    }

    pub fn is_low(&self) -> bool {
        self.value < 0.5
    }

    /// Returns true if this confidence is calibrated (not heuristic).
    #[must_use]
    pub fn is_calibrated(&self) -> bool {
        !matches!(self.calibration, CalibrationMode::Heuristic)
    }

    /// Validates that a confidence value is in the valid range.
    fn validate_value(value: f32) -> Result<(), ValidationError> {
        if value.is_nan() {
            return Err(ValidationError::ConfidenceOutOfRange { value });
        }
        if !(Self::MIN_VALUE..=Self::MAX_VALUE).contains(&value) {
            return Err(ValidationError::ConfidenceOutOfRange { value });
        }
        Ok(())
    }

    /// Combines two confidences using the minimum (conservative approach).
    /// Optional premise IDs allow provenance tracking for the inputs.
    #[must_use]
    pub fn and(
        &self,
        other: &Self,
        self_id: Option<BeliefId>,
        other_id: Option<BeliefId>,
    ) -> Self {
        let premise_ids: Vec<BeliefId> = [self_id, other_id].into_iter().flatten().collect();
        Self {
            value: self.value.min(other.value),
            calibration: CalibrationMode::Heuristic, // Combined loses calibration
            source: ConfidenceSource::DerivedFromPremises {
                premise_ids,
                propagation_rule: "min".to_string(),
            },
        }
    }

    /// Combines two confidences using the maximum.
    /// Optional premise IDs allow provenance tracking for the inputs.
    #[must_use]
    pub fn or(
        &self,
        other: &Self,
        self_id: Option<BeliefId>,
        other_id: Option<BeliefId>,
    ) -> Self {
        let premise_ids: Vec<BeliefId> = [self_id, other_id].into_iter().flatten().collect();
        Self {
            value: self.value.max(other.value),
            calibration: CalibrationMode::Heuristic,
            source: ConfidenceSource::DerivedFromPremises {
                premise_ids,
                propagation_rule: "max".to_string(),
            },
        }
    }
}

impl Default for Confidence {
    fn default() -> Self {
        Self::zero()
    }
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.2} ({})", self.value, self.calibration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confidence_valid_values() {
        assert!(Confidence::probability(0.0, "test").is_ok());
        assert!(Confidence::probability(0.5, "test").is_ok());
        assert!(Confidence::probability(1.0, "test").is_ok());
    }

    #[test]
    fn test_confidence_invalid_values() {
        assert!(Confidence::probability(-0.1, "test").is_err());
        assert!(Confidence::probability(1.1, "test").is_err());
        assert!(Confidence::probability(f32::NAN, "test").is_err());
    }

    #[test]
    fn test_confidence_value_getter() {
        let conf = Confidence::probability(0.75, "test").unwrap();
        assert!((conf.value() - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn test_confidence_levels() {
        let high = Confidence::probability(0.9, "test").unwrap();
        let medium = Confidence::probability(0.6, "test").unwrap();
        let low = Confidence::probability(0.3, "test").unwrap();

        assert!(high.is_high());
        assert!(!high.is_medium());
        assert!(!high.is_low());

        assert!(!medium.is_high());
        assert!(medium.is_medium());
        assert!(!medium.is_low());

        assert!(!low.is_high());
        assert!(!low.is_medium());
        assert!(low.is_low());
    }

    #[test]
    fn test_confidence_is_calibrated() {
        let calibrated = Confidence::probability(0.8, "test").unwrap();
        let uncalibrated = Confidence::heuristic(0.8, "test").unwrap();

        assert!(calibrated.is_calibrated());
        assert!(!uncalibrated.is_calibrated());
    }

    #[test]
    fn test_confidence_and() {
        let a = Confidence::probability(0.8, "test").unwrap();
        let b = Confidence::probability(0.6, "test").unwrap();
        let combined = a.and(&b, None, None);

        assert!((combined.value() - 0.6).abs() < f32::EPSILON);
    }

    #[test]
    fn test_confidence_or() {
        let a = Confidence::probability(0.8, "test").unwrap();
        let b = Confidence::probability(0.6, "test").unwrap();
        let combined = a.or(&b, None, None);

        assert!((combined.value() - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn test_confidence_and_with_premises() {
        let a = Confidence::probability(0.8, "test").unwrap();
        let b = Confidence::probability(0.6, "test").unwrap();
        let aid = BeliefId::new();
        let bid = BeliefId::new();
        let combined = a.and(&b, Some(aid), Some(bid));

        assert_eq!(combined.value(), 0.6);
        if let ConfidenceSource::DerivedFromPremises { premise_ids, .. } = combined.source {
            assert_eq!(premise_ids, vec![aid, bid]);
        } else {
            panic!("expected DerivedFromPremises");
        }
    }

    #[test]
    fn test_confidence_zero_and_one() {
        let zero = Confidence::zero();
        let one = Confidence::one();

        assert!((zero.value() - 0.0).abs() < f32::EPSILON);
        assert!((one.value() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_confidence_from_model() {
        let conf = Confidence::from_model(0.85, "gpt-4", "2024-01").unwrap();

        assert_eq!(conf.calibration, CalibrationMode::ModelLogprob);
        if let ConfidenceSource::ComputedByModel {
            model_id,
            model_version,
        } = &conf.source
        {
            assert_eq!(model_id, "gpt-4");
            assert_eq!(model_version, "2024-01");
        } else {
            panic!("Expected ComputedByModel source");
        }
    }

    #[test]
    fn test_confidence_display() {
        let conf = Confidence::probability(0.85, "test").unwrap();
        let display = format!("{conf}");
        assert!(display.contains("0.85"));
        assert!(display.contains("probability"));
    }

    #[test]
    fn test_confidence_serialization() {
        let conf = Confidence::probability(0.75, "test-agent").unwrap();
        let json = serde_json::to_string(&conf).unwrap();
        let deserialized: Confidence = serde_json::from_str(&json).unwrap();

        assert!((conf.value() - deserialized.value()).abs() < f32::EPSILON);
        assert_eq!(conf.calibration, deserialized.calibration);
    }

    #[test]
    fn test_belief_id() {
        let id1 = BeliefId::new();
        let id2 = BeliefId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_source_id() {
        let id1 = SourceId::new();
        let id2 = SourceId::new();
        assert_ne!(id1, id2);
    }
}
