//! Persistent storage backend for KyroQL.
//!
//! This module provides durable, crash-safe storage with:
//! - Write-Ahead Logging (WAL) for crash recovery
//! - File locking for single-process concurrency
//! - CRC32 checksums for corruption detection
//! - Segmented storage for efficient reads
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │                     PersistentStores                          │
//! ├──────────────────────────────────────────────────────────────┤
//! │  ┌─────────────────┐  ┌─────────────────┐                    │
//! │  │ WriteAheadLog   │  │ SegmentManager  │                    │
//! │  │ (append-only)   │  │ (read-optimized)│                    │
//! │  └────────┬────────┘  └────────┬────────┘                    │
//! │           │                    │                             │
//! │           └──────────┬─────────┘                             │
//! │                      ↓                                       │
//! │           ┌─────────────────────┐                            │
//! │           │   FileLock (flock)  │                            │
//! │           └─────────────────────┘                            │
//! └──────────────────────────────────────────────────────────────┘
//! ```

mod wal;
mod file_lock;
mod segment;
mod codec;
mod stores;

pub use file_lock::FileLock;
pub use wal::{WalEntry, WalEntryKind, WriteAheadLog};
pub use segment::{Segment, SegmentManager};
pub use stores::{
    PersistentEntityStore, PersistentBeliefStore, PersistentPatternStore,
    PersistentConflictStore, PersistentDerivationStore, PersistentStores,
};

use std::path::Path;
use crate::error::{ExecutionError, KyroError};

/// Configuration for persistent storage.
#[derive(Debug, Clone)]
pub struct PersistentConfig {
    /// Maximum WAL size before compaction (bytes).
    pub max_wal_size: u64,
    /// Whether to fsync after every write (slower but safer).
    pub sync_on_write: bool,
    /// Maximum segment size (bytes).
    pub max_segment_size: u64,
}

impl Default for PersistentConfig {
    fn default() -> Self {
        Self {
            max_wal_size: 64 * 1024 * 1024,  // 64 MB
            sync_on_write: true,
            max_segment_size: 256 * 1024 * 1024,  // 256 MB
        }
    }
}

impl PersistentConfig {
    const MIN_WAL_SIZE: u64 = 4 * 1024; // 4 KiB minimum to avoid degenerate compaction loops
    const MIN_SEGMENT_SIZE: u64 = 16 * 1024; // 16 KiB minimum to avoid tiny segments

    pub fn validate(self) -> Result<Self, KyroError> {
        if self.max_wal_size < Self::MIN_WAL_SIZE {
            return Err(KyroError::Execution(ExecutionError::Storage {
                message: format!(
                    "max_wal_size must be at least {} bytes (got {})",
                    Self::MIN_WAL_SIZE, self.max_wal_size
                ),
            }));
        }

        if self.max_segment_size < Self::MIN_SEGMENT_SIZE {
            return Err(KyroError::Execution(ExecutionError::Storage {
                message: format!(
                    "max_segment_size must be at least {} bytes (got {})",
                    Self::MIN_SEGMENT_SIZE, self.max_segment_size
                ),
            }));
        }

        Ok(self)
    }
}

/// Open or create a persistent KyroQL database at the given path.
///
/// # Arguments
/// * `path` - Directory to store the database files
/// * `config` - Optional configuration (uses defaults if None)
///
/// # Returns
/// A `PersistentStores` instance ready for use with `KyroEngine`.
///
/// # Errors
/// - If the path cannot be created or accessed
/// - If another process holds the lock
/// - If WAL replay fails due to corruption
///
/// # Example
/// ```rust,ignore
/// use kyroql::storage::persistent::{open_database, PersistentConfig};
///
/// let stores = open_database("./brain.kyro", None)?;
/// let engine = KyroEngine::new(
///     Arc::new(stores.entities),
///     Arc::new(stores.beliefs),
///     // ...
/// );
/// ```
pub fn open_database(
    path: impl AsRef<Path>,
    config: Option<PersistentConfig>,
) -> Result<PersistentStores, KyroError> {
    let cfg = config.unwrap_or_default().validate()?;
    PersistentStores::open(path.as_ref(), cfg)
}
