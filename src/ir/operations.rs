//! KyroQL operation definitions and payloads.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::confidence::{BeliefId, Confidence};
use crate::entity::EntityId;
use crate::pattern::PatternRule;
use crate::source::Source;
use crate::time::TimeRange;
use crate::value::Value;

use super::ConsistencyMode;

/// The top-level IR wrapper for all KyroQL operations.
///
/// Every operation is wrapped in this struct to provide:
/// - Protocol versioning for forward/backward compatibility
/// - Request tracking via unique IDs
/// - Timestamp for audit logs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KyroIR {
    /// Protocol version (e.g., "1.0").
    pub version: String,

    /// Unique identifier for this request (for tracing/debugging).
    pub request_id: Uuid,

    /// When this IR was created.
    pub timestamp: DateTime<Utc>,

    /// The operation to execute.
    pub operation: Operation,
}

impl KyroIR {
    /// Current protocol version.
    pub const CURRENT_VERSION: &'static str = "1.0";

    /// Creates a new IR with the given operation.
    pub fn new(operation: Operation) -> Self {
        Self {
            version: Self::CURRENT_VERSION.to_string(),
            request_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            operation,
        }
    }

    /// Sets a custom request ID (useful for correlation).
    pub fn with_request_id(mut self, request_id: Uuid) -> Self {
        self.request_id = request_id;
        self
    }
}

/// All supported KyroQL operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", content = "payload", rename_all = "snake_case")]
pub enum Operation {
    /// Assert a new belief into the knowledge base.
    Assert(AssertPayload),

    /// Resolve/query beliefs from the knowledge base.
    Resolve(ResolvePayload),

    /// Retract (mark as superseded) an existing belief.
    Retract(RetractPayload),

    /// Define a new pattern/constraint.
    DefinePattern(DefinePatternPayload),
}

/// Payload for ASSERT operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssertPayload {
    /// The entity this belief is about.
    pub entity_id: EntityId,

    /// The attribute/relationship being asserted.
    pub predicate: String,

    /// The value being asserted.
    pub value: Value,

    /// Confidence in this assertion.
    pub confidence: Confidence,

    /// Provenance of this assertion.
    pub source: Source,

    /// When this belief is/was valid in the real world.
    pub valid_time: TimeRange,

    /// How to handle consistency checks.
    #[serde(default)]
    pub consistency_mode: ConsistencyMode,

    /// Optional pre-computed embedding for semantic search.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}

/// Payload for RESOLVE operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvePayload {
    /// Natural language or structured query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,

    /// Filter by specific entity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<EntityId>,

    /// Filter by specific predicate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate: Option<String>,

    /// Query as of a specific point in time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_of: Option<DateTime<Utc>>,

    /// Minimum confidence threshold (0.0 to 1.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_confidence: Option<f32>,

    /// Maximum number of results to return.
    #[serde(default = "default_limit")]
    pub limit: usize,

    /// Whether to include counter-evidence in the response.
    #[serde(default)]
    pub include_counter_evidence: bool,

    /// Whether to include knowledge gaps in the response.
    #[serde(default = "default_true")]
    pub include_gaps: bool,
}

fn default_limit() -> usize {
    10
}

fn default_true() -> bool {
    true
}

impl Default for ResolvePayload {
    fn default() -> Self {
        Self {
            query: None,
            entity_id: None,
            predicate: None,
            as_of: None,
            min_confidence: None,
            limit: default_limit(),
            include_counter_evidence: false,
            include_gaps: true,
        }
    }
}

/// Payload for RETRACT operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetractPayload {
    /// The belief to retract.
    pub belief_id: BeliefId,

    /// Reason for retraction (for audit trail).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Source authorizing the retraction.
    pub authorized_by: Source,
}

/// Payload for DEFINE_PATTERN operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefinePatternPayload {
    /// Human-readable name for the pattern.
    pub name: String,

    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The rule to enforce.
    pub rule: PatternRule,

    /// Confidence in this pattern (how sure are we it's correct?).
    pub confidence: Confidence,

    /// When this pattern is valid.
    pub valid_time: TimeRange,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Source;

    fn sample_assert_payload() -> AssertPayload {
        AssertPayload {
            entity_id: EntityId::new(),
            predicate: "temperature".to_string(),
            value: Value::Float(25.5),
            confidence: Confidence::from_agent(0.9, "test_agent").unwrap(),
            source: Source::Agent {
                agent_id: "test_agent".to_string(),
                agent_type: None,
                model_version: None,
            },
            valid_time: TimeRange::from_now(),
            consistency_mode: ConsistencyMode::Strict,
            embedding: None,
        }
    }

    #[test]
    fn test_kyro_ir_creation() {
        let payload = sample_assert_payload();
        let ir = KyroIR::new(Operation::Assert(payload));

        assert_eq!(ir.version, KyroIR::CURRENT_VERSION);
        assert!(matches!(ir.operation, Operation::Assert(_)));
    }

    #[test]
    fn test_kyro_ir_serialization_roundtrip() {
        let payload = sample_assert_payload();
        let ir = KyroIR::new(Operation::Assert(payload));

        let json = serde_json::to_string_pretty(&ir).unwrap();
        let deserialized: KyroIR = serde_json::from_str(&json).unwrap();

        assert_eq!(ir.version, deserialized.version);
        assert_eq!(ir.request_id, deserialized.request_id);
    }

    #[test]
    fn test_operation_tagging() {
        let payload = sample_assert_payload();
        let op = Operation::Assert(payload);
        let json = serde_json::to_string(&op).unwrap();

        assert!(json.contains("\"op\":\"assert\""));
        assert!(json.contains("\"payload\""));
    }

    #[test]
    fn test_resolve_payload_defaults() {
        let payload = ResolvePayload::default();

        assert_eq!(payload.limit, 10);
        assert!(payload.include_gaps);
        assert!(!payload.include_counter_evidence);
    }

    #[test]
    fn test_resolve_payload_serialization() {
        let payload = ResolvePayload {
            query: Some("What is the temperature?".to_string()),
            entity_id: Some(EntityId::new()),
            predicate: Some("temperature".to_string()),
            as_of: None,
            min_confidence: Some(0.5),
            limit: 5,
            include_counter_evidence: true,
            include_gaps: true,
        };

        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: ResolvePayload = serde_json::from_str(&json).unwrap();

        assert_eq!(payload.query, deserialized.query);
        assert_eq!(payload.min_confidence, deserialized.min_confidence);
    }

    #[test]
    fn test_retract_payload() {
        let payload = RetractPayload {
            belief_id: BeliefId::new(),
            reason: Some("Data was incorrect".to_string()),
            authorized_by: Source::Human {
                user_id: "admin".to_string(),
                role: Some("administrator".to_string()),
            },
        };

        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("belief_id"));
        assert!(json.contains("authorized_by"));
    }

    #[test]
    fn test_define_pattern_payload() {
        let payload = DefinePatternPayload {
            name: "temperature_range".to_string(),
            description: Some("Temperature must be between -50 and 150".to_string()),
            rule: PatternRule::range("temperature", Some(-50.0), Some(150.0)),
            confidence: Confidence::from_agent(0.99, "physics").unwrap(),
            valid_time: TimeRange::from_now(),
        };

        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: DefinePatternPayload = serde_json::from_str(&json).unwrap();

        assert_eq!(payload.name, deserialized.name);
    }
}
