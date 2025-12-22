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
//! use kyroql::{Entity, Belief, Confidence, CalibrationMode};
//!
//! // Create an entity
//! let entity = Entity::new("LK-99", EntityType::Concept);
//!
//! // Assert a belief about the entity
//! let belief = Belief::builder()
//!     .subject(entity.id)
//!     .predicate("is_superconductor")
//!     .value(false)
//!     .confidence(Confidence::probability(0.99)?)
//!     .build()?;
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod entity;
pub mod belief;
pub mod confidence;
pub mod value;
pub mod time;
pub mod source;
pub mod conflict;
pub mod pattern;
pub mod frame;
pub mod error;

// Re-export primary types at crate root for convenience
pub use entity::{Entity, EntityId, EntityType};
pub use belief::{Belief, ConsistencyStatus};
pub use confidence::{Confidence, CalibrationMode, ConfidenceSource, BeliefId};
pub use value::Value;
pub use time::TimeRange;
pub use source::Source;
pub use conflict::{Conflict, ConflictId, ConflictType, ConflictStatus};
pub use pattern::{Pattern, PatternId, PatternRule};
pub use frame::{BeliefFrame, RankedClaim, Evidence, KnowledgeGap, GapType};
pub use error::{KyroError, ValidationError};
