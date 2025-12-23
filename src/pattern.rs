//! Pattern types for constraints and invariants.
//!
//! Patterns define expected properties that beliefs should satisfy.
//! When patterns are violated, conflicts are generated.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::confidence::Confidence;
use crate::entity::EntityType;
use crate::time::TimeRange;

/// Unique identifier for a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PatternId(Uuid);

impl PatternId {
    /// Creates a new random pattern ID.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PatternId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PatternId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Rules that patterns can enforce.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PatternRule {
    /// Value must be within a numeric range.
    Range {
        /// Predicate to check.
        predicate: String,
        /// Minimum value (inclusive).
        min: Option<f64>,
        /// Maximum value (inclusive).
        max: Option<f64>,
    },

    /// Only one value allowed per entity.
    Unique {
        /// Predicate that must be unique.
        predicate: String,
    },

    /// Number of values must be within bounds.
    Cardinality {
        /// Predicate to check.
        predicate: String,
        /// Minimum count.
        min: usize,
        /// Maximum count.
        max: usize,
    },

    /// Value must change monotonically.
    Monotonic {
        /// Predicate to check.
        predicate: String,
        /// Required direction.
        direction: MonotonicDirection,
    },

    /// Value must be one of allowed set.
    Enumerated {
        /// Predicate to check.
        predicate: String,
        /// Allowed values.
        allowed_values: Vec<String>,
    },

    /// Value must match regex pattern.
    Regex {
        /// Predicate to check.
        predicate: String,
        /// Regex pattern.
        pattern: String,
    },

    /// If A is true, B must also be true.
    Implication {
        /// Antecedent predicate.
        if_predicate: String,
        /// Consequent predicate.
        then_predicate: String,
    },

    /// Predicates cannot be true simultaneously.
    MutuallyExclusive {
        /// Conflicting predicates.
        predicates: Vec<String>,
    },

    /// Custom rule with expression.
    Custom {
        /// Rule name.
        name: String,
        /// Rule description.
        description: String,
        /// Optional expression.
        expression: Option<String>,
    },
}

impl PatternRule {
    /// Creates a range pattern.
    #[must_use]
    pub fn range(predicate: impl Into<String>, min: Option<f64>, max: Option<f64>) -> Self {
        Self::Range {
            predicate: predicate.into(),
            min,
            max,
        }
    }

    /// Creates a unique pattern.
    #[must_use]
    pub fn unique(predicate: impl Into<String>) -> Self {
        Self::Unique {
            predicate: predicate.into(),
        }
    }

    /// Creates a cardinality pattern.
    #[must_use]
    pub fn cardinality(predicate: impl Into<String>, min: usize, max: usize) -> Self {
        Self::Cardinality {
            predicate: predicate.into(),
            min,
            max,
        }
    }

    /// Creates a monotonically increasing pattern.
    #[must_use]
    pub fn monotonic_increasing(predicate: impl Into<String>) -> Self {
        Self::Monotonic {
            predicate: predicate.into(),
            direction: MonotonicDirection::Increasing,
        }
    }

    /// Creates a monotonically decreasing pattern.
    #[must_use]
    pub fn monotonic_decreasing(predicate: impl Into<String>) -> Self {
        Self::Monotonic {
            predicate: predicate.into(),
            direction: MonotonicDirection::Decreasing,
        }
    }

    /// Creates an enumerated pattern.
    #[must_use]
    pub fn enumerated(predicate: impl Into<String>, allowed_values: Vec<String>) -> Self {
        Self::Enumerated {
            predicate: predicate.into(),
            allowed_values,
        }
    }

    /// Creates a regex pattern.
    #[must_use]
    pub fn regex(predicate: impl Into<String>, pattern: impl Into<String>) -> Self {
        Self::Regex {
            predicate: predicate.into(),
            pattern: pattern.into(),
        }
    }

    /// Creates an implication pattern.
    #[must_use]
    pub fn implication(
        if_predicate: impl Into<String>,
        then_predicate: impl Into<String>,
    ) -> Self {
        Self::Implication {
            if_predicate: if_predicate.into(),
            then_predicate: then_predicate.into(),
        }
    }

    /// Creates a mutually exclusive pattern.
    #[must_use]
    pub fn mutually_exclusive(predicates: Vec<String>) -> Self {
        Self::MutuallyExclusive { predicates }
    }

    /// Returns the primary predicate this pattern applies to (if any).
    #[must_use]
    pub fn primary_predicate(&self) -> Option<&str> {
        match self {
            Self::Range { predicate, .. }
            | Self::Unique { predicate }
            | Self::Cardinality { predicate, .. }
            | Self::Monotonic { predicate, .. }
            | Self::Enumerated { predicate, .. }
            | Self::Regex { predicate, .. } => Some(predicate),
            Self::Implication { if_predicate, .. } => Some(if_predicate),
            Self::MutuallyExclusive { predicates } => predicates.first().map(String::as_str),
            Self::Custom { .. } => None,
        }
    }

    /// Returns all predicates this rule should be indexed under for lookup.
    ///
    /// This is used by storage backends to support `PatternStore::find_by_predicate`.
    ///
    /// For rules spanning multiple predicates (e.g., `MutuallyExclusive`), indexing only a
    /// single "primary" predicate would cause missed validations.
    #[must_use]
    pub fn indexed_predicates(&self) -> Vec<&str> {
        match self {
            Self::Range { predicate, .. }
            | Self::Unique { predicate }
            | Self::Cardinality { predicate, .. }
            | Self::Monotonic { predicate, .. }
            | Self::Enumerated { predicate, .. }
            | Self::Regex { predicate, .. } => vec![predicate.as_str()],
            Self::Implication {
                if_predicate,
                then_predicate,
            } => vec![if_predicate.as_str(), then_predicate.as_str()],
            Self::MutuallyExclusive { predicates } => predicates.iter().map(String::as_str).collect(),
            Self::Custom { .. } => Vec::new(),
        }
    }
}

impl fmt::Display for PatternRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Range { predicate, min, max } => {
                let min_str = min.map(|v| format!("{v}")).unwrap_or_else(|| "-∞".to_string());
                let max_str = max.map(|v| format!("{v}")).unwrap_or_else(|| "∞".to_string());
                write!(f, "range({predicate}: [{min_str}, {max_str}])")
            }
            Self::Unique { predicate } => write!(f, "unique({predicate})"),
            Self::Cardinality {
                predicate,
                min,
                max,
            } => write!(f, "cardinality({predicate}: [{min}, {max}])"),
            Self::Monotonic {
                predicate,
                direction,
            } => write!(f, "monotonic({predicate}, {direction})"),
            Self::Enumerated {
                predicate,
                allowed_values,
            } => write!(f, "enumerated({predicate}: {:?})", allowed_values),
            Self::Regex { predicate, pattern } => write!(f, "regex({predicate}: {pattern})"),
            Self::Implication {
                if_predicate,
                then_predicate,
            } => write!(f, "implication({if_predicate} → {then_predicate})"),
            Self::MutuallyExclusive { predicates } => {
                write!(f, "mutually_exclusive({:?})", predicates)
            }
            Self::Custom { name, .. } => write!(f, "custom({name})"),
        }
    }
}

/// Direction for monotonic patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonotonicDirection {
    /// Values must not decrease (non-strict/weak monotonicity).
    Increasing,
    /// Values must not increase (non-strict/weak monotonicity).
    Decreasing,
}

impl fmt::Display for MonotonicDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Increasing => write!(f, "increasing"),
            Self::Decreasing => write!(f, "decreasing"),
        }
    }
}

/// A pattern (invariant/constraint) that beliefs should satisfy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    /// Unique pattern ID.
    pub id: PatternId,
    /// Human-readable pattern name.
    pub name: String,

    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Domain entity type this pattern applies to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<EntityType>,

    /// The rule enforced by this pattern.
    pub rule: PatternRule,
    /// Confidence in the pattern itself.
    pub confidence: Confidence,
    /// When this pattern is valid.
    pub valid_time: TimeRange,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Whether the pattern is currently active.
    pub active: bool,

    /// Metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl Pattern {
    /// Creates a new pattern.
    #[must_use]
    pub fn new(name: impl Into<String>, rule: PatternRule, confidence: Confidence) -> Self {
        Self {
            id: PatternId::new(),
            name: name.into(),
            description: None,
            domain: None,
            rule,
            confidence,
            valid_time: TimeRange::from_now(),
            created_at: Utc::now(),
            active: true,
            metadata: serde_json::Value::Null,
        }
    }

    /// Sets the description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets the domain (entity type filter).
    #[must_use]
    pub fn with_domain(mut self, domain: EntityType) -> Self {
        self.domain = Some(domain);
        self
    }

    /// Sets the valid time.
    #[must_use]
    pub fn with_valid_time(mut self, valid_time: TimeRange) -> Self {
        self.valid_time = valid_time;
        self
    }

    /// Returns true if this pattern is active and currently valid.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active && self.valid_time.is_active()
    }

    /// Deactivates this pattern.
    pub fn deactivate(&mut self) {
        self.active = false;
    }

    /// Activates this pattern.
    pub fn activate(&mut self) {
        self.active = true;
    }

    /// Returns the primary predicate this pattern applies to (if any).
    #[must_use]
    pub fn primary_predicate(&self) -> Option<&str> {
        self.rule.primary_predicate()
    }
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Pattern {}

impl std::hash::Hash for Pattern {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_id() {
        let id1 = PatternId::new();
        let id2 = PatternId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_pattern_creation() {
        let pattern = Pattern::new(
            "temperature_range",
            PatternRule::range("temperature", Some(0.0), Some(100.0)),
            Confidence::from_agent(0.9, "test").unwrap(),
        );

        assert_eq!(pattern.name, "temperature_range");
        assert!(pattern.is_active());
        assert_eq!(pattern.primary_predicate(), Some("temperature"));
    }

    #[test]
    fn test_pattern_with_description() {
        let pattern = Pattern::new(
            "test",
            PatternRule::unique("id"),
            Confidence::from_agent(0.9, "test").unwrap(),
        )
        .with_description("Test pattern");

        assert_eq!(pattern.description.as_deref(), Some("Test pattern"));
    }

    #[test]
    fn test_pattern_with_domain() {
        let pattern = Pattern::new(
            "test",
            PatternRule::unique("id"),
            Confidence::from_agent(0.9, "test").unwrap(),
        )
        .with_domain(EntityType::Person);

        assert_eq!(pattern.domain, Some(EntityType::Person));
    }

    #[test]
    fn test_pattern_deactivate() {
        let mut pattern = Pattern::new(
            "test",
            PatternRule::unique("id"),
            Confidence::from_agent(0.9, "test").unwrap(),
        );

        assert!(pattern.is_active());
        pattern.deactivate();
        assert!(!pattern.is_active());
        pattern.activate();
        assert!(pattern.is_active());
    }

    #[test]
    fn test_pattern_rule_range() {
        let rule = PatternRule::range("temp", Some(-40.0), Some(150.0));
        let display = format!("{rule}");
        assert!(display.contains("range"));
        assert!(display.contains("temp"));
    }

    #[test]
    fn test_pattern_rule_unique() {
        let rule = PatternRule::unique("email");
        assert_eq!(rule.primary_predicate(), Some("email"));
    }

    #[test]
    fn test_pattern_rule_cardinality() {
        let rule = PatternRule::cardinality("phone", 0, 3);
        let display = format!("{rule}");
        assert!(display.contains("cardinality"));
    }

    #[test]
    fn test_pattern_rule_monotonic() {
        let rule = PatternRule::monotonic_increasing("version");
        let display = format!("{rule}");
        assert!(display.contains("monotonic"));
        assert!(display.contains("increasing"));
    }

    #[test]
    fn test_pattern_rule_enumerated() {
        let rule = PatternRule::enumerated(
            "status",
            vec!["pending".to_string(), "active".to_string(), "done".to_string()],
        );
        let display = format!("{rule}");
        assert!(display.contains("enumerated"));
    }

    #[test]
    fn test_pattern_rule_regex() {
        let rule = PatternRule::regex("email", r"^[\w-\.]+@[\w-\.]+\.\w+$");
        let display = format!("{rule}");
        assert!(display.contains("regex"));
    }

    #[test]
    fn test_pattern_rule_implication() {
        let rule = PatternRule::implication("is_admin", "has_permissions");
        let display = format!("{rule}");
        assert!(display.contains("implication"));
        assert!(display.contains("→"));
    }

    #[test]
    fn test_pattern_rule_mutually_exclusive() {
        let rule = PatternRule::mutually_exclusive(vec![
            "is_active".to_string(),
            "is_deleted".to_string(),
        ]);
        let display = format!("{rule}");
        assert!(display.contains("mutually_exclusive"));
    }

    #[test]
    fn test_monotonic_direction_display() {
        assert_eq!(format!("{}", MonotonicDirection::Increasing), "increasing");
        assert_eq!(format!("{}", MonotonicDirection::Decreasing), "decreasing");
    }

    #[test]
    fn test_pattern_serialization() {
        let pattern = Pattern::new(
            "test",
            PatternRule::unique("id"),
            Confidence::from_agent(0.9, "test").unwrap(),
        );

        let json = serde_json::to_string(&pattern).unwrap();
        let deserialized: Pattern = serde_json::from_str(&json).unwrap();
        assert_eq!(pattern.id, deserialized.id);
        assert_eq!(pattern.name, deserialized.name);
    }
}
