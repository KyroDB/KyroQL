//! KyroQL operation definitions and payloads.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json;
use uuid::Uuid;

use crate::confidence::{BeliefId, Confidence};
use crate::entity::EntityId;
use crate::inference::ConflictResolutionPolicy;
use crate::pattern::{PatternId, PatternRule};
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", content = "payload", rename_all = "snake_case")]
pub enum Operation {
    /// Assert a new belief into the knowledge base.
    Assert(AssertPayload),

    /// Resolve/query beliefs from the knowledge base.
    Resolve(ResolvePayload),

    /// Create a simulation context for counterfactual reasoning.
    Simulate(SimulatePayload),

    /// Register a monitor/trigger subscription.
    Monitor(MonitorPayload),

    /// Record a derived belief chain.
    Derive(DerivePayload),

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
    /// Controls how much work RESOLVE is allowed to do.
    ///
    /// This is used for routing between Reflex (fast, bounded) and Reflection
    /// (slow, deliberative) execution paths.
    #[serde(default)]
    pub mode: ResolveMode,

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

    /// Policy for resolving conflicts when multiple competing beliefs exist.
    ///
    /// If not provided, the engine uses its default policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_policy: Option<ConflictResolutionPolicy>,

    /// Optional vector embedding for the query (semantic RESOLVE path).
    ///
    /// If omitted and `query` is present, the engine may fall back to lexical matching.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_embedding: Option<Vec<f32>>,
}

/// Routing hint for RESOLVE.
///
/// - `Simple` is intended for Reflex execution (fast, bounded work).
/// - `Aggregate` and `Temporal` are intended for Reflection execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResolveMode {
    /// Fast, bounded RESOLVE.
    #[default]
    Simple,

    /// Synthesis RESOLVE (evidence/counter-evidence, richer frame).
    Aggregate,

    /// Temporal RESOLVE (as-of, diffs, trajectories).
    Temporal,
}

// NOTE: IR equality is used primarily for tests/roundtrips/debug assertions.
// We intentionally avoid bitwise/IEEE exact float equality here because:
// - `NaN != NaN` breaks reflexivity for `PartialEq`
// - small serialization/compute variations are expected
// Chosen tolerance: 1e-6 (abs + rel), which is strict enough for confidence/embeddings
// without being brittle.
const F32_ABS_EPS: f32 = 1.0e-6;
const F32_REL_EPS: f32 = 1.0e-6;

#[inline]
fn f32_approx_eq(a: f32, b: f32) -> bool {
    if a == b {
        return true;
    }
    if a.is_nan() && b.is_nan() {
        return true;
    }
    if !a.is_finite() || !b.is_finite() {
        return false;
    }
    let diff = (a - b).abs();
    diff <= F32_ABS_EPS || diff <= F32_REL_EPS * a.abs().max(b.abs())
}

#[inline]
fn opt_f32_approx_eq(a: &Option<f32>, b: &Option<f32>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => f32_approx_eq(*a, *b),
        _ => false,
    }
}

#[inline]
fn slice_f32_approx_eq(a: &[f32], b: &[f32]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(a, b)| f32_approx_eq(*a, *b))
}

#[inline]
fn opt_vec_f32_approx_eq(a: &Option<Vec<f32>>, b: &Option<Vec<f32>>) -> bool {
    match (a.as_deref(), b.as_deref()) {
        (None, None) => true,
        (Some(a), Some(b)) => slice_f32_approx_eq(a, b),
        _ => false,
    }
}

impl PartialEq for AssertPayload {
    fn eq(&self, other: &Self) -> bool {
        self.entity_id == other.entity_id
            && self.predicate == other.predicate
            && self.value == other.value
            && self.confidence == other.confidence
            && self.source == other.source
            && self.valid_time == other.valid_time
            && self.consistency_mode == other.consistency_mode
            && opt_vec_f32_approx_eq(&self.embedding, &other.embedding)
    }
}

impl PartialEq for ResolvePayload {
    fn eq(&self, other: &Self) -> bool {
        self.mode == other.mode
            && self.query == other.query
            && self.entity_id == other.entity_id
            && self.predicate == other.predicate
            && self.as_of == other.as_of
            && opt_f32_approx_eq(&self.min_confidence, &other.min_confidence)
            && self.limit == other.limit
            && self.include_counter_evidence == other.include_counter_evidence
            && self.include_gaps == other.include_gaps
            && self.conflict_policy == other.conflict_policy
            && opt_vec_f32_approx_eq(&self.query_embedding, &other.query_embedding)
    }
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
            mode: ResolveMode::Simple,
            query: None,
            entity_id: None,
            predicate: None,
            as_of: None,
            min_confidence: None,
            limit: default_limit(),
            include_counter_evidence: false,
            include_gaps: true,
            conflict_policy: None,
            query_embedding: None,
        }
    }
}

/// Payload for SIMULATE operations.
///
/// Simulation request payload.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct SimulatePayload {
    /// Optional scenario description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scenario: Option<String>,

    /// Optional context object (structured state).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,

    /// Optional list of entities participating in the simulation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entities: Option<Vec<EntityId>>,

    /// Optional initial conditions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_conditions: Option<Value>,

    /// Optional constraints or limits for the simulation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<Value>,

    /// Optional time range for the simulation horizon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_horizon: Option<TimeRange>,

    /// Optional outcome parameters to request specific projections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome_parameters: Option<Value>,
}

/// Payload for MONITOR operations.
///
/// Monitoring request payload.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct MonitorPayload {
    /// Optional human-readable description of what to monitor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Optional predicates to watch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicates: Option<Vec<String>>,

    /// Optional entity filters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_filter: Option<Vec<EntityId>>,

    /// Optional pattern filters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern_filter: Option<Vec<PatternId>>,

    /// Optional threshold or trigger condition payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<Value>,

    /// Optional expiration time for the monitor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,

    /// Optional callback or notification configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback: Option<Value>,
}

/// Payload for DERIVE operations.
///
/// Derivation request payload.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DerivePayload {
    /// Optional derivation rule identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,

    /// Optional derived belief to attach this derivation record to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_belief_id: Option<BeliefId>,

    /// Optional source belief identifiers that feed the derivation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<BeliefId>>,

    /// Optional list of inference steps or rules applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inference_steps: Option<Vec<String>>,

    /// Optional propagated confidence for the derived result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,

    /// Optional justification/explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,

    /// Optional extensible metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl PartialEq for DerivePayload {
    fn eq(&self, other: &Self) -> bool {
        self.rule == other.rule
            && self.derived_belief_id == other.derived_belief_id
            && self.sources == other.sources
            && self.inference_steps == other.inference_steps
            && opt_f32_approx_eq(&self.confidence, &other.confidence)
            && self.justification == other.justification
            && self.metadata == other.metadata
    }
}

/// Payload for RETRACT operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
            mode: ResolveMode::Simple,
            query: Some("What is the temperature?".to_string()),
            entity_id: Some(EntityId::new()),
            predicate: Some("temperature".to_string()),
            query_embedding: None,
            as_of: None,
            min_confidence: Some(0.5),
            limit: 5,
            include_counter_evidence: true,
            include_gaps: true,
            conflict_policy: None,
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
