//! Operation builders for KyroQL.
//!
//! These builders provide a fluent, type-safe API for constructing
//! KyroQL operations. They validate inputs before producing IR.

mod assert_op;
mod resolve_op;

pub use assert_op::AssertBuilder;
pub use resolve_op::ResolveBuilder;
