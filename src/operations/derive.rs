//! DERIVE operation builder.
//!
//! DERIVE records a derivation chain (premises + rule + optional steps) and can optionally
//! attach the record to a specific derived belief.

use crate::confidence::BeliefId;
use crate::error::ValidationError;
use crate::ir::{DerivePayload, KyroIR, Operation};

/// Builder for DERIVE operations.
#[derive(Debug, Clone, Default)]
pub struct DeriveBuilder {
    rule: Option<String>,
    derived_belief_id: Option<BeliefId>,
    sources: Vec<BeliefId>,
    inference_steps: Vec<String>,
    confidence: Option<f32>,
    justification: Option<String>,
    metadata: Option<serde_json::Value>,
}

impl DeriveBuilder {
    /// Create a new builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the derivation rule name/identifier.
    #[must_use]
    pub fn rule(mut self, rule: impl Into<String>) -> Self {
        self.rule = Some(rule.into());
        self
    }

    /// Attach this derivation record to a derived belief.
    #[must_use]
    pub fn derived_belief(mut self, belief_id: BeliefId) -> Self {
        self.derived_belief_id = Some(belief_id);
        self
    }

    /// Add a premise belief ID.
    #[must_use]
    pub fn add_source(mut self, belief_id: BeliefId) -> Self {
        self.sources.push(belief_id);
        self
    }

    /// Set all premise belief IDs.
    #[must_use]
    pub fn sources(mut self, sources: Vec<BeliefId>) -> Self {
        self.sources = sources;
        self
    }

    /// Add a human-readable inference step.
    #[must_use]
    pub fn add_step(mut self, step: impl Into<String>) -> Self {
        self.inference_steps.push(step.into());
        self
    }

    /// Set inference steps.
    #[must_use]
    pub fn inference_steps(mut self, steps: Vec<String>) -> Self {
        self.inference_steps = steps;
        self
    }

    /// Set propagated confidence for the derived belief (0.0..=1.0).
    #[must_use]
    pub fn confidence(mut self, value: f32) -> Self {
        self.confidence = Some(value);
        self
    }

    /// Set justification.
    #[must_use]
    pub fn justification(mut self, text: impl Into<String>) -> Self {
        self.justification = Some(text.into());
        self
    }

    /// Set metadata.
    #[must_use]
    pub fn metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Build the DERIVE IR.
    pub fn build(self) -> Result<KyroIR, ValidationError> {
        let payload = DerivePayload {
            rule: self.rule,
            derived_belief_id: self.derived_belief_id,
            sources: Some(self.sources),
            inference_steps: if self.inference_steps.is_empty() {
                None
            } else {
                Some(self.inference_steps)
            },
            confidence: self.confidence,
            justification: self.justification,
            metadata: self.metadata,
        };

        // Match the runtime contract: builders must validate before producing IR.
        payload.validate()?;

        Ok(KyroIR::new(Operation::Derive(payload)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_builder_requires_rule_and_sources() {
        assert!(DeriveBuilder::new().build().is_err());

        assert!(DeriveBuilder::new()
            .rule("r")
            .add_source(BeliefId::new())
            .build()
            .is_ok());
    }
}
