//! MONITOR subsystem for reactive subscriptions.
//!
//! Phase 4 introduces monitoring triggers that fire on changes to the belief state.
//! This implementation is embedded-first (in-process) and provides a stream handle
//! for subscribers. A future server build can layer a transport (e.g., gRPC streaming)
//! on top of the `MonitorStream` abstraction.

/// Trigger storage and event dispatch worker.
pub mod dispatcher;
/// Trigger matching logic.
pub mod matcher;
/// Subscriber stream handle.
pub mod stream;
/// Trigger and event type definitions.
pub mod triggers;

pub use dispatcher::{MonitorRegistration, MonitorSystem, MonitorSystemConfig};
pub use stream::MonitorStream;
pub use triggers::{EventPayload, MonitorEvent, MonitorEventError, SubscriptionId, Trigger, TriggerId};
