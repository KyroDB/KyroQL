//! Error types for KyroQL.
//!
//! All errors in KyroQL are strongly typed using thiserror.
//! This enables pattern matching on specific error conditions
//! and provides clear error messages.

use thiserror::Error;
use chrono::{DateTime, Utc};

use crate::confidence::BeliefId;
use crate::conflict::ConflictId;
use crate::entity::EntityId;

/// Validation errors that occur during input validation.
#[derive(Debug, Error)]
pub enum ValidationError {
    /// Confidence value outside [0.0, 1.0].
    #[error("Confidence value {value} is out of range [0.0, 1.0]")]
    ConfidenceOutOfRange {
        /// The invalid value.
        value: f32,
    },

    /// Time range has start >= end.
    #[error("Invalid time range: from ({from}) must be before to ({to})")]
    InvalidTimeRange {
        /// Start time.
        from: DateTime<Utc>,
        /// End time.
        to: DateTime<Utc>,
    },

    /// Entity name is empty.
    #[error("Entity name cannot be empty")]
    EmptyEntityName,

    /// Predicate is empty.
    #[error("Predicate cannot be empty")]
    EmptyPredicate,

    /// Required field missing.
    #[error("Required field '{field}' is missing")]
    MissingField {
        /// Name of missing field.
        field: String,
    },

    /// Field exceeds length limit.
    #[error("Field '{field}' exceeds maximum length of {max_length}")]
    FieldTooLong {
        /// Field name.
        field: String,
        /// Maximum allowed.
        max_length: usize,
    },

    /// Embedding dimension mismatch.
    #[error("Embedding has {actual} dimensions, expected {expected}")]
    InvalidEmbeddingDimension {
        /// Actual dimension.
        actual: usize,
        /// Expected dimension.
        expected: usize,
    },

    /// Pattern rule is invalid.
    #[error("Invalid pattern rule: {reason}")]
    InvalidPatternRule {
        /// Reason for invalidity.
        reason: String,
    },

    /// Conflict resolution policy is invalid.
    #[error("Invalid conflict resolution policy: {reason}")]
    InvalidConflictResolutionPolicy {
        /// Reason for invalidity.
        reason: String,
    },

    /// Simulation constraints are invalid.
    #[error("Invalid simulation constraints: {reason}")]
    InvalidSimulationConstraints {
        /// Reason for invalidity.
        reason: String,
    },

    /// Field is syntactically valid but semantically invalid.
    #[error("Invalid field '{field}': {reason}")]
    InvalidField {
        /// Field name.
        field: String,
        /// Reason the field is invalid.
        reason: String,
    },
}

/// Execution errors that occur during operation execution.
#[derive(Debug, Error)]
pub enum ExecutionError {
    /// Entity not found in storage.
    #[error("Entity not found: {id}")]
    EntityNotFound {
        /// Missing entity ID.
        id: EntityId,
    },

    /// Belief not found in storage.
    #[error("Belief not found: {id}")]
    BeliefNotFound {
        /// Missing belief ID.
        id: BeliefId,
    },

    /// Simulation not found.
    #[error("Simulation not found: {id}")]
    SimulationNotFound {
        /// Missing simulation ID.
        id: String,
    },

    /// Resource limit exceeded during simulation.
    #[error("Simulation limit exceeded: {limit_type} (max: {max_value}, actual: {actual_value})")]
    SimulationLimitExceeded {
        /// Type of limit.
        limit_type: String,
        /// Maximum allowed.
        max_value: u64,
        /// Actual value.
        actual_value: u64,
    },

    /// Commit of a simulation overlay is not allowed.
    #[error("Simulation commit not allowed: {reason}")]
    SimulationCommitNotAllowed {
        /// Reason commit was rejected.
        reason: String,
    },

    /// Operation timed out.
    #[error("Operation timed out after {duration_ms}ms")]
    Timeout {
        /// Duration before timeout.
        duration_ms: u64,
    },

    /// Runtime worker pool disconnected before producing a reply.
    #[error("Runtime worker pool disconnected for {path} path")]
    Disconnected {
        /// Execution path name.
        path: String,
    },

    /// The runtime queue is full.
    #[error("Runtime queue is full for {path} path (capacity={capacity})")]
    QueueFull {
        /// Execution path name.
        path: String,
        /// Queue capacity.
        capacity: usize,
    },

    /// An operation was provided where a different one was required.
    #[error("Invalid operation: expected {expected}, got {actual}")]
    InvalidOperation {
        /// Expected operation name.
        expected: String,
        /// Actual operation name.
        actual: String,
    },

    /// Operation is recognized but not implemented in the current build.
    #[error("Operation not implemented: {operation}")]
    NotImplemented {
        /// Operation name.
        operation: String,
    },

    /// Storage backend error.
    #[error("Storage error: {message}")]
    Storage {
        /// Error details.
        message: String,
    },

    /// Index error.
    #[error("Index error: {message}")]
    Index {
        /// Error details.
        message: String,
    },

    /// Conflict resolution failed.
    #[error("Conflict resolution failed: {reason}")]
    ConflictResolutionFailed {
        /// Reason for failure.
        reason: String,
    },

    /// Conflicts were detected that prevent the operation.
    #[error("Conflicts detected: {conflicts:?}")]
    ConflictsDetected {
        /// Human-readable conflict descriptors.
        conflicts: Vec<String>,
    },

    /// Pattern constraint violated.
    #[error("Pattern '{pattern_name}' was violated: {reason}")]
    PatternViolation {
        /// Pattern name.
        pattern_name: String,
        /// Violation reason.
        reason: String,
    },

    /// Derivation request is invalid.
    #[error("Invalid derivation: {reason}")]
    InvalidDerivation {
        /// Reason the derivation was rejected.
        reason: String,
    },

    /// Simulation commit partially succeeded before failing.
    #[error(
        "Simulation commit partially applied; committed {committed_len} beliefs before failure at {failed_belief_id}: {cause}"
    )]
    SimulationPartialCommit {
        /// Count of committed beliefs.
        committed_len: usize,
        /// Mapping of overlay belief IDs to newly committed base IDs.
        committed: Vec<(BeliefId, BeliefId)>,
        /// Conflicts recorded before failure.
        conflict_ids: Vec<ConflictId>,
        /// Overlay belief ID that failed.
        failed_belief_id: BeliefId,
        /// Underlying error.
        cause: Box<KyroError>,
    },
}

/// Transport errors for client-server communication.
#[derive(Debug, Error)]
pub enum TransportError {
    /// Network connection failed.
    #[error("Connection failed: {message}")]
    ConnectionFailed {
        /// Error details.
        message: String,
    },

    /// Request serialization failed.
    #[error("Failed to serialize request: {message}")]
    SerializationFailed {
        /// Error details.
        message: String,
    },

    /// Response deserialization failed.
    #[error("Failed to deserialize response: {message}")]
    DeserializationFailed {
        /// Error details.
        message: String,
    },

    /// Server returned an error code.
    #[error("Server error (code {code}): {message}")]
    ServerError {
        /// HTTP/RPC status code.
        code: u32,
        /// Error message.
        message: String,
    },
}

/// Top-level error type for KyroQL.
///
/// This enum encompasses all possible errors that can occur
/// when using KyroQL.
#[derive(Debug, Error)]
pub enum KyroError {
    /// Input validation failed.
    #[error("Validation error: {0}")]
    Validation(#[from] ValidationError),

    /// Execution failure.
    #[error("Execution error: {0}")]
    Execution(#[from] ExecutionError),

    /// Communication failure.
    #[error("Transport error: {0}")]
    Transport(#[from] TransportError),

    /// Internal system error.
    #[error("Internal error: {message}")]
    Internal {
        /// Error description.
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
            // Validation errors won't change on retry
            Self::Execution(e) => matches!(e, ExecutionError::Timeout { .. }),
            Self::Transport(e) => match e {
                TransportError::ConnectionFailed { .. } => true,
                TransportError::ServerError { code, .. } => *code >= 500,
                _ => false,
            },
            Self::Validation(_) | Self::Internal { .. } => false,
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
