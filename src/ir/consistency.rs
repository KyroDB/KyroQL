//! Consistency modes for ASSERT operations.

use serde::{Deserialize, Serialize};

/// Controls how consistency checks are applied during ASSERT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConsistencyMode {
    /// Check patterns and fail if conflicts are detected.
    /// This is the safest mode—no inconsistent data enters the system.
    #[default]
    Strict,

    /// Accept the belief immediately, check patterns asynchronously.
    /// Conflicts are recorded but do not block the write.
    Eventual,

    /// Override existing conflicts. Use with extreme caution.
    /// This should only be used for administrative corrections.
    Force,
}

impl ConsistencyMode {
    /// Returns `true` if this is `Strict` mode.
    pub const fn is_strict(&self) -> bool {
        matches!(self, Self::Strict)
    }

    /// Returns `true` if this is `Eventual` mode.
    pub const fn is_eventual(&self) -> bool {
        matches!(self, Self::Eventual)
    }

    /// Returns `true` if this is `Force` mode.
    pub const fn is_force(&self) -> bool {
        matches!(self, Self::Force)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_strict() {
        assert_eq!(ConsistencyMode::default(), ConsistencyMode::Strict);
    }

    #[test]
    fn test_mode_checks() {
        assert!(ConsistencyMode::Strict.is_strict());
        assert!(ConsistencyMode::Eventual.is_eventual());
        assert!(ConsistencyMode::Force.is_force());
    }

    #[test]
    fn test_serialization() {
        let mode = ConsistencyMode::Strict;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"strict\"");

        let mode = ConsistencyMode::Force;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"force\"");
    }

    #[test]
    fn test_deserialization() {
        let mode: ConsistencyMode = serde_json::from_str("\"eventual\"").unwrap();
        assert_eq!(mode, ConsistencyMode::Eventual);
    }
}
