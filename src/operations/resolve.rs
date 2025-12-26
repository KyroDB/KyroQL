//! RESOLVE operation builder.
//!
//! The ResolveBuilder provides a fluent, type-safe API for constructing
//! RESOLVE operations (queries). It validates inputs and provides sensible defaults.

use chrono::{DateTime, Utc};

use crate::entity::EntityId;
use crate::error::ValidationError;
use crate::inference::ConflictResolutionPolicy;
use crate::ir::{KyroIR, Operation, ResolveMode, ResolvePayload};

/// Builder for RESOLVE operations.
///
/// # Example
/// ```rust,ignore
/// let ir = ResolveBuilder::new()
///     .query("What is the current temperature?")
///     .entity(sensor_entity_id)
///     .min_confidence(0.7)
///     .limit(5)
///     .build()?;
/// ```
#[derive(Debug, Clone)]
pub struct ResolveBuilder {
    query: Option<String>,
    query_embedding: Option<Vec<f32>>,
    entity_id: Option<EntityId>,
    predicate: Option<String>,
    mode: ResolveMode,
    as_of: Option<DateTime<Utc>>,
    min_confidence: Option<f32>,
    limit: Option<usize>,
    include_counter_evidence: bool,
    include_gaps: bool,
    conflict_policy: Option<ConflictResolutionPolicy>,
    trust_domain: Option<String>,
}

impl Default for ResolveBuilder {
    fn default() -> Self {
        Self {
            query: None,
            query_embedding: None,
            entity_id: None,
            predicate: None,
            mode: ResolveMode::Simple,
            as_of: None,
            min_confidence: None,
            limit: None,
            include_counter_evidence: false,
            include_gaps: true,
            conflict_policy: None,
            trust_domain: None,
        }
    }
}

impl ResolveBuilder {
    /// Creates a new builder with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a natural language or structured query (optional).
    #[must_use]
    pub fn query(mut self, query: impl Into<String>) -> Self {
        self.query = Some(query.into());
        self
    }

    /// Provide a pre-computed query embedding for semantic RESOLVE.
    #[must_use]
    pub fn query_embedding(mut self, embedding: Vec<f32>) -> Self {
        self.query_embedding = Some(embedding);
        self
    }

    /// Filter results to a specific entity (optional).
    #[must_use]
    pub fn entity(mut self, id: EntityId) -> Self {
        self.entity_id = Some(id);
        self
    }

    /// Filter results to a specific predicate (optional).
    #[must_use]
    pub fn predicate(mut self, predicate: impl Into<String>) -> Self {
        self.predicate = Some(predicate.into());
        self
    }

    /// Select the RESOLVE mode.
    ///
    /// This is a routing hint for execution-path selection (Reflex vs Reflection).
    /// Default: `ResolveMode::Simple`.
    #[must_use]
    pub fn mode(mut self, mode: ResolveMode) -> Self {
        self.mode = mode;
        self
    }

    /// Query as of a specific point in time (optional).
    #[must_use]
    pub fn as_of(mut self, time: DateTime<Utc>) -> Self {
        self.as_of = Some(time);
        self
    }

    /// Set minimum confidence threshold (0.0 to 1.0).
    #[must_use]
    pub fn min_confidence(mut self, confidence: f32) -> Self {
        self.min_confidence = Some(confidence);
        self
    }

    /// Set maximum number of results (default: 10).
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Include counter-evidence in the response (default: false).
    #[must_use]
    pub fn include_counter_evidence(mut self) -> Self {
        self.include_counter_evidence = true;
        self
    }

    /// Exclude knowledge gaps from the response (default: include).
    #[must_use]
    pub fn exclude_gaps(mut self) -> Self {
        self.include_gaps = false;
        self
    }

    /// Select how RESOLVE should handle competing beliefs.
    #[must_use]
    pub fn conflict_policy(mut self, policy: ConflictResolutionPolicy) -> Self {
        self.conflict_policy = Some(policy);
        self
    }

    /// Scope trust weighting to a domain (predicate/topic).
    #[must_use]
    pub fn trust_domain(mut self, domain: impl Into<String>) -> Self {
        self.trust_domain = Some(domain.into());
        self
    }

    /// Build the RESOLVE IR.
    ///
    /// Returns `ValidationError` if:
    /// - No query, entity, or predicate is specified (at least one required)
    /// - min_confidence is out of range [0.0, 1.0]
    pub fn build(self) -> Result<KyroIR, ValidationError> {
        // At least one filter must be specified
        if self.query.is_none()
            && self.query_embedding.is_none()
            && self.entity_id.is_none()
            && self.predicate.is_none()
        {
            return Err(ValidationError::MissingField {
                field: "query, query_embedding, entity_id, or predicate (at least one required)"
                    .to_string(),
            });
        }

        // Validate min_confidence range
        if let Some(conf) = self.min_confidence {
            if !(0.0..=1.0).contains(&conf) {
                return Err(ValidationError::ConfidenceOutOfRange { value: conf });
            }
        }

        // If the caller provided a query but no embedding, generate a deterministic lexical embedding.
        let query_embedding = match (self.query.as_deref(), self.query_embedding) {
            (_, Some(v)) => Some(v),
            (Some(q), None) if !q.trim().is_empty() => Some(crate::embedding::lexical_embedding(q)),
            _ => None,
        };

        let payload = ResolvePayload {
            mode: self.mode,
            query: self.query,
            query_embedding,
            entity_id: self.entity_id,
            predicate: self.predicate,
            as_of: self.as_of,
            min_confidence: self.min_confidence,
            limit: self.limit.unwrap_or(10),
            include_counter_evidence: self.include_counter_evidence,
            include_gaps: self.include_gaps,
            conflict_policy: self.conflict_policy,
            trust_domain: self.trust_domain,
        };

        Ok(KyroIR::new(Operation::Resolve(payload)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confidence::SourceId;

    #[test]
    fn test_query_only() {
        let ir = ResolveBuilder::new()
            .query("What is the temperature?")
            .build();

        assert!(ir.is_ok());
        let ir = ir.unwrap();
        assert!(matches!(ir.operation, Operation::Resolve(_)));
    }

    #[test]
    fn test_entity_only() {
        let ir = ResolveBuilder::new().entity(EntityId::new()).build();
        assert!(ir.is_ok());
    }

    #[test]
    fn test_predicate_only() {
        let ir = ResolveBuilder::new().predicate("temperature").build();
        assert!(ir.is_ok());
    }

    #[test]
    fn test_no_filter_fails() {
        let result = ResolveBuilder::new().build();
        assert!(result.is_err());
    }

    #[test]
    fn test_min_confidence_valid() {
        let ir = ResolveBuilder::new()
            .query("test")
            .min_confidence(0.5)
            .build();
        assert!(ir.is_ok());
    }

    #[test]
    fn test_min_confidence_invalid_high() {
        let result = ResolveBuilder::new()
            .query("test")
            .min_confidence(1.5)
            .build();

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ValidationError::ConfidenceOutOfRange { .. }
        ));
    }

    #[test]
    fn test_min_confidence_invalid_low() {
        let result = ResolveBuilder::new()
            .query("test")
            .min_confidence(-0.1)
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_default_limit() {
        let ir = ResolveBuilder::new().query("test").build().unwrap();

        if let Operation::Resolve(payload) = ir.operation {
            assert_eq!(payload.limit, 10);
        } else {
            panic!("Expected Resolve operation");
        }
    }

    #[test]
    fn test_custom_limit() {
        let ir = ResolveBuilder::new()
            .query("test")
            .limit(25)
            .build()
            .unwrap();

        if let Operation::Resolve(payload) = ir.operation {
            assert_eq!(payload.limit, 25);
        } else {
            panic!("Expected Resolve operation");
        }
    }

    #[test]
    fn test_include_counter_evidence() {
        let ir = ResolveBuilder::new()
            .query("test")
            .include_counter_evidence()
            .build()
            .unwrap();

        if let Operation::Resolve(payload) = ir.operation {
            assert!(payload.include_counter_evidence);
        } else {
            panic!("Expected Resolve operation");
        }
    }

    #[test]
    fn test_exclude_gaps() {
        let ir = ResolveBuilder::new()
            .query("test")
            .exclude_gaps()
            .build()
            .unwrap();

        if let Operation::Resolve(payload) = ir.operation {
            assert!(!payload.include_gaps);
        } else {
            panic!("Expected Resolve operation");
        }
    }

    #[test]
    fn test_conflict_policy_plumbed() {
        let sid = SourceId::new();
        let ir = ResolveBuilder::new()
            .query("test")
            .conflict_policy(ConflictResolutionPolicy::source_priority(vec![sid]).unwrap())
            .build()
            .unwrap();

        let Operation::Resolve(payload) = ir.operation else {
            panic!("Expected Resolve operation");
        };

        assert!(matches!(
            payload.conflict_policy,
            Some(ConflictResolutionPolicy::SourcePriority { .. })
        ));
    }

    #[test]
    fn test_as_of() {
        let time = Utc::now();
        let ir = ResolveBuilder::new()
            .query("test")
            .as_of(time)
            .build()
            .unwrap();

        if let Operation::Resolve(payload) = ir.operation {
            assert_eq!(payload.as_of, Some(time));
        } else {
            panic!("Expected Resolve operation");
        }
    }

    #[test]
    fn test_combined_filters() {
        let entity_id = EntityId::new();
        let ir = ResolveBuilder::new()
            .query("What is the temperature?")
            .entity(entity_id)
            .predicate("temperature")
            .min_confidence(0.7)
            .limit(5)
            .build()
            .unwrap();

        if let Operation::Resolve(payload) = ir.operation {
            assert!(payload.query.is_some());
            assert_eq!(payload.entity_id, Some(entity_id));
            assert_eq!(payload.predicate, Some("temperature".to_string()));
            assert_eq!(payload.min_confidence, Some(0.7));
            assert_eq!(payload.limit, 5);
        } else {
            panic!("Expected Resolve operation");
        }
    }
}
