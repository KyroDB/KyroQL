//! ASSERT operation builder.
//!
//! The AssertBuilder provides a fluent, type-safe API for constructing
//! ASSERT operations. It validates all inputs before producing IR.

use crate::confidence::Confidence;
use crate::entity::EntityId;
use crate::error::ValidationError;
use crate::ir::{AssertPayload, ConsistencyMode, KyroIR, Operation};
use crate::source::Source;
use crate::time::TimeRange;
use crate::value::Value;

/// Builder for ASSERT operations.
///
/// # Example
/// ```rust,ignore
/// let ir = AssertBuilder::new()
///     .entity(entity_id)
///     .predicate("temperature")
///     .value(Value::Float(25.5))
///     .confidence(Confidence::from_agent(0.9, "sensor_1")?)
///     .source(Source::sensor_with_type("temp_1", "temperature"))
///     .valid_time(TimeRange::from_now())
///     .build()?;
/// ```
#[derive(Debug, Clone, Default)]
pub struct AssertBuilder {
    entity_id: Option<EntityId>,
    predicate: Option<String>,
    value: Option<Value>,
    confidence: Option<Confidence>,
    source: Option<Source>,
    valid_time: Option<TimeRange>,
    consistency_mode: ConsistencyMode,
    embedding: Option<Vec<f32>>,
}

impl AssertBuilder {
    /// Creates a new builder with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the entity this belief is about (required).
    #[must_use]
    pub fn entity(mut self, id: EntityId) -> Self {
        self.entity_id = Some(id);
        self
    }

    /// Set the predicate/attribute (required).
    #[must_use]
    pub fn predicate(mut self, predicate: impl Into<String>) -> Self {
        self.predicate = Some(predicate.into());
        self
    }

    /// Set the value being asserted (required).
    #[must_use]
    pub fn value(mut self, value: impl Into<Value>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Set the confidence in this assertion (required).
    #[must_use]
    pub fn confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = Some(confidence);
        self
    }

    /// Set the source/provenance of this assertion (required).
    #[must_use]
    pub fn source(mut self, source: Source) -> Self {
        self.source = Some(source);
        self
    }

    /// Set when this belief is valid in the real world (required).
    #[must_use]
    pub fn valid_time(mut self, time: TimeRange) -> Self {
        self.valid_time = Some(time);
        self
    }

    /// Set the consistency mode (default: Strict).
    #[must_use]
    pub fn consistency_mode(mut self, mode: ConsistencyMode) -> Self {
        self.consistency_mode = mode;
        self
    }

    /// Set a pre-computed embedding for semantic search (optional).
    #[must_use]
    pub fn embedding(mut self, embedding: Vec<f32>) -> Self {
        self.embedding = Some(embedding);
        self
    }

    /// Build the ASSERT IR.
    ///
    /// Returns `ValidationError::MissingField` if any required field is not set.
    /// Returns `ValidationError::EmptyPredicate` if predicate is empty or whitespace.
    pub fn build(self) -> Result<KyroIR, ValidationError> {
        let entity_id = self
            .entity_id
            .ok_or_else(|| ValidationError::MissingField {
                field: "entity_id".to_string(),
            })?;

        let predicate = self
            .predicate
            .ok_or_else(|| ValidationError::MissingField {
                field: "predicate".to_string(),
            })?;

        let predicate = predicate.trim().to_string();
        if predicate.is_empty() {
            return Err(ValidationError::EmptyPredicate);
        }

        let value = self.value.ok_or_else(|| ValidationError::MissingField {
            field: "value".to_string(),
        })?;

        let confidence = self
            .confidence
            .ok_or_else(|| ValidationError::MissingField {
                field: "confidence".to_string(),
            })?;

        let source = self.source.ok_or_else(|| ValidationError::MissingField {
            field: "source".to_string(),
        })?;

        let valid_time = self
            .valid_time
            .ok_or_else(|| ValidationError::MissingField {
                field: "valid_time".to_string(),
            })?;

        let payload = AssertPayload {
            entity_id,
            predicate,
            value,
            confidence,
            source,
            valid_time,
            consistency_mode: self.consistency_mode,
            embedding: self.embedding,
        };

        Ok(KyroIR::new(Operation::Assert(payload)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_builder() -> AssertBuilder {
        AssertBuilder::new()
            .entity(EntityId::new())
            .predicate("temperature")
            .value(Value::Float(25.5))
            .confidence(Confidence::from_agent(0.9, "test").unwrap())
            .source(Source::Agent {
                agent_id: "test".to_string(),
                agent_type: None,
                model_version: None,
            })
            .valid_time(TimeRange::from_now())
    }

    #[test]
    fn test_valid_build() {
        let ir = valid_builder().build();
        assert!(ir.is_ok());

        let ir = ir.unwrap();
        assert!(matches!(ir.operation, Operation::Assert(_)));
    }

    #[test]
    fn test_predicate_is_trimmed() {
        let ir = valid_builder().predicate("  temperature  ").build().unwrap();

        match ir.operation {
            Operation::Assert(payload) => {
                assert_eq!(payload.predicate, "temperature");
            }
            _ => panic!("expected assert operation"),
        }
    }

    #[test]
    fn test_missing_entity() {
        let result = AssertBuilder::new()
            .predicate("test")
            .value(Value::Bool(true))
            .confidence(Confidence::from_agent(0.5, "test").unwrap())
            .source(Source::Unknown { description: None })
            .valid_time(TimeRange::from_now())
            .build();

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ValidationError::MissingField { field } if field == "entity_id"));
    }

    #[test]
    fn test_missing_predicate() {
        let result = AssertBuilder::new()
            .entity(EntityId::new())
            .value(Value::Bool(true))
            .confidence(Confidence::from_agent(0.5, "test").unwrap())
            .source(Source::Unknown { description: None })
            .valid_time(TimeRange::from_now())
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_empty_predicate() {
        let result = valid_builder().predicate("").build();

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ValidationError::EmptyPredicate));
    }

    #[test]
    fn test_whitespace_predicate() {
        let result = valid_builder().predicate("   ").build();

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ValidationError::EmptyPredicate));
    }

    #[test]
    fn test_missing_confidence() {
        let result = AssertBuilder::new()
            .entity(EntityId::new())
            .predicate("test")
            .value(Value::Bool(true))
            .source(Source::Unknown { description: None })
            .valid_time(TimeRange::from_now())
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_consistency_mode_default() {
        let ir = valid_builder().build().unwrap();

        if let Operation::Assert(payload) = ir.operation {
            assert_eq!(payload.consistency_mode, ConsistencyMode::Strict);
        } else {
            panic!("Expected Assert operation");
        }
    }

    #[test]
    fn test_consistency_mode_override() {
        let ir = valid_builder()
            .consistency_mode(ConsistencyMode::Eventual)
            .build()
            .unwrap();

        if let Operation::Assert(payload) = ir.operation {
            assert_eq!(payload.consistency_mode, ConsistencyMode::Eventual);
        } else {
            panic!("Expected Assert operation");
        }
    }

    #[test]
    fn test_with_embedding() {
        let embedding = vec![0.1, 0.2, 0.3];
        let ir = valid_builder().embedding(embedding.clone()).build().unwrap();

        if let Operation::Assert(payload) = ir.operation {
            assert_eq!(payload.embedding, Some(embedding));
        } else {
            panic!("Expected Assert operation");
        }
    }
}
