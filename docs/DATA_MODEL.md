# KyroQL Data Model Specification

**Version**: 1.0

---

## Overview

This document provides the complete type definitions for KyroQL's data model. These types form the foundation of the cognitive protocol and must be implemented exactly as specified.

---

## 1. Identity Types

### 1.1 EntityId

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Globally unique, stable entity identifier.
/// Once created, an EntityId never changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(Uuid);

impl EntityId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for EntityId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
```

### 1.2 BeliefId

```rust
/// Unique identifier for a belief.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BeliefId(Uuid);

impl BeliefId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
```

### 1.3 Other IDs

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PatternId(Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConflictId(Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TriggerId(Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SimulationId(Uuid);
```

---

## 2. Entity

```rust
use chrono::{DateTime, Utc};

/// Classification of entity types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum EntityType {
    Person,
    Organization,
    Concept,
    Event,
    Location,
    Artifact,       // Code, documents, physical objects
    Hypothesis,     // Scientific hypotheses
    Custom(String), // Custom types (catch-all)
}

impl TryFrom<String> for EntityType {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let value = value.trim();
        if value.is_empty() {
            return Err("entity type cannot be empty".to_string());
        }

        // Custom values must be explicitly prefixed to avoid collisions with built-in names.
        if let Some(rest) = value.strip_prefix("custom:") {
            let rest = rest.trim();
            if rest.is_empty() {
                return Err("custom entity type cannot be empty".to_string());
            }
            return Ok(Self::Custom(rest.to_string()));
        }

        Ok(match value {
            "person" => Self::Person,
            "organization" => Self::Organization,
            "concept" => Self::Concept,
            "event" => Self::Event,
            "location" => Self::Location,
            "artifact" => Self::Artifact,
            "hypothesis" => Self::Hypothesis,
            other => Self::Custom(other.to_string()),
        })
    }
}

impl From<EntityType> for String {
    fn from(value: EntityType) -> Self {
        match value {
            EntityType::Person => "person".to_string(),
            EntityType::Organization => "organization".to_string(),
            EntityType::Concept => "concept".to_string(),
            EntityType::Event => "event".to_string(),
            EntityType::Location => "location".to_string(),
            EntityType::Artifact => "artifact".to_string(),
            EntityType::Hypothesis => "hypothesis".to_string(),
            EntityType::Custom(name) => format!("custom:{name}"),
        }
    }
}

/// The anchor of identity in KyroQL.
/// All beliefs attach to entities via EntityId.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// Stable, globally unique identifier
    pub id: EntityId,

    /// Primary name for this entity
    pub canonical_name: String,

    /// Alternative names/spellings
    pub aliases: Vec<String>,

    /// Type classification
    pub entity_type: EntityType,

    /// When this entity was first created
    pub created_at: DateTime<Utc>,

    /// When this entity was last modified
    pub updated_at: DateTime<Utc>,

    /// Optional embedding for semantic matching
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,

    /// Version number, incremented on each update
    pub version: u64,

    /// Arbitrary metadata
    #[serde(default)]
    pub metadata: serde_json::Value,
}
```

---

## 3. Confidence

```rust
/// How to interpret the confidence value.
/// This is critical: confidence without calibration is meaningless.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationMode {
    /// Calibrated probability.
    /// A value of 0.8 means that historically, ~80% of claims
    /// with this confidence level have been verified as true.
    Probability,

    /// Uncalibrated heuristic score.
    /// Use only for legacy/compatibility. Prefer Probability.
    Heuristic,

    /// Derived from model log-probability.
    /// Maps model confidence to a score.
    ModelLogprob,

    /// Weighted average of source reliabilities.
    /// Computed from the trust scores of contributing sources.
    SourceWeighted,
}

/// Who or what assigned this confidence value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConfidenceSource {
    /// Asserted by an agent
    AssertedByAgent {
        agent_id: String,
    },

    /// Asserted by a human
    AssertedByHuman {
        user_id: String,
    },

    /// Asserted by a sensor
    AssertedBySensor {
        sensor_id: String,
    },

    /// Computed by a model
    ComputedByModel {
        model_id: String,
        model_version: String,
    },

    /// Aggregated from multiple sources
    AggregatedFromSources {
        source_ids: Vec<SourceId>,
        aggregation_method: String,
    },

    /// Derived from premise beliefs
    DerivedFromPremises {
        premise_ids: Vec<BeliefId>,
        propagation_rule: String,
    },

    /// Unknown or unspecified source
    Unknown,
}

/// Formalized uncertainty.
/// Confidence values must always have calibration and provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Confidence {
    /// The confidence value (0.0 to 1.0)
    value: f32,

    /// How to interpret this value
    pub calibration: CalibrationMode,

    /// Who/what assigned this confidence
    pub source: ConfidenceSource,
}

impl Confidence {
    /// Create a new confidence with validation.
    pub fn new(
        value: f32,
        calibration: CalibrationMode,
        source: ConfidenceSource,
    ) -> Result<Self, ValidationError> {
        if value < 0.0 || value > 1.0 {
            return Err(ValidationError::ConfidenceOutOfRange { value });
        }
        Ok(Self { value, calibration, source })
    }

    /// Create a calibrated probability confidence.
    pub fn probability(value: f32, source: ConfidenceSource) -> Result<Self, ValidationError> {
        Self::new(value, CalibrationMode::Probability, source)
    }

    /// Create a heuristic confidence (use sparingly).
    pub fn heuristic(value: f32, source: ConfidenceSource) -> Result<Self, ValidationError> {
        Self::new(value, CalibrationMode::Heuristic, source)
    }

    /// Get the confidence value.
    pub fn value(&self) -> f32 {
        self.value
    }
}
```

---

## 4. Source (Provenance)

```rust
/// Source of a belief - where did this information come from?
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Source {
    /// Academic paper
    Paper {
        arxiv_id: Option<String>,
        doi: Option<String>,
        title: Option<String>,
        authors: Vec<String>,
    },

    /// Sensor or measurement device
    Sensor {
        sensor_id: String,
        sensor_type: Option<String>,
        calibration_date: Option<DateTime<Utc>>,
    },

    /// AI agent
    Agent {
        agent_id: String,
        agent_type: Option<String>,
        model_version: Option<String>,
    },

    /// Human user
    Human {
        user_id: String,
        role: Option<String>,
    },

    /// External API or service
    Api {
        service_name: String,
        endpoint: Option<String>,
        version: Option<String>,
    },

    /// Derived from other beliefs
    Derived {
        premise_ids: Vec<BeliefId>,
        derivation_rule: String,
    },

    /// Unknown or unspecified source
    Unknown {
        description: Option<String>,
    },
}

impl Source {
    /// Get a unique identifier for this source.
    pub fn source_id(&self) -> SourceId {
        // Deterministic, content-addressed identifier
        // 1. Serialize to a stable byte encoding (see `stable_encoding_of`)
        // 2. Hash with BLAKE3
        // 3. Map to a UUIDv5 in a fixed namespace
        const SOURCE_ID_NAMESPACE: Uuid = Uuid::from_bytes([0x5b, 0x1f, 0x67, 0x5e, 0x6d, 0x9f, 0x4b, 0x77, 0x8e, 0xf4, 0x4f, 0x8b, 0x9b, 0x2a, 0xa1, 0x1c]);

        let encoding = stable_encoding_of(self);
        let digest = blake3::hash(&encoding);
        let uuid = Uuid::new_v5(&SOURCE_ID_NAMESPACE, digest.as_bytes());
        SourceId::from_uuid(uuid)
    }
}

/// Deterministic, canonical encoding for a Source.
/// - Fixed discriminant ordering (paper, sensor, agent, human, api, derived, unknown)
/// - Options encoded with explicit presence markers (1 byte)
/// - Collections treated as sets are sorted lexicographically before encoding
/// - Timestamps encoded as seconds + nanoseconds (little-endian) to avoid locale/format drift
/// - No non-deterministic metadata or map iteration
pub fn stable_encoding_of(source: &Source) -> Vec<u8> { /* ... */ }

### Source stable encoding (compatibility contract)

- **Namespace immutability pledge**: The namespace UUID `5b1f675e-6d9f-4b77-8ef4-4f8b9b2aa11c` is permanent and will not be rotated.

#### Primitive Type Encoding

All integer encodings use **little-endian** byte order consistently throughout. There are no exceptions.

| Type | Encoding | Size | Notes |
|------|----------|------|-------|
| **String** | `u32` length (LE) + UTF-8 bytes | 4 + N bytes | No null-termination. Max length: 2³²−1 bytes. |
| **Timestamp** | `i64` seconds (LE) + `u32` nanos (LE) | 12 bytes | Seconds since Unix epoch. Nanos: 0–999,999,999. |
| **UUID/BeliefId** | String representation encoded as above | 4 + 36 bytes | Hyphenated lowercase format (e.g., `550e8400-e29b-41d4-a716-446655440000`). |
| **Vec length** | `u32` count (LE) | 4 bytes | Number of elements, followed by each element. |

#### Presence-Flag Semantics

For every **optional field**, emit a single-byte presence flag:

| Flag | Meaning | Payload |
|------|---------|---------|
| `0x00` | Absent | Only the flag is emitted; no payload follows. |
| `0x01` | Present | Flag followed immediately by the field's encoded payload. |

**Empty-but-present values**: A present empty value (e.g., empty string, empty collection) is encoded as `0x01` followed by the canonical encoding for that empty value:
- Empty string: `0x01` + `[0x00, 0x00, 0x00, 0x00]` (length = 0, no bytes)
- Empty sorted set: `0x01` + `[0x00, 0x00, 0x00, 0x00]` (count = 0)

#### Canonicalization Rules

- **Discriminant/tag encoding**: The encoding begins with a version prefix string (`"kyroql:source:v1"` as length-prefixed UTF-8), followed by a variant tag string (e.g., `"paper"`, `"sensor"`), then variant-specific fields.
    - **Note**: The 1-byte numeric discriminant (`0=paper`, etc.) described in earlier versions is superseded by the string-based variant tag for clarity. The canonical implementation uses string tags.
- **Field ordering**: Within each variant, fields MUST be emitted in the **exact declaration order** as specified below.
- **Sorted sets**: Collections treated as sets (`authors`, `premise_ids`) are sorted before encoding using **binary comparison of UTF-8 byte sequences** (not locale-aware collation). For `BeliefId`, sort by the string representation.
- **No map iteration**: Every field position is fixed and must be emitted (via presence marker for optional fields).

#### Per-Variant Field Ordering

Each Source variant MUST emit fields in the exact order listed. Fields marked with `?` are optional (emit presence flag). Fields marked with `[sorted]` are sorted sets.

| Variant | Field Order | Notes |
|---------|-------------|-------|
| **Paper** | `arxiv_id?`, `doi?`, `title?`, `authors[sorted]` | All string identifiers optional; authors is a sorted set. |
| **Sensor** | `sensor_id`, `sensor_type?`, `calibration_date?` | `sensor_id` is required. Timestamp for calibration. |
| **Agent** | `agent_id`, `agent_type?`, `model_version?` | `agent_id` is required. |
| **Human** | `user_id`, `role?` | `user_id` is required. |
| **Api** | `service_name`, `endpoint?`, `version?` | `service_name` is required. |
| **Derived** | `premise_ids[sorted]`, `derivation_rule` | `premise_ids` sorted by UUID string. Both required. |
| **Unknown** | `description?` | Single optional string. |

#### Sorting Rules

For collections that are **sorted sets**, elements MUST be sorted using **stable binary comparison** of their encoded byte representations:

- **Strings**: Sorted lexicographically by UTF-8 byte sequence (i.e., `memcmp`-style comparison). Not locale-aware.
- **UUIDs / BeliefIds**: Sorted by their hyphenated lowercase string representation, then by UTF-8 byte sequence.
- **Stability**: All sorts MUST produce identical ordering across implementations. Duplicate elements are invalid in sets.

#### Hashing and ID Derivation

```

blake3( stable_encoding ) → Uuid::new_v5(SOURCE_ID_NAMESPACE, digest) → SourceId

```

- **Cross-language determinism**: Any implementation (Rust, Python, Go) MUST reproduce the exact byte layout above; otherwise SourceIds will diverge.

#### Migration/Versioning Guidance

- If upgrading from a pre-canonical implementation, recompute SourceIds by re-encoding all sources with these rules and rebuild any indexes keyed by SourceId. Mixed old/new IDs are invalid.
- If a bug is found in the canonical encoding or SourceId derivation, the approved strategy is **versioned SourceIds**: introduce a new namespace UUID for the corrected scheme, keep the original namespace and IDs forever, and provide an explicit migration path that can carry both IDs during transition.

#### Verification / Locked Reference

The official namespace UUID is recorded in this spec header (**Version: 1.0**) and in the code constant `SOURCE_ID_NAMESPACE` in `src/belief/source.rs`; these are the normative references for confirming it is locked.
```

---

## 5. Value

```rust
/// Possible values a belief can hold.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    /// Boolean value
    Bool(bool),

    /// 64-bit signed integer
    Int(i64),

    /// 64-bit floating point
    Float(f64),

    /// UTF-8 string
    String(String),

    /// Reference to another entity
    Entity(EntityId),

    /// Embedding vector
    Embedding(Vec<f32>),

    /// Structured JSON data
    Structured(serde_json::Value),

    /// Null/missing value (use sparingly)
    Null,
}

impl From<bool> for Value {
    fn from(v: bool) -> Self { Value::Bool(v) }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self { Value::Int(v) }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self { Value::Float(v) }
}

impl From<String> for Value {
    fn from(v: String) -> Self { Value::String(v) }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self { Value::String(v.to_string()) }
}

impl From<EntityId> for Value {
    fn from(v: EntityId) -> Self { Value::Entity(v) }
}
```

---

## 6. Temporal Types

```rust
/// A range of time (half-open interval: [from, to)).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    /// Start of the range (inclusive)
    pub from: DateTime<Utc>,

    /// End of the range (exclusive)
    /// None means "until further notice" (open-ended)
    pub to: Option<DateTime<Utc>>,
}

impl TimeRange {
    /// Create a time range from two timestamps.
    pub fn new(from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Self, ValidationError> {
        if from >= to {
            return Err(ValidationError::InvalidTimeRange { from, to });
        }
        Ok(Self { from, to: Some(to) })
    }

    /// Create an open-ended time range starting now.
    pub fn from_now() -> Self {
        Self { from: Utc::now(), to: None }
    }

    /// Create a time range starting now with a duration.
    pub fn from_now_for(duration: chrono::Duration) -> Self {
        let from = Utc::now();
        Self { from, to: Some(from + duration) }
    }

    /// Check if a timestamp falls within this range.
    pub fn contains(&self, time: DateTime<Utc>) -> bool {
        time >= self.from && self.to.map_or(true, |to| time < to)
    }

    /// Check if this range overlaps with another.
    pub fn overlaps(&self, other: &TimeRange) -> bool {
        let self_end = self.to.unwrap_or(DateTime::<Utc>::MAX_UTC);
        let other_end = other.to.unwrap_or(DateTime::<Utc>::MAX_UTC);
        self.from < other_end && other.from < self_end
    }
}
```

---

## 7. Belief

```rust
/// Consistency status of a belief.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ConsistencyStatus {
    /// Checked against patterns, passed
    Verified,

    /// Accepted but unchecked
    Provisional,

    /// Conflicts with other beliefs or patterns
    Contested {
        conflict_ids: Vec<ConflictId>,
    },
}

/// The atomic unit of knowledge in KyroQL.
/// A belief represents a single claim about reality with full metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Belief {
    /// Unique identifier for this belief
    pub id: BeliefId,

    /// The entity this belief is about
    pub subject: EntityId,

    /// The attribute or relationship being asserted
    pub predicate: String,

    /// The value being asserted
    pub value: Value,

    // --- Epistemic Metadata ---

    /// How confident are we in this belief?
    pub confidence: Confidence,

    /// Where did this belief come from?
    pub source: Source,

    // --- Temporal Dimensions (Bitemporal) ---

    /// When is this belief valid in reality?
    /// (Valid Time / Application Time)
    pub valid_time: TimeRange,

    /// When did the system learn this belief?
    /// (Transaction Time / System Time)
    pub tx_time: DateTime<Utc>,

    // --- Status ---

    /// Consistency status
    pub consistency_status: ConsistencyStatus,

    /// If this belief supersedes another
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<BeliefId>,

    /// If this belief was superseded by another
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<BeliefId>,

    // --- Embedding (Optional) ---

    /// Embedding for semantic retrieval
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}
```

---

## 8. Conflict Types

```rust
/// Types of conflicts that can exist between beliefs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConflictType {
    /// Same (subject, predicate) has different values
    ValueContradiction {
        belief_a: BeliefId,
        belief_b: BeliefId,
        value_a: Value,
        value_b: Value,
    },

    /// Beliefs claim to be valid at overlapping times but are incompatible
    TemporalInconsistency {
        belief_a: BeliefId,
        belief_b: BeliefId,
        overlap: TimeRange,
    },

    /// Different sources disagree about the same claim
    SourceDisagreement {
        source_a: SourceId,
        source_b: SourceId,
        beliefs: Vec<BeliefId>,
    },

    /// Belief violates a stored pattern/invariant
    PatternViolation {
        belief: BeliefId,
        pattern: PatternId,
        pattern_name: String,
        violation_description: String,
    },
}

/// Status of a conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStatus {
    /// Conflict is open and unresolved
    Open,

    /// Conflict has been resolved
    Resolved,

    /// Conflict is acknowledged but suppressed
    Suppressed,
}

/// A detected conflict between beliefs or with patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub id: ConflictId,
    pub conflict_type: ConflictType,
    pub detected_at: DateTime<Utc>,
    pub status: ConflictStatus,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<ConflictResolution>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution_at: Option<DateTime<Utc>>,
}

/// How a conflict was resolved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictResolution {
    pub method: ConflictResolutionPolicy,
    pub winning_belief: Option<BeliefId>,
    pub resolved_by: String, // agent_id or "system"
}
```

---

## 9. Pattern

```rust
/// Direction for monotonic patterns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Increasing,
    Decreasing,
}

/// Rules that beliefs must satisfy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PatternRule {
    // --- Numeric Constraints ---

    GreaterThan {
        attribute: String,
        value: f64,
    },

    GreaterThanOrEqual {
        attribute: String,
        value: f64,
    },

    LessThan {
        attribute: String,
        value: f64,
    },

    LessThanOrEqual {
        attribute: String,
        value: f64,
    },

    Between {
        attribute: String,
        min: f64,
        max: f64,
        inclusive: bool,
    },

    NonNegative {
        attribute: String,
    },

    // --- Uniqueness Constraints ---

    Unique {
        entity_type: EntityType,
        attribute: String,
    },

    // --- Cardinality Constraints ---

    Cardinality {
        predicate: String,
        min: usize,
        max: usize,
    },

    // --- Temporal Constraints ---

    Monotonic {
        attribute: String,
        direction: Direction,
    },

    // --- Custom Logic (Future) ---

    Custom {
        expression: String,
        language: String, // "datalog", "prolog", etc.
    },
}

/// An invariant or constraint that beliefs must satisfy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub id: PatternId,
    pub name: String,
    pub description: Option<String>,
    pub domain: String,
    pub rule: PatternRule,

    /// How confident are we in this pattern?
    /// Patterns can be uncertain too.
    pub confidence: f32,

    /// When is this pattern valid?
    pub valid_time: TimeRange,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

---

## 10. Conflict Resolution Policies

```rust
/// Policies for resolving conflicts between beliefs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "policy", rename_all = "snake_case")]
pub enum ConflictResolutionPolicy {
    /// Newest claim wins (by tx_time)
    LatestWins,

    /// Highest confidence wins
    HighestConfidence,

    /// Trust hierarchy of sources
    SourcePriority {
        priority: Vec<SourceId>,
    },

    /// Weighted Bayesian combination
    BayesianMerge {
        prior: f32,
    },

    /// Don't resolve - return conflict set for agent to decide
    ExplicitConflict,
}
```

---

## 11. BeliefFrame (Response Type)

```rust
/// A ranked claim with separate confidence and relevance scores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedClaim {
    pub belief: Belief,

    /// Epistemic confidence: Is this claim true? (0.0 - 1.0)
    pub epistemic_confidence: f32,

    /// Retrieval relevance: Is this relevant to the query? (0.0 - 1.0)
    pub retrieval_relevance: f32,

    /// Combined score (weighted combination)
    pub combined_score: f32,
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

/// Assumptions made during query execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryAssumptions {
    pub conflict_policy: ConflictResolutionPolicy,
    pub min_confidence: Option<f32>,
    pub trust_model: String,
    pub as_of_time: DateTime<Utc>,
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
```

---

## 12. Error Types

```rust
/// Validation errors for KyroQL operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ValidationError {
    #[error("Confidence value {value} out of range [0.0, 1.0]")]
    ConfidenceOutOfRange { value: f32 },

    #[error("Invalid time range: from ({from}) must be before to ({to})")]
    InvalidTimeRange { from: DateTime<Utc>, to: DateTime<Utc> },

    #[error("Missing required field: {field}")]
    MissingField { field: String },

    #[error("Entity not found: {id}")]
    EntityNotFound { id: EntityId },

    #[error("Belief not found: {id}")]
    BeliefNotFound { id: BeliefId },

    #[error("Pattern violation: {pattern_name} - {description}")]
    PatternViolation { pattern_name: String, description: String },
}

/// Execution errors for KyroQL operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ExecutionError {
    #[error("Simulation limit exceeded: {limit_type}")]
    SimulationLimitExceeded { limit_type: String },

    #[error("Simulation timeout after {duration_ms}ms")]
    SimulationTimeout { duration_ms: u64 },

    #[error("Storage error: {message}")]
    StorageError { message: String },

    #[error("Connection error: {message}")]
    ConnectionError { message: String },
}

/// Top-level error type for KyroQL.
#[derive(Debug, Clone, thiserror::Error)]
pub enum KyroError {
    #[error(transparent)]
    Validation(#[from] ValidationError),

    #[error(transparent)]
    Execution(#[from] ExecutionError),
}
```

---

## 13. IR (Intermediate Representation)

```rust
/// The serializable representation of all KyroQL operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KyroIR {
    /// IR format version
    pub version: String,

    /// Unique request identifier
    pub request_id: Uuid,

    /// When this IR was generated
    pub timestamp: DateTime<Utc>,

    /// The operation
    pub operation: Operation,
}

/// All supported KyroQL operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssertPayload {
    pub entity_id: EntityId,
    pub predicate: String,
    pub value: Value,
    pub confidence: Confidence,
    pub source: Source,
    pub valid_time: TimeRange,
    #[serde(default)]
    pub consistency_mode: ConsistencyMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConsistencyMode {
    /// Check patterns and fail if conflicts are detected.
    /// Safest mode: no inconsistent data enters the system.
    #[default]
    Strict,

    /// Accept the belief immediately, check patterns asynchronously.
    /// Conflicts are recorded but do not block the write.
    Eventual,

    /// Override existing conflicts. Use with extreme caution.
    /// Intended for administrative corrections.
    Force,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvePayload {
    /// Controls how much work RESOLVE is allowed to do (routing hint).
    #[serde(default)]
    pub mode: ResolveMode,

    /// Natural language query.
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
    pub limit: usize,

    /// Whether to include counter-evidence in the response.
    pub include_counter_evidence: bool,

    /// Whether to include knowledge gaps in the response.
    pub include_gaps: bool,

    /// Policy for resolving conflicts when multiple competing beliefs exist.
    /// If not provided, the engine uses its default policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_policy: Option<ConflictResolutionPolicy>,

    /// Optional trust domain to scope source weighting (predicate/topic).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust_domain: Option<String>,

    /// Optional vector embedding for the query (semantic RESOLVE path).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResolveMode {
    /// Just return top-k beliefs (fastest)
    #[default]
    Simple,
    /// Aggregate and synthesize
    Aggregate,
    /// Temporal RESOLVE (as-of, diffs, trajectories).
    Temporal,
}
```

---

## Usage Examples

### Creating an Entity

```rust
let entity = Entity {
    id: EntityId::new(),
    canonical_name: "LK-99".to_string(),
    aliases: vec!["Room Temperature Superconductor".to_string()],
    entity_type: EntityType::Concept,
    created_at: Utc::now(),
    updated_at: Utc::now(),
    embedding: None,
    version: 1,
    metadata: serde_json::json!({}),
};
```

### Creating a Belief

```rust
let belief = Belief {
    id: BeliefId::new(),
    subject: entity.id,
    predicate: "is_superconductor".to_string(),
    value: Value::Bool(false),
    confidence: Confidence::probability(
        0.99,
        ConfidenceSource::AggregatedFromSources {
            source_ids: vec![source_1, source_2],
            aggregation_method: "weighted_average".to_string(),
        },
    )?,
    source: Source::Paper {
        arxiv_id: Some("2308.12345".to_string()),
        doi: None,
        title: Some("LK-99 Replication Failure".to_string()),
        authors: vec!["Smith, J.".to_string()],
    },
    valid_time: TimeRange::from_now(),
    tx_time: Utc::now(),
    consistency_status: ConsistencyStatus::Verified,
    supersedes: None,
    superseded_by: None,
    embedding: None,
};
```

### Serializing to IR

```rust
let ir = KyroIR {
    version: "1.0".to_string(),
    request_id: Uuid::new_v4(),
    timestamp: Utc::now(),
    operation: Operation::Assert(AssertPayload {
        entity_id: entity.id,
        predicate: "is_superconductor".to_string(),
        value: Value::Bool(false),
        confidence: confidence.clone(),
        source: source.clone(),
        valid_time: TimeRange::from_now(),
        consistency_mode: ConsistencyMode::Strict,
    }),
};

let json = serde_json::to_string_pretty(&ir)?;
println!("{}", json);
```
