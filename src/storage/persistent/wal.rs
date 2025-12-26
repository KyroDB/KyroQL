//! Write-Ahead Log (WAL) for crash recovery.
//!
//! The WAL provides durability by:
//! 1. Writing all mutations to an append-only log before applying them
//! 2. Using fsync to ensure data reaches disk
//! 3. Replaying the log on startup to recover state
//!
//! # File Format
//! ```text
//! [MAGIC: 4 bytes][VERSION: 1 byte]
//! [ENTRY 1: codec-encoded WalEntry]
//! [ENTRY 2: codec-encoded WalEntry]
//! ...
//! ```

use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Seek, Write, Result as IoResult, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};

use crate::belief::Belief;
use crate::conflict::Conflict;
use crate::derivation::DerivationRecord;
use crate::entity::{Entity, EntityId};
use crate::pattern::Pattern;
use crate::confidence::BeliefId;

use super::codec;

/// A single entry in the write-ahead log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalEntry {
    /// Monotonically increasing sequence number.
    pub sequence: u64,
    /// When this entry was written.
    pub timestamp: DateTime<Utc>,
    /// The operation being logged.
    pub kind: WalEntryKind,
}

/// The type of WAL entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WalEntryKind {
    // Entity operations
    EntityInsert(Entity),
    EntityUpdate(Entity),
    EntityDelete { id: crate::entity::EntityId },
    EntityMerge {
        /// The merged primary entity (post-merge state).
        merged: Entity,
        /// Secondary entity that was merged into the primary.
        secondary_id: EntityId,
        /// Secondary's canonical name for index cleanup during replay.
        secondary_canonical: String,
    },
    
    // Belief operations
    BeliefInsert(Belief),
    BeliefSupersede { old_id: BeliefId, new_id: BeliefId },
    
    // Pattern operations
    PatternInsert(Pattern),
    PatternUpdate(Pattern),
    PatternDelete { id: crate::pattern::PatternId },
    
    // Conflict operations
    ConflictInsert(Conflict),
    ConflictUpdate(Conflict),
    
    // Derivation operations
    DerivationInsert(DerivationRecord),
    
    // Checkpoint marker (all entries before this are persisted to segments)
    Checkpoint { up_to_sequence: u64 },
}

/// Write-Ahead Log for crash recovery.
///
/// Thread-safe via internal mutex.
pub struct WriteAheadLog {
    path: PathBuf,
    writer: Mutex<BufWriter<File>>,
    current_sequence: Mutex<u64>,
    sync_on_write: bool,
}

impl WriteAheadLog {
    /// Open or create a WAL file.
    ///
    /// If the file exists, reads the last sequence number.
    /// If the file doesn't exist, creates it with the header.
    pub fn open(path: &Path, sync_on_write: bool) -> IoResult<Self> {
        let exists = path.exists();
        
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        
        let current_sequence = if exists && file.metadata()?.len() >= 5 {
            // Read existing entries to find last sequence
            Self::find_last_sequence(path)?
        } else {
            // New file, write header
            let mut file = file;
            codec::write_header(&mut file)?;
            if sync_on_write {
                file.sync_all()?;
            }
            0
        };
        
        // Reopen for appending
        let file = OpenOptions::new()
            .append(true)
            .open(path)?;
        
        Ok(Self {
            path: path.to_path_buf(),
            writer: Mutex::new(BufWriter::new(file)),
            current_sequence: Mutex::new(current_sequence),
            sync_on_write,
        })
    }
    
    /// Append an entry to the WAL.
    ///
    /// Returns the sequence number assigned to this entry.
    pub fn append(&self, kind: WalEntryKind) -> IoResult<u64> {
        let mut writer = self.writer.lock().unwrap();
        let mut seq_guard = self.current_sequence.lock().unwrap();

        let candidate = *seq_guard + 1;
        let entry = WalEntry {
            sequence: candidate,
            timestamp: Utc::now(),
            kind,
        };

        let encoded = codec::encode(&entry)?;

        writer.write_all(&encoded)?;
        writer.flush()?;

        if self.sync_on_write {
            writer.get_ref().sync_all()?;
        }

        *seq_guard = candidate;

        Ok(candidate)
    }
    
    /// Iterate over all entries in the WAL.
    ///
    /// Used during recovery to replay mutations.
    pub fn iter(&self) -> IoResult<WalIterator> {
        WalIterator::new(&self.path)
    }
    
    /// Get the current sequence number.
    pub fn current_sequence(&self) -> u64 {
        *self.current_sequence.lock().unwrap()
    }
    
    /// Get the WAL file size in bytes.
    pub fn size_bytes(&self) -> IoResult<u64> {
        Ok(std::fs::metadata(&self.path)?.len())
    }
    
    /// Truncate the WAL after a checkpoint (compaction).
    ///
    /// This is called after segments have been written and the WAL
    /// entries are no longer needed.
    ///
    /// # Safety
    /// Only call this after successfully writing a checkpoint.
    pub fn truncate(&self) -> IoResult<()> {
        {
            // Flush pending writes, then drop the existing writer to release the handle.
            let mut writer = self.writer.lock().unwrap();
            writer.flush()?;
            let placeholder_path = std::env::temp_dir().join("kyroql_wal_truncate_placeholder");
            let placeholder = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(placeholder_path)?;
            let _old = std::mem::replace(&mut *writer, BufWriter::new(placeholder));
            // _old is dropped here, releasing the WAL file handle.
        }

        // Truncate and rewrite header with a fresh handle.
        {
            let mut file = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&self.path)?;

            codec::write_header(&mut file)?;
            if self.sync_on_write {
                file.sync_all()?;
            }
        }

        // Reset sequence to the base of the fresh WAL.
        {
            let mut seq = self.current_sequence.lock().unwrap();
            *seq = 0;
        }

        // Reopen for appending and replace writer in place.
        let file = OpenOptions::new()
            .append(true)
            .open(&self.path)?;

        let mut writer = self.writer.lock().unwrap();
        *writer = BufWriter::new(file);

        Ok(())
    }
    
    fn find_last_sequence(path: &Path) -> IoResult<u64> {
        let mut last_seq = 0;
        
        for entry_result in WalIterator::new(path)? {
            match entry_result {
                Ok(entry) => last_seq = entry.sequence,
                Err(e) => {
                    // Log corruption but continue - we'll replay up to the valid point
                    eprintln!("WAL: corruption detected at sequence {}: {}", last_seq + 1, e);
                    break;
                }
            }
        }
        
        Ok(last_seq)
    }
}

/// Iterator over WAL entries.
pub struct WalIterator {
    reader: BufReader<File>,
    file_size: u64,
}

impl WalIterator {
    fn new(path: &Path) -> IoResult<Self> {
        let file = File::open(path)?;
        let file_size = file.metadata()?.len();
        let mut reader = BufReader::new(file);
        
        // Skip header
        let _version = codec::read_header(&mut reader)?;
        
        Ok(Self { reader, file_size })
    }
    
    fn at_eof(&mut self) -> IoResult<bool> {
        let pos = self.reader.stream_position()?;
        Ok(pos >= self.file_size)
    }
}

impl Iterator for WalIterator {
    type Item = IoResult<WalEntry>;
    
    fn next(&mut self) -> Option<Self::Item> {
        // Check if we've reached EOF using position
        match self.at_eof() {
            Ok(true) => return None,
            Ok(false) => {}
            Err(e) => return Some(Err(e)),
        }
        
        match codec::decode(&mut self.reader) {
            Ok(entry) => Some(Ok(entry)),
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => None,
            Err(e) => Some(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use crate::entity::EntityType;
    
    #[test]
    fn test_wal_append_and_iterate() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");
        
        let wal = WriteAheadLog::open(&wal_path, false).unwrap();
        
        // Append some entries
        let entity = Entity::new("test", EntityType::Concept);
        wal.append(WalEntryKind::EntityInsert(entity.clone())).unwrap();
        wal.append(WalEntryKind::EntityUpdate(entity)).unwrap();
        
        assert_eq!(wal.current_sequence(), 2);
        
        // Drop and reopen to ensure file is properly flushed
        drop(wal);
        
        let wal = WriteAheadLog::open(&wal_path, false).unwrap();
        
        // Iterate and verify
        let entries: Vec<_> = wal.iter().unwrap().collect();
        assert_eq!(entries.len(), 2);
        
        let first = entries[0].as_ref().unwrap();
        assert_eq!(first.sequence, 1);
        assert!(matches!(first.kind, WalEntryKind::EntityInsert(_)));
    }
    
    #[test]
    fn test_wal_persistence_across_reopen() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("persist.wal");
        
        // Write some entries
        {
            let wal = WriteAheadLog::open(&wal_path, true).unwrap();
            let entity = Entity::new("persist", EntityType::Concept);
            wal.append(WalEntryKind::EntityInsert(entity)).unwrap();
        }
        
        // Reopen and verify
        {
            let wal = WriteAheadLog::open(&wal_path, true).unwrap();
            assert_eq!(wal.current_sequence(), 1);
            
            let entries: Vec<_> = wal.iter().unwrap().collect();
            assert_eq!(entries.len(), 1);
        }
    }
}
