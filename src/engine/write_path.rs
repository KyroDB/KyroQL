//! Engine write-path helpers.
//!
//! Phase 1 splits the engine into write/read paths. The current implementation
//! keeps execution in `crate::engine` and this module is reserved for extracting
//! the ASSERT/RETRACT write path without changing behavior.
