//! Trust evaluation models for KyroQL.
//!
//! Trust is modeled separately from epistemic confidence and retrieval relevance.
//! A trust weight (0.0-1.0) scales how much a source should influence ranking
//! without mutating the stored belief confidence.

use std::collections::HashMap;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::source::Source;
use crate::confidence::SourceId;

/// Result of a trust evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TrustAssessment {
    /// Multiplicative weight in [0.0, 1.0].
    weight: f32,
}

impl TrustAssessment {
    /// Clamp and construct an assessment.
    pub fn new(weight: f32) -> Self {
        Self {
            weight: weight.clamp(0.0, 1.0),
        }
    }

    /// Returns the clamped weight.
    pub const fn weight(&self) -> f32 {
        self.weight
    }
}

/// Trust evaluation interface.
pub trait TrustModel: Send + Sync {
    /// Name of the trust model (for audit/debugging).
    fn name(&self) -> &str;

    /// Compute trust for a source within an optional domain (predicate, topic, etc.).
    fn assess(&self, source: &Source, domain: Option<&str>) -> TrustAssessment;
}

/// Simple trust model backed by in-memory weights.
///
/// - Global weights apply to all domains.
/// - Domain-specific weights override global weights when present.
#[derive(Debug, Default)]
pub struct SimpleTrustModel {
    global: RwLock<HashMap<SourceId, f32>>,
    domain_overrides: RwLock<HashMap<String, HashMap<SourceId, f32>>>,
}

impl SimpleTrustModel {
    /// Create a new instance.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a global trust weight for a source.
    pub fn set_global(&self, source: SourceId, weight: f32) {
        let mut guard = self.global.write().expect("trust global lock poisoned");
        guard.insert(source, weight.clamp(0.0, 1.0));
    }

    /// Set a domain-specific trust weight for a source.
    pub fn set_domain(&self, domain: impl Into<String>, source: SourceId, weight: f32) {
        let mut guard = self
            .domain_overrides
            .write()
            .expect("trust domain lock poisoned");
        guard
            .entry(domain.into())
            .or_default()
            .insert(source, weight.clamp(0.0, 1.0));
    }

    fn lookup(&self, source: SourceId, domain: Option<&str>) -> Option<f32> {
        if let Some(dom) = domain {
            let guard = self
                .domain_overrides
                .read()
                .expect("trust domain lock poisoned");
            if let Some(map) = guard.get(dom) {
                if let Some(w) = map.get(&source) {
                    return Some(*w);
                }
            }
        }
        let guard = self.global.read().expect("trust global lock poisoned");
        guard.get(&source).copied()
    }
}

impl TrustModel for SimpleTrustModel {
    fn name(&self) -> &str {
        "simple_trust"
    }

    fn assess(&self, source: &Source, domain: Option<&str>) -> TrustAssessment {
        let source_id = source.source_id();
        let weight = self.lookup(source_id, domain).unwrap_or(1.0);
        TrustAssessment::new(weight)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_weight_is_one() {
        let model = SimpleTrustModel::new();
        let source = Source::agent("agent-1", None::<String>);
        let assessment = model.assess(&source, None);
        assert_eq!(assessment.weight(), 1.0);
    }

    #[test]
    fn domain_overrides_take_precedence() {
        let model = SimpleTrustModel::new();
        let source = Source::agent("agent-1", None::<String>);
        let sid = source.source_id();
        model.set_global(sid, 0.8);
        model.set_domain("science", sid, 0.2);

        let global = model.assess(&source, None);
        let domain = model.assess(&source, Some("science"));
        assert_eq!(global.weight(), 0.8);
        assert_eq!(domain.weight(), 0.2);
    }
}
