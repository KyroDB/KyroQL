//! IR validation.
//!
//! This module performs validation on deserialized IR payloads.
//! Builders already validate inputs, but IR validation is required to
//! defend server/embedded execution against malformed or adversarial JSON.

use crate::error::ValidationError;
use crate::ir::operations::{
    AssertPayload, DefinePatternPayload, DerivePayload, MonitorPayload, Operation, ResolvePayload,
    RetractPayload, SimulatePayload,
};

/// Conservative upper bound for embedding vector sizes.
///
/// This is a safety limit to prevent memory/CPU abuse via unbounded vectors.
pub const MAX_EMBEDDING_DIM: usize = 8192;

/// Conservative upper bound for free-form text fields.
pub const MAX_TEXT_LEN: usize = 16 * 1024;

/// Validate a non-empty trimmed string field.
fn validate_non_empty(field: &'static str, value: &str) -> Result<(), ValidationError> {
    let v = value.trim();
    if v.is_empty() {
        return Err(ValidationError::MissingField {
            field: field.to_string(),
        });
    }
    if v.len() > MAX_TEXT_LEN {
        return Err(ValidationError::FieldTooLong {
            field: field.to_string(),
            max_length: MAX_TEXT_LEN,
        });
    }
    Ok(())
}

fn validate_optional_text(field: &'static str, value: &Option<String>) -> Result<(), ValidationError> {
    if let Some(v) = value {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            return Err(ValidationError::MissingField {
                field: field.to_string(),
            });
        }
        if trimmed.len() > MAX_TEXT_LEN {
            return Err(ValidationError::FieldTooLong {
                field: field.to_string(),
                max_length: MAX_TEXT_LEN,
            });
        }
    }
    Ok(())
}

fn validate_embedding(field: &'static str, embedding: &Option<Vec<f32>>) -> Result<(), ValidationError> {
    let Some(v) = embedding else { return Ok(()); };
    if v.is_empty() {
        return Err(ValidationError::InvalidEmbeddingDimension {
            actual: 0,
            expected: 1,
        });
    }
    if v.len() > MAX_EMBEDDING_DIM {
        return Err(ValidationError::FieldTooLong {
            field: field.to_string(),
            max_length: MAX_EMBEDDING_DIM,
        });
    }
    Ok(())
}

fn validate_confidence_range(opt: &Option<f32>) -> Result<(), ValidationError> {
    if let Some(v) = opt {
        if !(0.0..=1.0).contains(v) {
            return Err(ValidationError::ConfidenceOutOfRange { value: *v });
        }
    }
    Ok(())
}

impl AssertPayload {
    /// Validates this payload.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_non_empty("predicate", &self.predicate)?;
        validate_embedding("embedding", &self.embedding)?;
        Ok(())
    }
}

impl ResolvePayload {
    /// Validates this payload.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_optional_text("query", &self.query)?;
        if let Some(p) = &self.predicate {
            validate_non_empty("predicate", p)?;
        }
        validate_confidence_range(&self.min_confidence)?;
        validate_embedding("query_embedding", &self.query_embedding)?;
        Ok(())
    }
}

impl RetractPayload {
    /// Validates this payload.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_optional_text("reason", &self.reason)?;
        Ok(())
    }
}

impl DefinePatternPayload {
    /// Validates this payload.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_non_empty("name", &self.name)?;
        validate_optional_text("description", &self.description)?;
        Ok(())
    }
}

impl SimulatePayload {
    /// Validates this payload.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_optional_text("scenario", &self.scenario)?;
        Ok(())
    }
}

impl MonitorPayload {
    /// Validates this payload.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_optional_text("description", &self.description)?;
        Ok(())
    }
}

impl DerivePayload {
    /// Validates this payload.
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_optional_text("rule", &self.rule)?;
        Ok(())
    }
}

impl Operation {
    /// Validate the operation payload.
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Assert(p) => p.validate(),
            Self::Resolve(p) => p.validate(),
            Self::Retract(p) => p.validate(),
            Self::DefinePattern(p) => p.validate(),
            Self::Simulate(p) => p.validate(),
            Self::Monitor(p) => p.validate(),
            Self::Derive(p) => p.validate(),
        }
    }
}
