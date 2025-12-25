//! Trigger and event types for the MONITOR subsystem.
//!
//! These types are intentionally serializable so they can be represented in IR
//! payloads and streamed to subscribers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::confidence::BeliefId;
use crate::conflict::ConflictType;
use crate::entity::EntityId;
use crate::pattern::PatternId;
use crate::value::Value;

/// Unique identifier for a trigger.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TriggerId(Uuid);

impl TriggerId {
    /// Create a new random trigger id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Wrap an existing UUID.
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for TriggerId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for a subscription.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SubscriptionId(Uuid);

impl SubscriptionId {
    /// Create a new random subscription id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Wrap an existing UUID.
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for SubscriptionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Monitoring trigger definitions.
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    /// Confidence changed by more than threshold.
    ConfidenceShift {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entity_id: Option<EntityId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        predicate: Option<String>,
        threshold: f32,
    },

    /// New conflict detected.
    ConflictCreated {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entity_id: Option<EntityId>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        conflict_types: Vec<ConflictType>,
    },

    /// Pattern violated.
    PatternViolation {
        pattern_id: PatternId,
    },

    /// Entropy spike for a predicate "domain".
    ///
    /// Current embedded implementation interprets `domain` as an exact predicate
    /// string and computes Shannon entropy (base-2) over competing AS-OF values
    /// weighted by belief confidence.
    EntropySpike {
        domain: String,
        threshold: f32,
    },

    /// Previously missing data now available.
    GapFilled {
        entity_id: EntityId,
        predicate: String,
    },
}

/// Event payload emitted when a trigger fires.
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventPayload {
    /// Fired after an ASSERT has committed.
    AssertCommitted {
        belief_id: BeliefId,
        entity_id: EntityId,
        predicate: String,
        value: Value,
        confidence: f32,
    },

    /// Confidence shift details.
    ConfidenceShift {
        belief_id: BeliefId,
        entity_id: EntityId,
        predicate: String,
        previous: f32,
        current: f32,
        delta: f32,
    },

    /// Conflict details.
    ConflictCreated {
        belief_id: BeliefId,
        entity_id: EntityId,
        predicate: String,
        conflict_types: Vec<ConflictType>,
    },

    /// Pattern violation details.
    PatternViolation {
        belief_id: BeliefId,
        entity_id: EntityId,
        predicate: String,
        pattern_id: PatternId,
    },

    /// Entropy spike details.
    EntropySpike {
        belief_id: BeliefId,
        entity_id: EntityId,
        predicate: String,
        entropy_bits: f32,
        threshold_bits: f32,
    },

    /// Gap filled details.
    GapFilled {
        belief_id: BeliefId,
        entity_id: EntityId,
        predicate: String,
    },
}

/// A fired monitoring event.
#[allow(missing_docs)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MonitorEvent {
    pub event_id: Uuid,
    pub trigger_id: TriggerId,
    pub trigger_type: Trigger,
    pub timestamp: DateTime<Utc>,
    pub payload: EventPayload,
}

/// Errors constructing monitor events.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum MonitorEventError {
    /// The trigger type and event payload are inconsistent.
    #[error("trigger/payload mismatch: trigger={trigger:?} payload={payload:?}")]
    TriggerPayloadMismatch { trigger: Trigger, payload: EventPayload },
}

impl MonitorEvent {
    #[must_use]
    pub fn new(
        trigger_id: TriggerId,
        trigger_type: Trigger,
        payload: EventPayload,
    ) -> Result<Self, MonitorEventError> {
        let ok = match (&trigger_type, &payload) {
            (Trigger::ConfidenceShift { .. }, EventPayload::ConfidenceShift { .. }) => true,
            (Trigger::ConflictCreated { .. }, EventPayload::ConflictCreated { .. }) => true,
            (Trigger::PatternViolation { .. }, EventPayload::PatternViolation { .. }) => true,
            (Trigger::EntropySpike { .. }, EventPayload::EntropySpike { .. }) => true,
            (Trigger::GapFilled { .. }, EventPayload::GapFilled { .. }) => true,
            _ => false,
        };

        if !ok {
            return Err(MonitorEventError::TriggerPayloadMismatch {
                trigger: trigger_type,
                payload,
            });
        }

        Ok(Self {
            event_id: Uuid::new_v4(),
            trigger_id,
            trigger_type,
            timestamp: Utc::now(),
            payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_event_rejects_mismatched_trigger_and_payload() {
        let trigger_id = TriggerId::new();
        let trigger = Trigger::ConfidenceShift {
            entity_id: None,
            predicate: None,
            threshold: 0.5,
        };

        let payload = EventPayload::GapFilled {
            belief_id: BeliefId::new(),
            entity_id: EntityId::new(),
            predicate: "p".to_string(),
        };

        let err = MonitorEvent::new(trigger_id, trigger.clone(), payload.clone()).unwrap_err();
        assert_eq!(
            err,
            MonitorEventError::TriggerPayloadMismatch {
                trigger,
                payload
            }
        );
    }

    #[test]
    fn monitor_event_accepts_matching_trigger_and_payload() {
        let trigger_id = TriggerId::new();
        let trigger = Trigger::GapFilled {
            entity_id: EntityId::new(),
            predicate: "p".to_string(),
        };

        let payload = EventPayload::GapFilled {
            belief_id: BeliefId::new(),
            entity_id: EntityId::new(),
            predicate: "p".to_string(),
        };

        let ev = MonitorEvent::new(trigger_id, trigger.clone(), payload.clone()).unwrap();
        assert_eq!(ev.trigger_id, trigger_id);
        assert_eq!(ev.trigger_type, trigger);
        assert_eq!(ev.payload, payload);
    }
}
