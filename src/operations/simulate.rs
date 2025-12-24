//! SIMULATE operation builder.
//!
//! The SimulateBuilder provides a fluent API for constructing SIMULATE operations.
//!
//! Vision requirement: SIMULATE creates an isolated branched reality backed by delta
//! overlays (no base store mutation).

use crate::entity::EntityId;
use crate::error::ValidationError;
use crate::ir::{KyroIR, Operation, SimulatePayload};
use crate::simulation::SimulateConstraints;
use crate::time::TimeRange;
use crate::value::Value;

/// Builder for SIMULATE operations.
#[derive(Debug, Clone, Default)]
pub struct SimulateBuilder {
    scenario: Option<String>,
    context: Option<Value>,
    entities: Option<Vec<EntityId>>,
    initial_conditions: Option<Value>,
    constraints: Option<SimulateConstraints>,
    time_horizon: Option<TimeRange>,
    outcome_parameters: Option<Value>,
}

impl SimulateBuilder {
    /// Create a new builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a human-readable scenario description.
    #[must_use]
    pub fn scenario(mut self, scenario: impl Into<String>) -> Self {
        self.scenario = Some(scenario.into());
        self
    }

    /// Attach structured context for the simulation.
    #[must_use]
    pub fn context(mut self, context: Value) -> Self {
        self.context = Some(context);
        self
    }

    /// Restrict the simulation scope to a set of entities.
    #[must_use]
    pub fn entities(mut self, entities: Vec<EntityId>) -> Self {
        self.entities = Some(entities);
        self
    }

    /// Provide initial conditions (structured input).
    #[must_use]
    pub fn initial_conditions(mut self, conditions: Value) -> Self {
        self.initial_conditions = Some(conditions);
        self
    }

    /// Set resource constraints for the simulation.
    #[must_use]
    pub fn constraints(mut self, constraints: SimulateConstraints) -> Self {
        self.constraints = Some(constraints);
        self
    }

    /// Set a time horizon for the simulation.
    #[must_use]
    pub fn time_horizon(mut self, horizon: TimeRange) -> Self {
        self.time_horizon = Some(horizon);
        self
    }

    /// Provide outcome parameters.
    #[must_use]
    pub fn outcome_parameters(mut self, params: Value) -> Self {
        self.outcome_parameters = Some(params);
        self
    }

    /// Build the SIMULATE IR.
    pub fn build(self) -> Result<KyroIR, ValidationError> {
        let constraints = match self.constraints {
            None => None,
            Some(c) => {
                c.validate()?;
                let json = serde_json::to_value(c).map_err(|e| {
                    ValidationError::InvalidSimulationConstraints {
                        reason: format!("failed to serialize constraints: {e}"),
                    }
                })?;
                Some(Value::Structured(json))
            }
        };

        let payload = SimulatePayload {
            scenario: self.scenario,
            context: self.context,
            entities: self.entities,
            initial_conditions: self.initial_conditions,
            constraints,
            time_horizon: self.time_horizon,
            outcome_parameters: self.outcome_parameters,
        };

        Ok(KyroIR::new(Operation::Simulate(payload)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulate_builder_builds_with_defaults() {
        let ir = SimulateBuilder::new().build().unwrap();
        assert!(matches!(ir.operation, Operation::Simulate(_)));
    }

    #[test]
    fn simulate_builder_serializes_constraints_to_structured_value() {
        let ir = SimulateBuilder::new()
            .constraints(SimulateConstraints {
                max_affected_entities: 5,
                max_depth: 1,
                max_duration_ms: 10,
            })
            .build()
            .unwrap();

        let Operation::Simulate(p) = ir.operation else {
            panic!("expected simulate");
        };

        let Some(Value::Structured(v)) = p.constraints else {
            panic!("expected structured constraints");
        };

        assert_eq!(v.get("max_affected_entities").unwrap().as_u64().unwrap(), 5);
    }
}
