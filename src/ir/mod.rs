//! Intermediate Representation (IR) for KyroQL operations.
//!
//! The IR provides a serializable, transportable format for all KyroQL
//! operations. This enables:
//! - Network transport between clients and servers
//! - Operation logging and replay
//! - Debugging and inspection

mod consistency;
mod operations;

pub use consistency::ConsistencyMode;
pub use operations::{
    AssertPayload, DefinePatternPayload, KyroIR, Operation, ResolvePayload, RetractPayload,
};
