//! Operation builders for KyroQL.
//!
//! These builders provide a fluent, type-safe API for constructing
//! KyroQL operations. They validate inputs before producing IR.

mod assert;
mod assert_builder;
mod resolve;
mod resolve_builder;

pub mod belief_frame;

pub use assert_builder::AssertBuilder;
pub use resolve_builder::ResolveBuilder;
