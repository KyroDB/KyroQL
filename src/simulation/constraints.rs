//! Simulation constraints (resource limits).

use serde::{Deserialize, Serialize};

use crate::error::ValidationError;

/// Constraints that bound a simulation's resource usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulateConstraints {
    /// Maximum unique entities that may be affected by the simulation.
    pub max_affected_entities: usize,
    /// Maximum reasoning depth / number of steps.
    pub max_depth: usize,
    /// Maximum wall-clock duration for the simulation.
    pub max_duration_ms: u64,
}

impl Default for SimulateConstraints {
    fn default() -> Self {
        Self {
            max_affected_entities: 1000,
            max_depth: 2,
            max_duration_ms: 500,
        }
    }
}

impl SimulateConstraints {
    /// Validate constraints.
    ///
    /// This must be called before constructing a `SimulationContext`.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.max_affected_entities == 0 {
            return Err(ValidationError::InvalidSimulationConstraints {
                reason: "max_affected_entities must be > 0".to_string(),
            });
        }
        if self.max_depth == 0 {
            return Err(ValidationError::InvalidSimulationConstraints {
                reason: "max_depth must be > 0".to_string(),
            });
        }
        if self.max_duration_ms == 0 {
            return Err(ValidationError::InvalidSimulationConstraints {
                reason: "max_duration_ms must be > 0".to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constraints_default_is_valid() {
        SimulateConstraints::default().validate().unwrap();
    }

    #[test]
    fn constraints_reject_zero_limits() {
        let mut c = SimulateConstraints::default();
        c.max_affected_entities = 0;
        assert!(c.validate().is_err());

        let mut c = SimulateConstraints::default();
        c.max_depth = 0;
        assert!(c.validate().is_err());

        let mut c = SimulateConstraints::default();
        c.max_duration_ms = 0;
        assert!(c.validate().is_err());
    }
}
