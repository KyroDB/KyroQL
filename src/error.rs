//! Error types for KyroQL.
//!
//! All errors in KyroQL are strongly typed using thiserror.
//! This enables pattern matching on specific error conditions
//! and provides clear error messages.

use thiserror::Error;
use chrono::{DateTime, Utc};

use crate::entity::EntityId;
use crate::confidence::BeliefId;

/// Validation errors that occur during input validation.
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("Confidence value {value} is out of range [0.0, 1.0]")]
    ConfidenceOutOfRange {
        value: f32,
    },

    #[error("Invalid time range: from ({from}) must be before to ({to})")]
    InvalidTimeRange {
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    },

    #[error("Entity name cannot be empty")]
    EmptyEntityName,

    #[error("Predicate cannot be empty")]
    EmptyPredicate,

    #[error("Required field '{field}' is missing")]
    MissingField {
        field: String,
    },

    #[error("Field '{field}' exceeds maximum length of {max_length}")]
    FieldTooLong {
        field: String,
        max_length: usize,
    },

    #[error("Embedding has {actual} dimensions, expected {expected}")]
    InvalidEmbeddingDimension {
        actual: usize,
        expected: usize,
    },

    #[error("Invalid pattern rule: {reason}")]
    InvalidPatternRule {
        reason: String,
    },
}

/// Execution errors that occur during operation execution.
#[derive(Debug, Error)]
pub enum ExecutionError {
    #[error("Entity not found: {id}")]
    EntityNotFound {
        id: EntityId,
    },

    #[error("Belief not found: {id}")]
    BeliefNotFound {
        id: BeliefId,
    },

    #[error("Simulation not found: {id}")]
    SimulationNotFound {
        id: String,
    },

    #[error("Simulation limit exceeded: {limit_type} (max: {max_value}, actual: {actual_value})")]
    SimulationLimitExceeded {
        limit_type: String,
        max_value: u64,
        actual_value: u64,
    },

    #[error("Operation timed out after {duration_ms}ms")]
    Timeout {
        duration_ms: u64,
    },

    #[error("Storage error: {message}")]
    Storage {
        message: String,
    },

    #[error("Index error: {message}")]
    Index {
        message: String,
    },

    #[error("Conflict resolution failed: {reason}")]
    ConflictResolutionFailed {
        reason: String,
    },

    #[error("Pattern '{pattern_name}' was violated: {reason}")]
    PatternViolation {
        pattern_name: String,
        reason: String,
    },
}

/// Transport errors for client-server communication.
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("Connection failed: {message}")]
    ConnectionFailed {
        message: String,
    },

    #[error("Failed to serialize request: {message}")]
    SerializationFailed {
        message: String,
    },

    #[error("Failed to deserialize response: {message}")]
    DeserializationFailed {
        message: String,
    },

    #[error("Server error (code {code}): {message}")]
    ServerError {
        code: u32,
        message: String,
    },
}

/// Top-level error type for KyroQL.
///
/// This enum encompasses all possible errors that can occur
/// when using KyroQL.
#[derive(Debug, Error)]
pub enum KyroError {
    #[error("Validation error: {0}")]
    Validation(#[from] ValidationError),

    #[error("Execution error: {0}")]
    Execution(#[from] ExecutionError),

    #[error("Transport error: {0}")]
    Transport(#[from] TransportError),

    #[error("Internal error: {message}")]
    Internal {
        message: String,
    },
}

impl KyroError {
    /// Creates an internal error.
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Returns true if this is a validation error.
    #[must_use]
    pub const fn is_validation(&self) -> bool {
        matches!(self, Self::Validation(_))
    }

    /// Returns true if this is an execution error.
    #[must_use]
    pub const fn is_execution(&self) -> bool {
        matches!(self, Self::Execution(_))
    }

    /// Returns true if this is a transport error.
    #[must_use]
    pub const fn is_transport(&self) -> bool {
        matches!(self, Self::Transport(_))
    }

    /// Returns true if this is an internal error.
    #[must_use]
    pub const fn is_internal(&self) -> bool {
        matches!(self, Self::Internal { .. })
    }

    /// Returns true if this error is retryable.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::Validation(_) => false, // Validation errors won't change on retry
            Self::Execution(e) => matches!(e, ExecutionError::Timeout { .. }),
            Self::Transport(e) => match e {
                TransportError::ConnectionFailed { .. } => true,
                TransportError::ServerError { code, .. } => *code >= 500,
                _ => false,
            },
            Self::Internal { .. } => false,
        }
    }
}

/// Result type alias for KyroQL operations.
pub type KyroResult<T> = Result<T, KyroError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_error_confidence() {
        let err = ValidationError::ConfidenceOutOfRange { value: 1.5 };
        let msg = format!("{err}");
        assert!(msg.contains("1.5"));
        assert!(msg.contains("out of range"));
    }

    #[test]
    fn test_validation_error_time_range() {
        let now = Utc::now();
        let later = now + chrono::Duration::hours(1);
        let err = ValidationError::InvalidTimeRange { from: later, to: now };
        let msg = format!("{err}");
        assert!(msg.contains("Invalid time range"));
    }

    #[test]
    fn test_execution_error_entity_not_found() {
        let id = EntityId::new();
        let err = ExecutionError::EntityNotFound { id };
        let msg = format!("{err}");
        assert!(msg.contains("Entity not found"));
    }

    #[test]
    fn test_execution_error_timeout() {
        let err = ExecutionError::Timeout { duration_ms: 5000 };
        let msg = format!("{err}");
        assert!(msg.contains("5000ms"));
    }

    #[test]
    fn test_execution_error_simulation_limit() {
        let err = ExecutionError::SimulationLimitExceeded {
            limit_type: "entities".to_string(),
            max_value: 1000,
            actual_value: 1500,
        };
        let msg = format!("{err}");
        assert!(msg.contains("entities"));
        assert!(msg.contains("1000"));
        assert!(msg.contains("1500"));
    }

    #[test]
    fn test_transport_error() {
        let err = TransportError::ConnectionFailed {
            message: "refused".to_string(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("Connection failed"));
        assert!(msg.contains("refused"));
    }

    #[test]
    fn test_kyro_error_from_validation() {
        let validation_err = ValidationError::EmptyEntityName;
        let kyro_err: KyroError = validation_err.into();
        assert!(kyro_err.is_validation());
        assert!(!kyro_err.is_retryable());
    }

    #[test]
    fn test_kyro_error_from_execution() {
        let exec_err = ExecutionError::Timeout { duration_ms: 1000 };
        let kyro_err: KyroError = exec_err.into();
        assert!(kyro_err.is_execution());
        assert!(kyro_err.is_retryable());
    }

    #[test]
    fn test_kyro_error_from_transport() {
        let transport_err = TransportError::ConnectionFailed {
            message: "test".to_string(),
        };
        let kyro_err: KyroError = transport_err.into();
        assert!(kyro_err.is_transport());
        assert!(kyro_err.is_retryable());
    }

    #[test]
    fn test_kyro_error_internal() {
        let err = KyroError::internal("unexpected state");
        assert!(err.is_internal());
        assert!(!err.is_retryable());
        let msg = format!("{err}");
        assert!(msg.contains("unexpected state"));
    }

    #[test]
    fn test_kyro_error_retryable() {
        // Not retryable
        let err1: KyroError = ValidationError::EmptyPredicate.into();
        assert!(!err1.is_retryable());

        // Retryable
        let err2: KyroError = ExecutionError::Timeout { duration_ms: 100 }.into();
        assert!(err2.is_retryable());

        let err3: KyroError = TransportError::ConnectionFailed {
            message: "test".to_string(),
        }
        .into();
        assert!(err3.is_retryable());
    }
}
