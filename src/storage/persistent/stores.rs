//! Persistent store implementations.
//!
//! Each store wraps:
//! - An in-memory index for fast reads
//! - WAL integration for durable writes
//! - Segment manager for long-term storage

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};

use crate::belief::Belief;
use crate::confidence::BeliefId;
use crate::conflict::{Conflict, ConflictId, ConflictStatus};
use crate::derivation::{DerivationId, DerivationRecord};
use crate::entity::{Entity, EntityId};
use crate::error::{ExecutionError, KyroError};
use crate::pattern::{Pattern, PatternId};
use crate::storage::traits::{
    BeliefStore, ConflictStore, DerivationStore, EntityStore, PatternStore, StorageError,
};
use crate::time::TimeRange;

use super::file_lock::FileLock;
use super::segment::SegmentManager;
use super::wal::{WalEntryKind, WriteAheadLog};
use super::PersistentConfig;

/// Aggregate type containing all persistent stores.
///
/// This is the primary entry point for persistent storage.
pub struct PersistentStores {
    /// The database directory.
    pub dir: std::path::PathBuf,
    /// File lock preventing concurrent access.
    _lock: FileLock,
    /// Write-ahead log for durability.
    wal: Arc<WriteAheadLog>,
    /// Segment manager for compacted data.
    _segments: Arc<RwLock<SegmentManager>>,
    /// Configuration.
    _config: PersistentConfig,
    
    // Individual stores
    pub entities: PersistentEntityStore,
    pub beliefs: PersistentBeliefStore,
    pub patterns: PersistentPatternStore,
    pub conflicts: PersistentConflictStore,
    pub derivations: PersistentDerivationStore,
}

impl PersistentStores {
    /// Open or create a persistent database.
    pub fn open(dir: &Path, config: PersistentConfig) -> Result<Self, KyroError> {
        // Create directory if needed
        fs::create_dir_all(dir).map_err(|e| {
            KyroError::Execution(ExecutionError::Storage {
                message: format!("failed to create database directory: {}", e),
            })
        })?;
        
        // Acquire exclusive lock
        let lock = FileLock::acquire(dir).map_err(|e| {
            KyroError::Execution(ExecutionError::Storage {
                message: format!("failed to acquire lock: {}", e),
            })
        })?;
        
        // Open WAL
        let wal_path = dir.join("kyro.wal");
        let wal = Arc::new(WriteAheadLog::open(&wal_path, config.sync_on_write).map_err(|e| {
            KyroError::Execution(ExecutionError::Storage {
                message: format!("failed to open WAL: {}", e),
            })
        })?);
        
        // Open segment manager
        let segments_dir = dir.join("segments");
        let segments = Arc::new(RwLock::new(
            SegmentManager::open(&segments_dir).map_err(|e| {
                KyroError::Execution(ExecutionError::Storage {
                    message: format!("failed to open segments: {}", e),
                })
            })?,
        ));
        
        // Create stores with shared WAL
        let entities = PersistentEntityStore::new(wal.clone());
        let beliefs = PersistentBeliefStore::new(wal.clone());
        let patterns = PersistentPatternStore::new(wal.clone());
        let conflicts = PersistentConflictStore::new(wal.clone());
        let derivations = PersistentDerivationStore::new(wal.clone());
        
        let mut stores = Self {
            dir: dir.to_path_buf(),
            _lock: lock,
            wal,
            _segments: segments,
            _config: config,
            entities,
            beliefs,
            patterns,
            conflicts,
            derivations,
        };
        
        // Load data from segments first (compacted data)
        stores.load_segments()?;
        
        // Replay WAL to restore state (recent changes since last compaction)
        stores.replay_wal()?;
        
        Ok(stores)
    }
    
    /// Load data from all segment files.
    fn load_segments(&mut self) -> Result<(), KyroError> {
        let segments = self._segments.read().unwrap();
        
        if segments.segments().is_empty() {
            return Ok(());
        }
        
        let data = segments.load_all_data().map_err(|e| {
            KyroError::Execution(ExecutionError::Storage {
                message: format!("failed to load segment data: {}", e),
            })
        })?;
        drop(segments);
        
        // Populate in-memory indexes
        *self.entities.index.write().unwrap() = data.entities;
        *self.beliefs.index.write().unwrap() = data.beliefs;
        *self.patterns.index.write().unwrap() = data.patterns;
        *self.conflicts.index.write().unwrap() = data.conflicts;
        *self.derivations.index.write().unwrap() = data.derivations;
        
        Ok(())
    }
    
    /// Replay WAL entries to restore in-memory state.
    fn replay_wal(&mut self) -> Result<(), KyroError> {
        let iter = self.wal.iter().map_err(|e| {
            KyroError::Execution(ExecutionError::Storage {
                message: format!("failed to iterate WAL: {}", e),
            })
        })?;
        
        let mut count = 0;
        for entry_result in iter {
            let entry = entry_result.map_err(|e| {
                KyroError::Execution(ExecutionError::Storage {
                    message: format!("corrupted WAL entry: {}", e),
                })
            })?;
            
            match entry.kind {
                WalEntryKind::EntityInsert(entity) => {
                    self.entities.index.write().unwrap().insert(entity.id, entity);
                }
                WalEntryKind::EntityUpdate(entity) => {
                    self.entities.index.write().unwrap().insert(entity.id, entity);
                }
                WalEntryKind::EntityDelete { id } => {
                    self.entities.index.write().unwrap().remove(&id);
                }
                WalEntryKind::BeliefInsert(belief) => {
                    self.beliefs.index.write().unwrap().insert(belief.id, belief);
                }
                WalEntryKind::BeliefSupersede { old_id, new_id } => {
                    if let Some(belief) = self.beliefs.index.write().unwrap().get_mut(&old_id) {
                        belief.superseded_by = Some(new_id);
                    }
                }
                WalEntryKind::PatternInsert(pattern) => {
                    self.patterns.index.write().unwrap().insert(pattern.id, pattern);
                }
                WalEntryKind::PatternUpdate(pattern) => {
                    self.patterns.index.write().unwrap().insert(pattern.id, pattern);
                }
                WalEntryKind::PatternDelete { id } => {
                    self.patterns.index.write().unwrap().remove(&id);
                }
                WalEntryKind::ConflictInsert(conflict) => {
                    self.conflicts.index.write().unwrap().insert(conflict.id, conflict);
                }
                WalEntryKind::ConflictUpdate(conflict) => {
                    self.conflicts.index.write().unwrap().insert(conflict.id, conflict);
                }
                WalEntryKind::DerivationInsert(record) => {
                    self.derivations.index.write().unwrap().insert(record.id, record);
                }
                WalEntryKind::Checkpoint { .. } => {
                    // Checkpoint markers are informational during replay
                }
            }
            
            count += 1;
        }
        
        if count > 0 {
            eprintln!("Replayed {} WAL entries", count);
        }
        
        Ok(())
    }
    
    /// Compact the WAL into a segment file.
    ///
    /// This operation:
    /// 1. Snapshots all in-memory state
    /// 2. Writes it atomically to a new segment file
    /// 3. Writes a checkpoint marker to the WAL
    /// 4. Truncates the WAL
    ///
    /// This is safe to call at any time - if it fails partway through,
    /// the WAL still contains all data and will be replayed on next open.
    pub fn compact(&mut self) -> Result<CompactionResult, KyroError> {
        use super::segment::SegmentData;
        
        let current_seq = self.wal.current_sequence();
        if current_seq == 0 {
            return Ok(CompactionResult {
                entries_compacted: 0,
                segment_path: None,
                wal_size_before: 0,
                wal_size_after: 0,
            });
        }
        
        let wal_size_before = self.wal.size_bytes().unwrap_or(0);
        
        // Snapshot current state. This assumes callers do not perform concurrent writes while
        // holding &mut self; the public API enforces that, but the interior RwLocks would allow
        // a misuse via borrowed sub-stores. If that misuse happens, WAL replay remains the source
        // of truth and will reconcile any missing entries on reopen.
        let data = SegmentData {
            entities: self.entities.index.read().unwrap().clone(),
            beliefs: self.beliefs.index.read().unwrap().clone(),
            patterns: self.patterns.index.read().unwrap().clone(),
            conflicts: self.conflicts.index.read().unwrap().clone(),
            derivations: self.derivations.index.read().unwrap().clone(),
        };
        
        let entry_count = data.entry_count();
        
        // Write segment atomically
        let mut segments = self._segments.write().unwrap();
        let persisted_seq = segments.persisted_sequence();
        
        let mut writer = segments.create_segment_writer(persisted_seq + 1).map_err(|e| {
            KyroError::Execution(ExecutionError::Storage {
                message: format!("failed to create segment writer: {}", e),
            })
        })?;
        
        if let Err(e) = writer.write_data(&data, current_seq) {
            let _ = writer.abort();
            return Err(KyroError::Execution(ExecutionError::Storage {
                message: format!("failed to write segment data: {}", e),
            }));
        }
        
        let segment = writer.finalize().map_err(|e| {
            KyroError::Execution(ExecutionError::Storage {
                message: format!("failed to finalize segment: {}", e),
            })
        })?;
        
        let segment_path = segment.path().to_path_buf();
        segments.add_segment(segment);
        drop(segments);
        
        // Write checkpoint marker to WAL
        self.wal.append(WalEntryKind::Checkpoint { up_to_sequence: current_seq }).map_err(|e| {
            KyroError::Execution(ExecutionError::Storage {
                message: format!("failed to write checkpoint: {}", e),
            })
        })?;
        
        // Truncate WAL
        self.wal.truncate().map_err(|e| {
            KyroError::Execution(ExecutionError::Storage {
                message: format!("failed to truncate WAL: {}", e),
            })
        })?;
        
        let wal_size_after = self.wal.size_bytes().unwrap_or(0);
        
        Ok(CompactionResult {
            entries_compacted: entry_count,
            segment_path: Some(segment_path),
            wal_size_before,
            wal_size_after,
        })
    }
    
    /// Get the current WAL size in bytes.
    pub fn wal_size(&self) -> u64 {
        self.wal.size_bytes().unwrap_or(0)
    }
    
    /// Get the number of segments.
    pub fn segment_count(&self) -> usize {
        self._segments.read().unwrap().segments().len()
    }
}

/// Result of a compaction operation.
#[derive(Debug, Clone)]
pub struct CompactionResult {
    /// Number of entries compacted.
    pub entries_compacted: u64,
    /// Path to the new segment file (if created).
    pub segment_path: Option<std::path::PathBuf>,
    /// WAL size before compaction.
    pub wal_size_before: u64,
    /// WAL size after compaction.
    pub wal_size_after: u64,
}

// --- Entity Store ---

pub struct PersistentEntityStore {
    wal: Arc<WriteAheadLog>,
    index: RwLock<HashMap<EntityId, Entity>>,
}

impl PersistentEntityStore {
    fn new(wal: Arc<WriteAheadLog>) -> Self {
        Self {
            wal,
            index: RwLock::new(HashMap::new()),
        }
    }
}

impl EntityStore for PersistentEntityStore {
    fn insert(&self, entity: Entity) -> Result<(), StorageError> {
        let mut index = self.index.write().unwrap();
        
        if index.contains_key(&entity.id) {
            return Err(StorageError::DuplicateKey(format!("entity:{}", entity.id)));
        }
        
        self.wal.append(WalEntryKind::EntityInsert(entity.clone()))
            .map_err(|e| StorageError::BackendError(format!("WAL write failed: {}", e)))?;
        
        index.insert(entity.id, entity);
        Ok(())
    }
    
    fn get(&self, id: EntityId) -> Result<Option<Entity>, StorageError> {
        Ok(self.index.read().unwrap().get(&id).cloned())
    }
    
    fn update(&self, entity: Entity) -> Result<(), StorageError> {
        let mut index = self.index.write().unwrap();
        
        if !index.contains_key(&entity.id) {
            return Err(StorageError::EntityNotFound(entity.id));
        }
        
        self.wal.append(WalEntryKind::EntityUpdate(entity.clone()))
            .map_err(|e| StorageError::BackendError(format!("WAL write failed: {}", e)))?;
        
        index.insert(entity.id, entity);
        Ok(())
    }
    
    fn delete(&self, id: EntityId) -> Result<(), StorageError> {
        let mut index = self.index.write().unwrap();
        
        if !index.contains_key(&id) {
            return Err(StorageError::EntityNotFound(id));
        }
        
        self.wal.append(WalEntryKind::EntityDelete { id })
            .map_err(|e| StorageError::BackendError(format!("WAL write failed: {}", e)))?;
        
        index.remove(&id);
        Ok(())
    }
    
    fn find_by_name(&self, name: &str) -> Result<Vec<Entity>, StorageError> {
        let index = self.index.read().unwrap();
        Ok(index.values()
            .filter(|e| e.canonical_name == name)
            .cloned()
            .collect())
    }
    
    fn find_by_name_fuzzy(&self, query: &str, limit: usize) -> Result<Vec<Entity>, StorageError> {
        let index = self.index.read().unwrap();
        let query_lower = query.to_lowercase();
        
        let mut matches: Vec<_> = index.values()
            .filter(|e| {
                e.canonical_name.to_lowercase().contains(&query_lower)
                    || e.aliases.iter().any(|a| a.to_lowercase().contains(&query_lower))
            })
            .cloned()
            .collect();
        
        matches.truncate(limit);
        Ok(matches)
    }
    
    fn find_by_embedding(&self, embedding: &[f32], limit: usize) -> Result<Vec<(Entity, f32)>, StorageError> {
        let index = self.index.read().unwrap();
        
        let mut scored: Vec<_> = index.values()
            .filter_map(|e| {
                e.embedding.as_ref().map(|emb| {
                    let score = cosine_similarity(embedding, emb);
                    (e.clone(), score)
                })
            })
            .collect();
        
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        Ok(scored)
    }
    
    fn merge(&self, _primary: EntityId, _secondary: EntityId) -> Result<Entity, StorageError> {
        // TODO: Implement entity merging
        Err(StorageError::BackendError("entity merge not yet implemented".to_string()))
    }
    
    fn get_at_version(&self, _id: EntityId, _version: u64) -> Result<Option<Entity>, StorageError> {
        // TODO: Implement versioning
        Err(StorageError::BackendError("versioning not yet implemented".to_string()))
    }
    
    fn list_versions(&self, _id: EntityId) -> Result<Vec<Entity>, StorageError> {
        // TODO: Implement versioning
        Err(StorageError::BackendError("versioning not yet implemented".to_string()))
    }
}

// --- Belief Store ---

pub struct PersistentBeliefStore {
    wal: Arc<WriteAheadLog>,
    index: RwLock<HashMap<BeliefId, Belief>>,
}

impl PersistentBeliefStore {
    fn new(wal: Arc<WriteAheadLog>) -> Self {
        Self {
            wal,
            index: RwLock::new(HashMap::new()),
        }
    }
}

impl BeliefStore for PersistentBeliefStore {
    fn insert(&self, belief: Belief) -> Result<(), StorageError> {
        let mut index = self.index.write().unwrap();
        
        if index.contains_key(&belief.id) {
            return Err(StorageError::DuplicateKey(format!("belief:{}", belief.id)));
        }
        
        self.wal.append(WalEntryKind::BeliefInsert(belief.clone()))
            .map_err(|e| StorageError::BackendError(format!("WAL write failed: {}", e)))?;
        
        index.insert(belief.id, belief);
        Ok(())
    }
    
    fn get(&self, id: BeliefId) -> Result<Option<Belief>, StorageError> {
        Ok(self.index.read().unwrap().get(&id).cloned())
    }
    
    fn supersede(&self, old_id: BeliefId, new_id: BeliefId) -> Result<(), StorageError> {
        let mut index = self.index.write().unwrap();
        
        if !index.contains_key(&old_id) {
            return Err(StorageError::BeliefNotFound(old_id));
        }
        
        self.wal.append(WalEntryKind::BeliefSupersede { old_id, new_id })
            .map_err(|e| StorageError::BackendError(format!("WAL write failed: {}", e)))?;
        
        if let Some(belief) = index.get_mut(&old_id) {
            belief.superseded_by = Some(new_id);
        }
        Ok(())
    }
    
    fn find_by_entity_predicate(&self, entity_id: EntityId, predicate: &str) -> Result<Vec<Belief>, StorageError> {
        let index = self.index.read().unwrap();
        Ok(index.values()
            .filter(|b| b.subject == entity_id && b.predicate == predicate)
            .cloned()
            .collect())
    }
    
    fn find_as_of(&self, entity_id: EntityId, predicate: &str, as_of: DateTime<Utc>) -> Result<Vec<Belief>, StorageError> {
        let index = self.index.read().unwrap();
        Ok(index.values()
            .filter(|b| {
                b.subject == entity_id
                    && b.predicate == predicate
                    && b.valid_time.contains(as_of)
            })
            .cloned()
            .collect())
    }
    
    fn find_by_time_range(&self, range: &TimeRange) -> Result<Vec<Belief>, StorageError> {
        let index = self.index.read().unwrap();
        Ok(index.values()
            .filter(|b| b.valid_time.overlaps(range))
            .cloned()
            .collect())
    }
    
    fn find_by_embedding(&self, embedding: &[f32], limit: usize, min_confidence: Option<f32>) -> Result<Vec<(Belief, f32)>, StorageError> {
        let index = self.index.read().unwrap();
        let min_conf = min_confidence.unwrap_or(0.0);
        
        let mut scored: Vec<_> = index.values()
            .filter(|b| b.confidence.value() >= min_conf)
            .filter_map(|b| {
                b.embedding.as_ref().map(|emb| {
                    let score = cosine_similarity(embedding, emb);
                    (b.clone(), score)
                })
            })
            .collect();
        
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        Ok(scored)
    }
    
    fn count_by_entity(&self, entity_id: EntityId) -> Result<usize, StorageError> {
        let index = self.index.read().unwrap();
        Ok(index.values().filter(|b| b.subject == entity_id).count())
    }
}

// --- Pattern Store ---

pub struct PersistentPatternStore {
    wal: Arc<WriteAheadLog>,
    index: RwLock<HashMap<PatternId, Pattern>>,
}

impl PersistentPatternStore {
    fn new(wal: Arc<WriteAheadLog>) -> Self {
        Self {
            wal,
            index: RwLock::new(HashMap::new()),
        }
    }
}

impl PatternStore for PersistentPatternStore {
    fn insert(&self, pattern: Pattern) -> Result<(), StorageError> {
        let mut index = self.index.write().unwrap();
        
        if index.contains_key(&pattern.id) {
            return Err(StorageError::DuplicateKey(format!("pattern:{}", pattern.id)));
        }
        
        self.wal.append(WalEntryKind::PatternInsert(pattern.clone()))
            .map_err(|e| StorageError::BackendError(format!("WAL write failed: {}", e)))?;
        
        index.insert(pattern.id, pattern);
        Ok(())
    }
    
    fn get(&self, id: PatternId) -> Result<Option<Pattern>, StorageError> {
        Ok(self.index.read().unwrap().get(&id).cloned())
    }
    
    fn update(&self, pattern: Pattern) -> Result<(), StorageError> {
        let mut index = self.index.write().unwrap();
        
        if !index.contains_key(&pattern.id) {
            return Err(StorageError::PatternNotFound(pattern.id));
        }
        
        self.wal.append(WalEntryKind::PatternUpdate(pattern.clone()))
            .map_err(|e| StorageError::BackendError(format!("WAL write failed: {}", e)))?;
        
        index.insert(pattern.id, pattern);
        Ok(())
    }
    
    fn delete(&self, id: PatternId) -> Result<(), StorageError> {
        let mut index = self.index.write().unwrap();
        
        if !index.contains_key(&id) {
            return Err(StorageError::PatternNotFound(id));
        }
        
        self.wal.append(WalEntryKind::PatternDelete { id })
            .map_err(|e| StorageError::BackendError(format!("WAL write failed: {}", e)))?;
        
        index.remove(&id);
        Ok(())
    }
    
    fn find_by_predicate(&self, predicate: &str) -> Result<Vec<Pattern>, StorageError> {
        let index = self.index.read().unwrap();
        Ok(index.values()
            .filter(|p| p.rule.matches_predicate(predicate))
            .cloned()
            .collect())
    }
    
    fn find_active(&self) -> Result<Vec<Pattern>, StorageError> {
        let now = Utc::now();
        let index = self.index.read().unwrap();
        Ok(index.values()
            .filter(|p| p.valid_time.contains(now))
            .cloned()
            .collect())
    }
}

// --- Conflict Store ---

pub struct PersistentConflictStore {
    wal: Arc<WriteAheadLog>,
    index: RwLock<HashMap<ConflictId, Conflict>>,
}

impl PersistentConflictStore {
    fn new(wal: Arc<WriteAheadLog>) -> Self {
        Self {
            wal,
            index: RwLock::new(HashMap::new()),
        }
    }
}

impl ConflictStore for PersistentConflictStore {
    fn insert(&self, conflict: Conflict) -> Result<(), StorageError> {
        let mut index = self.index.write().unwrap();
        
        if index.contains_key(&conflict.id) {
            return Err(StorageError::DuplicateKey(format!("conflict:{}", conflict.id)));
        }
        
        self.wal.append(WalEntryKind::ConflictInsert(conflict.clone()))
            .map_err(|e| StorageError::BackendError(format!("WAL write failed: {}", e)))?;
        
        index.insert(conflict.id, conflict);
        Ok(())
    }
    
    fn get(&self, id: ConflictId) -> Result<Option<Conflict>, StorageError> {
        Ok(self.index.read().unwrap().get(&id).cloned())
    }
    
    fn update(&self, conflict: Conflict) -> Result<(), StorageError> {
        let mut index = self.index.write().unwrap();
        
        if !index.contains_key(&conflict.id) {
            return Err(StorageError::ConflictNotFound(conflict.id));
        }
        
        self.wal.append(WalEntryKind::ConflictUpdate(conflict.clone()))
            .map_err(|e| StorageError::BackendError(format!("WAL write failed: {}", e)))?;
        
        index.insert(conflict.id, conflict);
        Ok(())
    }
    
    fn find_by_belief(&self, belief_id: BeliefId) -> Result<Vec<Conflict>, StorageError> {
        let index = self.index.read().unwrap();
        Ok(index.values()
            .filter(|c| c.involves_belief(belief_id))
            .cloned()
            .collect())
    }
    
    fn find_open(&self) -> Result<Vec<Conflict>, StorageError> {
        let index = self.index.read().unwrap();
        Ok(index.values()
            .filter(|c| c.status == ConflictStatus::Open)
            .cloned()
            .collect())
    }
}

// --- Derivation Store ---

pub struct PersistentDerivationStore {
    wal: Arc<WriteAheadLog>,
    index: RwLock<HashMap<DerivationId, DerivationRecord>>,
}

impl PersistentDerivationStore {
    fn new(wal: Arc<WriteAheadLog>) -> Self {
        Self {
            wal,
            index: RwLock::new(HashMap::new()),
        }
    }
}

impl DerivationStore for PersistentDerivationStore {
    fn insert(&self, record: DerivationRecord) -> Result<(), StorageError> {
        let mut index = self.index.write().unwrap();
        
        if index.contains_key(&record.id) {
            return Err(StorageError::DuplicateKey(format!("derivation:{}", record.id)));
        }
        
        self.wal.append(WalEntryKind::DerivationInsert(record.clone()))
            .map_err(|e| StorageError::BackendError(format!("WAL write failed: {}", e)))?;
        
        index.insert(record.id, record);
        Ok(())
    }
    
    fn get(&self, id: DerivationId) -> Result<Option<DerivationRecord>, StorageError> {
        Ok(self.index.read().unwrap().get(&id).cloned())
    }
    
    fn find_by_premise(&self, premise_id: BeliefId) -> Result<Vec<DerivationRecord>, StorageError> {
        let index = self.index.read().unwrap();
        Ok(index.values()
            .filter(|r| r.premise_ids.contains(&premise_id))
            .cloned()
            .collect())
    }
    
    fn find_by_derived_belief(&self, derived_belief_id: BeliefId) -> Result<Vec<DerivationRecord>, StorageError> {
        let index = self.index.read().unwrap();
        Ok(index.values()
            .filter(|r| r.derived_belief_id == Some(derived_belief_id))
            .cloned()
            .collect())
    }
}

// --- Utility Functions ---

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    
    dot / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityType;
    use tempfile::tempdir;
    
    #[test]
    fn test_persistent_stores_open_and_write() {
        let dir = tempdir().unwrap();
        
        {
            let stores = PersistentStores::open(dir.path(), PersistentConfig::default()).unwrap();
            let entity = Entity::new("test", EntityType::Concept);
            stores.entities.insert(entity).unwrap();
        }
        
        // Reopen and verify persistence
        {
            let stores = PersistentStores::open(dir.path(), PersistentConfig::default()).unwrap();
            let entities: Vec<_> = stores.entities.find_by_name("test").unwrap();
            assert_eq!(entities.len(), 1);
        }
    }
    
    #[test]
    fn test_compaction_creates_segment() {
        let dir = tempdir().unwrap();
        
        let mut stores = PersistentStores::open(dir.path(), PersistentConfig::default()).unwrap();
        
        // Insert some data
        for i in 0..10 {
            let entity = Entity::new(format!("entity_{}", i), EntityType::Concept);
            stores.entities.insert(entity).unwrap();
        }
        
        let wal_size_before = stores.wal_size();
        assert!(wal_size_before > 0);
        assert_eq!(stores.segment_count(), 0);
        
        // Compact
        let result = stores.compact().unwrap();
        
        assert_eq!(result.entries_compacted, 10);
        assert!(result.segment_path.is_some());
        assert!(result.wal_size_after < result.wal_size_before);
        assert_eq!(stores.segment_count(), 1);
    }
    
    #[test]
    fn test_compaction_empty_wal() {
        let dir = tempdir().unwrap();
        
        let mut stores = PersistentStores::open(dir.path(), PersistentConfig::default()).unwrap();
        
        // Compact empty database
        let result = stores.compact().unwrap();
        
        assert_eq!(result.entries_compacted, 0);
        assert!(result.segment_path.is_none());
    }
    
    #[test]
    fn test_data_survives_compaction_and_reopen() {
        let dir = tempdir().unwrap();
        let entity_id;
        
        // Write, compact, close
        {
            let mut stores = PersistentStores::open(dir.path(), PersistentConfig::default()).unwrap();
            let entity = Entity::new("survive_compaction", EntityType::Concept);
            entity_id = entity.id;
            stores.entities.insert(entity).unwrap();
            stores.compact().unwrap();
        }
        
        // Reopen and verify data still exists
        {
            let stores = PersistentStores::open(dir.path(), PersistentConfig::default()).unwrap();
            let entity = stores.entities.get(entity_id).unwrap();
            assert!(entity.is_some());
            assert_eq!(entity.unwrap().canonical_name, "survive_compaction");
        }
    }
}
