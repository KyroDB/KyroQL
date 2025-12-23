//! Entity versioning helpers.
//!
//! Version history is provided by storage backends that implement
//! `EntityStore::{get_at_version,list_versions}`.

pub use crate::storage::EntityStore;
