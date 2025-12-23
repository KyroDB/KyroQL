//! Entity layer modules.
//!
//! This module groups entity, resolution, store, and versioning.

pub mod entity;
pub mod resolution;
pub mod store;
pub mod versioning;

pub use entity::{Entity, EntityId, EntityType};
