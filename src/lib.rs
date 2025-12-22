//! # KyroQL - The Cognitive Protocol for Superintelligence
//!
//! KyroQL is a protocol for synchronizing belief states between intelligent agents
//! and their memory substrate. It transforms the database from a passive storage
//! container into an active cognitive co-processor.
//!
//! ## Core Concepts
//!
//! - **Entity**: A stable identity anchor for beliefs
//! - **Belief**: An atomic unit of knowledge with confidence, provenance, and temporal validity
//! - **Confidence**: Formalized uncertainty with calibration semantics
//! - **BeliefFrame**: Structured response containing answer, evidence, conflicts, and gaps
//!
//! ## Usage
//!
//! ```rust,ignore
//! use kyroql::{Belief, Confidence, Entity, EntityType, TimeRange, Value};
//! use kyroql::Source;
//!
//! // Create an entity
//! let entity = Entity::new("LK-99", EntityType::Concept);
//!
//! // Assert a belief about the entity
//! let belief = Belief::builder()
//!     .subject(entity.id)
//!     .predicate("is_superconductor")
//!     .value(Value::Bool(false))
//!     .confidence(Confidence::from_agent(0.99, "researcher-1")?)
//!     .source(Source::paper("2308.12345", "LK-99 report"))
//!     .valid_time(TimeRange::from_now())
//!     .build()?;
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

// Phase 0: Core types
pub mod belief;
pub mod confidence;
pub mod conflict;
pub mod entity;
pub mod error;
pub mod frame;
pub mod pattern;
pub mod source;
pub mod time;
pub mod value;

// Phase 1: IR, Storage, and Operations
pub mod ir;
pub mod operations;
pub mod storage;

// Re-export primary types at crate root for convenience
pub use belief::{Belief, ConsistencyStatus};
pub use confidence::{BeliefId, CalibrationMode, Confidence, ConfidenceSource, SourceId};
pub use conflict::{Conflict, ConflictId, ConflictStatus, ConflictType};
pub use entity::{Entity, EntityId, EntityType};
pub use error::{KyroError, ValidationError};
pub use frame::{BeliefFrame, Evidence, GapType, KnowledgeGap, RankedClaim};
pub use pattern::{Pattern, PatternId, PatternRule};
pub use source::Source;
pub use time::TimeRange;
pub use value::Value;

// Phase 1 re-exports
pub use ir::{
	AssertPayload, ConsistencyMode, DefinePatternPayload, KyroIR, Operation, ResolvePayload,
	RetractPayload,
};
pub use operations::{AssertBuilder, ResolveBuilder};
pub use storage::{BeliefStore, ConflictStore, EntityStore, PatternStore, StorageError};

