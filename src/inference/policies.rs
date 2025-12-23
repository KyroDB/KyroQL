use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::confidence::SourceId;
use crate::error::ValidationError;

/// A non-empty, order-preserving, deduplicated list of source IDs.
///
/// - Empty lists are rejected.
/// - Duplicate source IDs are ignored (first occurrence wins).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SourcePriorityList(Vec<SourceId>);

impl SourcePriorityList {
    /// Construct a validated priority list.
    ///
    /// # Validation
    /// - Returns an error if `priority` is empty.
    /// - Deduplicates entries while preserving order.
    pub fn new(priority: Vec<SourceId>) -> Result<Self, ValidationError> {
        if priority.is_empty() {
            return Err(ValidationError::InvalidConflictResolutionPolicy {
                reason: "source_priority list cannot be empty".to_string(),
            });
        }

        let mut seen: HashSet<SourceId> = HashSet::with_capacity(priority.len());
        let mut deduped: Vec<SourceId> = Vec::with_capacity(priority.len());
        for id in priority {
            if seen.insert(id) {
                deduped.push(id);
            }
        }

        if deduped.is_empty() {
            return Err(ValidationError::InvalidConflictResolutionPolicy {
                reason: "source_priority list cannot be empty after deduplication".to_string(),
            });
        }

        Ok(Self(deduped))
    }

    /// Returns the list as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[SourceId] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SourcePriorityList {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = Vec::<SourceId>::deserialize(deserializer)?;
        SourcePriorityList::new(raw).map_err(serde::de::Error::custom)
    }
}

/// Conflict resolution policy used during RESOLVE.
///
/// Policies are intentionally *pure* (no I/O) so a RESOLVE result can be
/// reproduced deterministically given the same belief set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ConflictResolutionPolicy {
    /// Select the newest claim (by `tx_time`).
    LatestWins,

    /// Select the claim with the highest belief confidence.
    HighestConfidence,

    /// Select based on a source-trust priority list.
    ///
    /// Earlier entries have higher priority.
    ///
    /// Validation / normalization rules:
    /// - An empty list is rejected.
    /// - Duplicate `SourceId`s are ignored (first occurrence wins).
    SourcePriority {
        /// Ordered list of trusted sources (first = highest priority).
        priority: SourcePriorityList,
    },

    /// Do not resolve; return conflicts and competing evidence.
    ExplicitConflict,
}

impl Default for ConflictResolutionPolicy {
    fn default() -> Self {
        Self::HighestConfidence
    }
}

impl ConflictResolutionPolicy {
    /// Create a validated `SourcePriority` policy.
    ///
    /// Rejects empty lists and deduplicates entries while preserving order.
    pub fn source_priority(priority: Vec<SourceId>) -> Result<Self, ValidationError> {
        Ok(Self::SourcePriority {
            priority: SourcePriorityList::new(priority)?,
        })
    }

    /// Returns the source priority list, if this policy is `SourcePriority`.
    #[must_use]
    pub fn priority_list(&self) -> Option<&[SourceId]> {
        match self {
            Self::SourcePriority { priority } => Some(priority.as_slice()),
            _ => None,
        }
    }

    /// Returns a short stable identifier suitable for logging/debugging.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::LatestWins => "latest_wins",
            Self::HighestConfidence => "highest_confidence",
            Self::SourcePriority { .. } => "source_priority",
            Self::ExplicitConflict => "explicit_conflict",
        }
    }
}
