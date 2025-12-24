//! Intermediate Representation (IR) for KyroQL operations.
//!
//! The IR provides a serializable, transportable format for all KyroQL
//! operations. This enables:
//! - Network transport between clients and servers
//! - Operation logging and replay
//! - Debugging and inspection

mod consistency;
mod operations;
mod serialization;
mod validation;

pub use consistency::ConsistencyMode;
pub use operations::{
    AssertPayload, DefinePatternPayload, DerivePayload, KyroIR, MonitorPayload, Operation,
    ResolveMode, ResolvePayload, RetractPayload, SimulatePayload,
};

pub use serialization::{from_json, to_json_pretty};
pub use validation::{MAX_EMBEDDING_DIM, MAX_TEXT_LEN};
