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

#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]

pub mod belief;
pub mod conflict;
pub mod derivation;
pub mod entity;
pub mod embedding;
pub mod error;
pub mod pattern;
pub mod inference; // Exposing the inference module

pub mod engine;

pub mod simulation;

pub mod monitor;

pub mod ir;
pub mod operations;
pub mod storage;

// Module aliases to preserve stable paths while matching the documented layout.
pub use belief::{confidence, source, time, value};
pub use operations::belief_frame as frame;

// Re-export primary types at crate root for convenience
pub use belief::{Belief, ConsistencyStatus};
pub use confidence::{BeliefId, CalibrationMode, Confidence, ConfidenceSource, SourceId};
pub use conflict::{Conflict, ConflictId, ConflictStatus, ConflictType};
pub use derivation::{DerivationId, DerivationRecord};
pub use entity::{Entity, EntityId, EntityType};
pub use embedding::{lexical_embedding, DEFAULT_EMBEDDING_DIM};
pub use error::{KyroError, ValidationError};
pub use frame::{BeliefFrame, Evidence, GapType, KnowledgeGap, RankedClaim};
pub use pattern::{Pattern, PatternId, PatternRule};
pub use source::Source;
pub use time::TimeRange;
pub use value::Value;

pub use ir::{
	AssertPayload, ConsistencyMode, DefinePatternPayload, DerivePayload, KyroIR, Operation,
	ResolvePayload, ResolveMode, RetractPayload,
};
pub use operations::{AssertBuilder, DeriveBuilder, ResolveBuilder};
pub use operations::SimulateBuilder;
pub use storage::{
    BeliefStore, ConflictStore, DerivationStore, EntityStore, PatternStore, StorageError,
};
pub use storage::{
	InMemoryBeliefStore, InMemoryConflictStore, InMemoryDerivationStore, InMemoryEntityStore,
	InMemoryPatternStore, InMemoryStores,
};

pub use engine::{EngineResponse, KyroEngine};
pub use engine::runtime::{DefaultRouter, ExecutionHandle, ExecutionPath, KyroRuntime, KyroRuntimeConfig};
pub use inference::ConflictResolutionPolicy; // Exposing ConflictResolutionPolicy from inference module

pub use simulation::{SimulateConstraints, SimulationContext, SimulationId, SimulationImpact};

pub use monitor::{EventPayload, MonitorEvent, MonitorEventError, MonitorRegistration, MonitorStream, MonitorSystem, MonitorSystemConfig, SubscriptionId, Trigger, TriggerId};

