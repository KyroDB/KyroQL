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

fn lock_err(context: &'static str) -> StorageError {
    StorageError::BackendError(format!("poisoned lock: {context}"))
}

fn normalize_key(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

fn resolve_canonical_id(index: &EntityIndex, id: EntityId) -> Result<EntityId, StorageError> {
    let mut current = id;
    for _ in 0..128 {
        if let Some(next) = index.merged_into.get(&current).copied() {
            if next == current {
                return Err(StorageError::BackendError(
                    "entity merge map contains a self-cycle".to_string(),
                ));
            }
            current = next;
            continue;
        }
        return Ok(current);
    }

    Err(StorageError::BackendError(
        "entity merge map resolution exceeded hop limit".to_string(),
    ))
}

fn record_entity_version(
    index: &mut EntityIndex,
    entity: &Entity,
    context: &'static str,
) -> Result<(), StorageError> {
    let versions = index.versions.entry(entity.id).or_default();
    if versions.contains_key(&entity.version) {
        return Err(StorageError::BackendError(format!(
            "duplicate entity version ({context}): id={} version={}",
            entity.id, entity.version
        )));
    }
    versions.insert(entity.version, entity.clone());
    Ok(())
}

fn merge_metadata(primary: &serde_json::Value, secondary: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;

    match (primary, secondary) {
        (Value::Null, other) => other.clone(),
        (Value::Object(a), Value::Object(b)) => {
            let mut out = a.clone();
            for (k, v) in b {
                out.entry(k.clone()).or_insert_with(|| v.clone());
            }
            Value::Object(out)
        }
        (a, _) => a.clone(),
    }
}

fn merge_embeddings(a: &[f32], b: &[f32]) -> Result<Vec<f32>, StorageError> {
    if a.len() != b.len() {
        return Err(StorageError::BackendError(format!(
            "cannot merge embeddings with different dimensions: {} vs {}",
            a.len(),
            b.len()
        )));
    }
    if a.is_empty() {
        return Ok(Vec::new());
    }

    let mut out = Vec::with_capacity(a.len());
    let mut norm2 = 0.0f64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let v = (f64::from(x) + f64::from(y)) * 0.5;
        norm2 += v * v;
        #[allow(clippy::cast_possible_truncation)]
        out.push(v as f32);
    }

    if norm2 <= 0.0 {
        return Ok(out);
    }
    let inv = 1.0 / norm2.sqrt();
    for v in &mut out {
        *v = (*v as f64 * inv) as f32;
    }
    Ok(out)
}

fn validate_embedding_dim(
    expected: Option<usize>,
    embedding: &[f32],
    context: &'static str,
) -> Result<(), StorageError> {
    if embedding.is_empty() {
        return Err(StorageError::BackendError(format!(
            "embedding dimension must be non-zero ({context})"
        )));
    }

    if let Some(exp) = expected {
        if exp != embedding.len() {
            return Err(StorageError::BackendError(format!(
                "embedding dimension mismatch ({context}): expected={exp} actual={}",
                embedding.len()
            )));
        }
    }

    Ok(())
}

fn apply_embedding_dim(
    expected: &mut Option<usize>,
    embedding: Option<&Vec<f32>>,
    context: &'static str,
) -> Result<(), StorageError> {
    let Some(emb) = embedding else {
        return Ok(());
    };
    validate_embedding_dim(*expected, emb, context)?;
    if expected.is_none() {
        *expected = Some(emb.len());
    }
    Ok(())
}

type EntityIndex = super::segment::EntityIndex;

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
        *self.beliefs.index.write().unwrap() = BeliefIndex::from_map(data.beliefs);
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
                WalEntryKind::EntityInsert(_)
                | WalEntryKind::EntityUpdate(_)
                | WalEntryKind::EntityDelete { .. }
                | WalEntryKind::EntityMerge { .. } => {
                    self.entities.apply_wal(&entry.kind).map_err(|e| {
                        KyroError::Execution(ExecutionError::Storage {
                            message: format!("failed to apply WAL entity entry: {e}"),
                        })
                    })?;
                }
                WalEntryKind::BeliefInsert(belief) => {
                    self.beliefs
                        .index
                        .write()
                        .map_err(|_| KyroError::Execution(ExecutionError::Storage {
                            message: "poisoned lock: belief.wal".to_string(),
                        }))?
                        .insert(belief);
                }
                WalEntryKind::BeliefSupersede { old_id, new_id } => {
                    if let Some(belief) = self
                        .beliefs
                        .index
                        .write()
                        .map_err(|_| KyroError::Execution(ExecutionError::Storage {
                            message: "poisoned lock: belief.wal".to_string(),
                        }))?
                        .by_id
                        .get_mut(&old_id)
                    {
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
            beliefs: self.beliefs.index.read().unwrap().by_id.clone(),
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
    index: RwLock<EntityIndex>,
}

impl PersistentEntityStore {
    fn new(wal: Arc<WriteAheadLog>) -> Self {
        Self {
            wal,
            index: RwLock::new(EntityIndex::default()),
        }
    }

    fn insert_internal(&self, entity: Entity, emit_wal: bool) -> Result<(), StorageError> {
        let mut index = self.index.write().map_err(|_| lock_err("entity.insert"))?;

        if index.by_id.contains_key(&entity.id) || index.merged_into.contains_key(&entity.id) {
            return Err(StorageError::DuplicateKey(entity.id.to_string()));
        }

        if let Some(emb) = entity.embedding.as_ref() {
            validate_embedding_dim(index.embedding_dim, emb, "entity.insert")?;
        }

        if index
            .versions
            .get(&entity.id)
            .map_or(false, |m| m.contains_key(&entity.version))
        {
            return Err(StorageError::BackendError(format!(
                "entity version already exists (entity.insert): id={} version={}",
                entity.id, entity.version
            )));
        }

        if emit_wal {
            self
                .wal
                .append(WalEntryKind::EntityInsert(entity.clone()))
                .map_err(|e| StorageError::BackendError(format!("WAL write failed: {e}")))?;
        }

        apply_embedding_dim(&mut index.embedding_dim, entity.embedding.as_ref(), "entity.insert")?;
        record_entity_version(&mut index, &entity, "entity.insert")?;

        let name_key = normalize_key(&entity.canonical_name);
        index.by_name.entry(name_key).or_default().insert(entity.id);
        index.by_id.insert(entity.id, entity);
        Ok(())
    }

    fn update_internal(&self, entity: Entity, emit_wal: bool) -> Result<(), StorageError> {
        let mut index = self.index.write().map_err(|_| lock_err("entity.update"))?;

        let canonical = resolve_canonical_id(&index, entity.id)?;
        if canonical != entity.id {
            return Err(StorageError::BackendError(
                "cannot update an entity that has been merged".to_string(),
            ));
        }

        let prev = index
            .by_id
            .get(&entity.id)
            .cloned()
            .ok_or(StorageError::EntityNotFound(entity.id))?;

        if entity.version <= prev.version {
            return Err(StorageError::BackendError(format!(
                "entity version must increase on update: id={} prev={} new={}",
                entity.id, prev.version, entity.version
            )));
        }

        if let Some(emb) = entity.embedding.as_ref() {
            validate_embedding_dim(index.embedding_dim, emb, "entity.update")?;
        }

        if index
            .versions
            .get(&entity.id)
            .map_or(false, |m| m.contains_key(&entity.version))
        {
            return Err(StorageError::BackendError(format!(
                "entity version already exists (entity.update): id={} version={}",
                entity.id, entity.version
            )));
        }

        if emit_wal {
            self
                .wal
                .append(WalEntryKind::EntityUpdate(entity.clone()))
                .map_err(|e| StorageError::BackendError(format!("WAL write failed: {e}")))?;
        }

        apply_embedding_dim(&mut index.embedding_dim, entity.embedding.as_ref(), "entity.update")?;

        let prev_key = normalize_key(&prev.canonical_name);
        let new_key = normalize_key(&entity.canonical_name);
        if prev_key != new_key {
            if let Some(set) = index.by_name.get_mut(&prev_key) {
                set.remove(&entity.id);
                if set.is_empty() {
                    index.by_name.remove(&prev_key);
                }
            }
            index.by_name.entry(new_key).or_default().insert(entity.id);
        }

        record_entity_version(&mut index, &entity, "entity.update")?;
        index.by_id.insert(entity.id, entity);
        Ok(())
    }

    fn delete_internal(&self, id: EntityId, emit_wal: bool) -> Result<(), StorageError> {
        let mut index = self.index.write().map_err(|_| lock_err("entity.delete"))?;

        let canonical = resolve_canonical_id(&index, id)?;
        if canonical != id {
            return Err(StorageError::BackendError(
                "cannot delete an entity that has been merged".to_string(),
            ));
        }

        if index
            .merged_from
            .get(&id)
            .map_or(false, |s| !s.is_empty())
        {
            return Err(StorageError::BackendError(
                "cannot delete an entity that has other entities merged into it".to_string(),
            ));
        }

        if emit_wal {
            self
                .wal
                .append(WalEntryKind::EntityDelete { id })
                .map_err(|e| StorageError::BackendError(format!("WAL write failed: {e}")))?;
        }

        let prev = index
            .by_id
            .remove(&id)
            .ok_or(StorageError::EntityNotFound(id))?;

        let prev_key = normalize_key(&prev.canonical_name);
        if let Some(set) = index.by_name.get_mut(&prev_key) {
            set.remove(&id);
            if set.is_empty() {
                index.by_name.remove(&prev_key);
            }
        }

        Ok(())
    }

    fn merge_internal(
        &self,
        merged: Entity,
        secondary_id: EntityId,
        secondary_canonical: String,
        emit_wal: bool,
    ) -> Result<Entity, StorageError> {
        let mut index = self.index.write().map_err(|_| lock_err("entity.merge"))?;

        let primary_canonical = resolve_canonical_id(&index, merged.id)?;
        let secondary_canonical_id = resolve_canonical_id(&index, secondary_id)?;

        if primary_canonical == secondary_canonical_id {
            return Err(StorageError::BackendError(
                "cannot merge: both IDs resolve to the same canonical entity".to_string(),
            ));
        }

        let prev_primary = index
            .by_id
            .get(&primary_canonical)
            .cloned()
            .ok_or(StorageError::EntityNotFound(primary_canonical))?;

        if !index.by_id.contains_key(&secondary_canonical_id) {
            return Err(StorageError::EntityNotFound(secondary_canonical_id));
        }

        if merged.version <= prev_primary.version {
            return Err(StorageError::BackendError(format!(
                "entity version must increase on merge: id={} prev={} new={}",
                merged.id, prev_primary.version, merged.version
            )));
        }

        if let Some(emb) = merged.embedding.as_ref() {
            validate_embedding_dim(index.embedding_dim, emb, "entity.merge")?;
        }

        if index
            .versions
            .get(&merged.id)
            .map_or(false, |m| m.contains_key(&merged.version))
        {
            return Err(StorageError::BackendError(format!(
                "entity version already exists (entity.merge): id={} version={}",
                merged.id, merged.version
            )));
        }

        if emit_wal {
            self
                .wal
                .append(WalEntryKind::EntityMerge {
                    merged: merged.clone(),
                    secondary_id: secondary_canonical_id,
                    secondary_canonical: secondary_canonical.clone(),
                })
                .map_err(|e| StorageError::BackendError(format!("WAL write failed: {e}")))?;
        }

        apply_embedding_dim(&mut index.embedding_dim, merged.embedding.as_ref(), "entity.merge")?;

        let prev_key = normalize_key(&prev_primary.canonical_name);
        let new_key = normalize_key(&merged.canonical_name);
        if prev_key != new_key {
            if let Some(set) = index.by_name.get_mut(&prev_key) {
                set.remove(&merged.id);
                if set.is_empty() {
                    index.by_name.remove(&prev_key);
                }
            }
            index.by_name.entry(new_key).or_default().insert(merged.id);
        }

        record_entity_version(&mut index, &merged, "entity.merge")?;
        index.by_id.insert(primary_canonical, merged.clone());

        let secondary_key = normalize_key(&secondary_canonical);
        if let Some(set) = index.by_name.get_mut(&secondary_key) {
            set.remove(&secondary_canonical_id);
            if set.is_empty() {
                index.by_name.remove(&secondary_key);
            }
        }
        index.by_id.remove(&secondary_canonical_id);

        index
            .merged_into
            .insert(secondary_canonical_id, primary_canonical);
        index
            .merged_from
            .entry(primary_canonical)
            .or_default()
            .insert(secondary_canonical_id);

        Ok(merged)
    }

    fn apply_wal(&self, kind: &WalEntryKind) -> Result<(), StorageError> {
        match kind {
            WalEntryKind::EntityInsert(entity) => self.insert_internal(entity.clone(), false),
            WalEntryKind::EntityUpdate(entity) => self.update_internal(entity.clone(), false),
            WalEntryKind::EntityDelete { id } => self.delete_internal(*id, false),
            WalEntryKind::EntityMerge {
                merged,
                secondary_id,
                secondary_canonical,
            } => self.merge_internal(merged.clone(), *secondary_id, secondary_canonical.clone(), false).map(|_| ()),
            _ => Ok(()),
        }
    }
}

impl EntityStore for PersistentEntityStore {
    fn insert(&self, entity: Entity) -> Result<(), StorageError> {
        self.insert_internal(entity, true)
    }
    
    fn get(&self, id: EntityId) -> Result<Option<Entity>, StorageError> {
        let index = self.index.read().map_err(|_| lock_err("entity.get"))?;
        let canonical = resolve_canonical_id(&index, id)?;
        Ok(index.by_id.get(&canonical).cloned())
    }
    
    fn update(&self, entity: Entity) -> Result<(), StorageError> {
        self.update_internal(entity, true)
    }
    
    fn delete(&self, id: EntityId) -> Result<(), StorageError> {
        self.delete_internal(id, true)
    }
    
    fn find_by_name(&self, name: &str) -> Result<Vec<Entity>, StorageError> {
        let name_key = normalize_key(name);
        let index = self.index.read().map_err(|_| lock_err("entity.find_by_name"))?;
        let Some(ids) = index.by_name.get(&name_key) else {
            return Ok(Vec::new());
        };

        let mut results: Vec<Entity> = ids
            .iter()
            .filter_map(|id| index.by_id.get(id).cloned())
            .collect();
        results.sort_by(|a, b| a.canonical_name.cmp(&b.canonical_name));
        Ok(results)
    }
    
    fn find_by_name_fuzzy(&self, query: &str, limit: usize) -> Result<Vec<Entity>, StorageError> {
        let query_key = normalize_key(query);
        if query_key.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let index = self
            .index
            .read()
            .map_err(|_| lock_err("entity.find_by_name_fuzzy"))?;

        let mut scored: Vec<(u8, Entity)> = Vec::new();
        for entity in index.by_id.values() {
            let canonical = normalize_key(&entity.canonical_name);
            let mut score = 0u8;
            if canonical.starts_with(&query_key) {
                score = score.max(3);
            } else if canonical.contains(&query_key) {
                score = score.max(2);
            }

            for alias in &entity.aliases {
                let alias_key = normalize_key(alias);
                if alias_key.starts_with(&query_key) {
                    score = score.max(2);
                } else if alias_key.contains(&query_key) {
                    score = score.max(1);
                }
            }

            if score > 0 {
                scored.push((score, entity.clone()));
            }
        }

        scored.sort_by(|(sa, ea), (sb, eb)| {
            sb.cmp(sa)
                .then_with(|| ea.canonical_name.cmp(&eb.canonical_name))
                .then_with(|| ea.id.to_string().cmp(&eb.id.to_string()))
        });

        Ok(scored
            .into_iter()
            .take(limit)
            .map(|(_, e)| e)
            .collect())
    }
    
    fn find_by_embedding(&self, embedding: &[f32], limit: usize) -> Result<Vec<(Entity, f32)>, StorageError> {
        if embedding.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let index = self
            .index
            .read()
            .map_err(|_| lock_err("entity.find_by_embedding"))?;

        if let Some(exp) = index.embedding_dim {
            if exp != embedding.len() {
                return Err(StorageError::BackendError(format!(
                    "embedding dimension mismatch (entity.find_by_embedding): expected={exp} actual={}",
                    embedding.len()
                )));
            }
        }

        let mut scored: Vec<(Entity, f32)> = Vec::new();
        for entity in index.by_id.values() {
            let Some(stored) = entity.embedding.as_ref() else {
                continue;
            };
            let score = cosine_similarity(embedding, stored)?;
            if score > 0.0 {
                scored.push((entity.clone(), score));
            }
        }

        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(limit);
        Ok(scored)
    }
    
    fn merge(&self, primary: EntityId, secondary: EntityId) -> Result<Entity, StorageError> {
        if primary == secondary {
            return Err(StorageError::BackendError(
                "cannot merge an entity into itself".to_string(),
            ));
        }

        let mut index = self.index.write().map_err(|_| lock_err("entity.merge"))?;

        let primary_canonical = resolve_canonical_id(&index, primary)?;
        let secondary_canonical = resolve_canonical_id(&index, secondary)?;
        if primary_canonical == secondary_canonical {
            return Err(StorageError::BackendError(
                "cannot merge: both IDs resolve to the same canonical entity".to_string(),
            ));
        }

        let mut primary_entity = index
            .by_id
            .get(&primary_canonical)
            .cloned()
            .ok_or(StorageError::EntityNotFound(primary_canonical))?;
        let secondary_entity = index
            .by_id
            .get(&secondary_canonical)
            .cloned()
            .ok_or(StorageError::EntityNotFound(secondary_canonical))?;

        let secondary_names = std::iter::once(secondary_entity.canonical_name.clone())
            .chain(secondary_entity.aliases.clone());
        for name in secondary_names {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            if name.eq_ignore_ascii_case(&primary_entity.canonical_name) {
                continue;
            }
            if primary_entity
                .aliases
                .iter()
                .any(|a| a.eq_ignore_ascii_case(name))
            {
                continue;
            }
            primary_entity.aliases.push(name.to_string());
        }

        primary_entity.metadata =
            merge_metadata(&primary_entity.metadata, &secondary_entity.metadata);

        primary_entity.embedding = match (
            primary_entity.embedding.as_ref(),
            secondary_entity.embedding.as_ref(),
        ) {
            (Some(a), Some(b)) => Some(merge_embeddings(a, b)?),
            (Some(a), None) => Some(a.clone()),
            (None, Some(b)) => Some(b.clone()),
            (None, None) => None,
        };

        let now = Utc::now();
        primary_entity.updated_at = now;
        primary_entity.version = primary_entity
            .version
            .checked_add(1)
            .ok_or_else(|| StorageError::BackendError("entity version overflow".to_string()))?;

        if let Some(emb) = primary_entity.embedding.as_ref() {
            validate_embedding_dim(index.embedding_dim, emb, "entity.merge")?;
        }

        if index
            .versions
            .get(&primary_entity.id)
            .map_or(false, |m| m.contains_key(&primary_entity.version))
        {
            return Err(StorageError::BackendError(format!(
                "entity version already exists (entity.merge): id={} version={}",
                primary_entity.id, primary_entity.version
            )));
        }

        self
            .wal
            .append(WalEntryKind::EntityMerge {
                merged: primary_entity.clone(),
                secondary_id: secondary_canonical,
                secondary_canonical: secondary_entity.canonical_name.clone(),
            })
            .map_err(|e| StorageError::BackendError(format!("WAL write failed: {e}")))?;

        apply_embedding_dim(&mut index.embedding_dim, primary_entity.embedding.as_ref(), "entity.merge")?;

        let prev_key = normalize_key(&primary_entity.canonical_name);
        index
            .by_name
            .entry(prev_key)
            .or_default()
            .insert(primary_canonical);

        record_entity_version(&mut index, &primary_entity, "entity.merge")?;
        index.by_id.insert(primary_canonical, primary_entity.clone());

        let secondary_key = normalize_key(&secondary_entity.canonical_name);
        if let Some(set) = index.by_name.get_mut(&secondary_key) {
            set.remove(&secondary_canonical);
            if set.is_empty() {
                index.by_name.remove(&secondary_key);
            }
        }
        index.by_id.remove(&secondary_canonical);

        index
            .merged_into
            .insert(secondary_canonical, primary_canonical);
        index
            .merged_from
            .entry(primary_canonical)
            .or_default()
            .insert(secondary_canonical);

        Ok(primary_entity)
    }
    
    fn get_at_version(&self, id: EntityId, version: u64) -> Result<Option<Entity>, StorageError> {
        let index = self
            .index
            .read()
            .map_err(|_| lock_err("entity.get_at_version"))?;
        Ok(index
            .versions
            .get(&id)
            .and_then(|m| m.get(&version))
            .cloned())
    }
    
    fn list_versions(&self, id: EntityId) -> Result<Vec<Entity>, StorageError> {
        let index = self
            .index
            .read()
            .map_err(|_| lock_err("entity.list_versions"))?;
        let Some(map) = index.versions.get(&id) else {
            return Ok(Vec::new());
        };
        Ok(map.values().cloned().collect())
    }
}

// --- Belief Store ---

#[derive(Debug, Default, Clone)]
struct BeliefIndex {
    by_id: HashMap<BeliefId, Belief>,
    by_entity: HashMap<EntityId, Vec<BeliefId>>,
}

impl BeliefIndex {
    fn from_map(map: HashMap<BeliefId, Belief>) -> Self {
        let mut index = Self {
            by_id: map,
            by_entity: HashMap::new(),
        };

        for (id, belief) in index.by_id.iter() {
            index
                .by_entity
                .entry(belief.subject)
                .or_default()
                .push(*id);
        }

        index
    }

    fn insert(&mut self, belief: Belief) {
        let id = belief.id;
        let subject = belief.subject;
        self.by_entity.entry(subject).or_default().push(id);
        self.by_id.insert(id, belief);
    }
}

pub struct PersistentBeliefStore {
    wal: Arc<WriteAheadLog>,
    index: RwLock<BeliefIndex>,
}

impl PersistentBeliefStore {
    fn new(wal: Arc<WriteAheadLog>) -> Self {
        Self {
            wal,
            index: RwLock::new(BeliefIndex::default()),
        }
    }
}

impl BeliefStore for PersistentBeliefStore {
    fn insert(&self, belief: Belief) -> Result<(), StorageError> {
        let mut index = self
            .index
            .write()
            .map_err(|_| lock_err("belief.insert"))?;

        if index.by_id.contains_key(&belief.id) {
            return Err(StorageError::DuplicateKey(format!("belief:{}", belief.id)));
        }

        self
            .wal
            .append(WalEntryKind::BeliefInsert(belief.clone()))
            .map_err(|e| StorageError::BackendError(format!("WAL write failed: {}", e)))?;

        index.insert(belief);
        Ok(())
    }
    
    fn get(&self, id: BeliefId) -> Result<Option<Belief>, StorageError> {
        let index = self.index.read().map_err(|_| lock_err("belief.get"))?;
        Ok(index.by_id.get(&id).cloned())
    }
    
    fn supersede(&self, old_id: BeliefId, new_id: BeliefId) -> Result<(), StorageError> {
        let mut index = self
            .index
            .write()
            .map_err(|_| lock_err("belief.supersede"))?;
        
        if !index.by_id.contains_key(&old_id) {
            return Err(StorageError::BeliefNotFound(old_id));
        }
        
        self.wal.append(WalEntryKind::BeliefSupersede { old_id, new_id })
            .map_err(|e| StorageError::BackendError(format!("WAL write failed: {}", e)))?;
        
        if let Some(belief) = index.by_id.get_mut(&old_id) {
            belief.superseded_by = Some(new_id);
        }
        Ok(())
    }

    fn find_by_entity(&self, entity_id: EntityId) -> Result<Vec<Belief>, StorageError> {
        let index = self
            .index
            .read()
            .map_err(|_| lock_err("belief.find_by_entity"))?;
        let Some(ids) = index.by_entity.get(&entity_id) else {
            return Ok(Vec::new());
        };

        let mut beliefs: Vec<Belief> = ids
            .iter()
            .filter_map(|id| index.by_id.get(id).cloned())
            .collect();
        beliefs.sort_by(|a, b| b.tx_time.cmp(&a.tx_time));
        Ok(beliefs)
    }
    
    fn find_by_entity_predicate(&self, entity_id: EntityId, predicate: &str) -> Result<Vec<Belief>, StorageError> {
        let index = self
            .index
            .read()
            .map_err(|_| lock_err("belief.find_by_entity_predicate"))?;
        let Some(ids) = index.by_entity.get(&entity_id) else {
            return Ok(Vec::new());
        };

        let mut beliefs: Vec<Belief> = ids
            .iter()
            .filter_map(|id| index.by_id.get(id))
            .filter(|b| b.predicate == predicate)
            .cloned()
            .collect();
        beliefs.sort_by(|a, b| b.tx_time.cmp(&a.tx_time));
        Ok(beliefs)
    }
    
    fn find_as_of(&self, entity_id: EntityId, predicate: &str, as_of: DateTime<Utc>) -> Result<Vec<Belief>, StorageError> {
        let index = self
            .index
            .read()
            .map_err(|_| lock_err("belief.find_as_of"))?;
        let Some(ids) = index.by_entity.get(&entity_id) else {
            return Ok(Vec::new());
        };

        let mut beliefs: Vec<Belief> = ids
            .iter()
            .filter_map(|id| index.by_id.get(id))
            .filter(|b| b.predicate == predicate && b.valid_time.contains(as_of))
            .cloned()
            .collect();
        beliefs.sort_by(|a, b| b.tx_time.cmp(&a.tx_time));
        Ok(beliefs)
    }
    
    fn find_by_time_range(&self, range: &TimeRange) -> Result<Vec<Belief>, StorageError> {
        let index = self
            .index
            .read()
            .map_err(|_| lock_err("belief.find_by_time_range"))?;

        let mut beliefs: Vec<Belief> = index
            .by_id
            .values()
            .filter(|b| b.valid_time.overlaps(range))
            .cloned()
            .collect();
        beliefs.sort_by(|a, b| b.tx_time.cmp(&a.tx_time));
        Ok(beliefs)
    }
    
    fn find_by_embedding(&self, embedding: &[f32], limit: usize, min_confidence: Option<f32>) -> Result<Vec<(Belief, f32)>, StorageError> {
        if embedding.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let index = self
            .index
            .read()
            .map_err(|_| lock_err("belief.find_by_embedding"))?;
        let min_conf = min_confidence.unwrap_or(0.0);

        let mut scored: Vec<(Belief, f32)> = Vec::new();
        for belief in index.by_id.values() {
            if belief.confidence.value() < min_conf {
                continue;
            }
            let Some(stored) = belief.embedding.as_ref() else {
                continue;
            };
            let score = cosine_similarity(embedding, stored)?;
            if score > 0.0 {
                scored.push((belief.clone(), score));
            }
        }

        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(limit);
        Ok(scored)
    }
    
    fn count_by_entity(&self, entity_id: EntityId) -> Result<usize, StorageError> {
        let index = self
            .index
            .read()
            .map_err(|_| lock_err("belief.count_by_entity"))?;
        Ok(index.by_entity.get(&entity_id).map_or(0, Vec::len))
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

fn cosine_similarity(a: &[f32], b: &[f32]) -> Result<f32, StorageError> {
    if a.is_empty() {
        return Ok(0.0);
    }
    if a.len() != b.len() {
        return Err(StorageError::BackendError(format!(
            "embedding dimension mismatch: query={} stored={}",
            a.len(),
            b.len()
        )));
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return Ok(0.0);
    }

    let sim = dot / (norm_a * norm_b);
    if sim.is_finite() {
        Ok(sim)
    } else {
        Ok(0.0)
    }
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

    #[test]
    fn test_entity_merge_records_versions_and_redirects() {
        let dir = tempdir().unwrap();

        let stores = PersistentStores::open(dir.path(), PersistentConfig::default()).unwrap();
        let primary = Entity::new("primary", EntityType::Concept);
        let secondary = Entity::new("secondary", EntityType::Concept);
        let primary_id = primary.id;
        let secondary_id = secondary.id;

        stores.entities.insert(primary.clone()).unwrap();
        stores.entities.insert(secondary.clone()).unwrap();

        let merged = stores.entities.merge(primary_id, secondary_id).unwrap();

        assert_eq!(merged.id, primary_id);
        assert!(merged.version > primary.version);
        assert!(merged.aliases.iter().any(|a| a.eq_ignore_ascii_case("secondary")));

        let primary_resolved = stores.entities.get(primary_id).unwrap().unwrap();
        let secondary_resolved = stores.entities.get(secondary_id).unwrap().unwrap();
        assert_eq!(primary_resolved.id, primary_id);
        assert_eq!(secondary_resolved.id, primary_id);

        let versions = stores.entities.list_versions(primary_id).unwrap();
        assert!(versions.iter().any(|v| v.version == 1));
        assert!(versions.iter().any(|v| v.version == merged.version));

        let secondary_versions = stores.entities.list_versions(secondary_id).unwrap();
        assert_eq!(secondary_versions.len(), 1);
    }

    #[test]
    fn test_entity_versions_survive_reopen() {
        let dir = tempdir().unwrap();
        let entity_id;

        {
            let mut stores = PersistentStores::open(dir.path(), PersistentConfig::default()).unwrap();
            let entity = Entity::new("versioned", EntityType::Concept);
            entity_id = entity.id;
            stores.entities.insert(entity.clone()).unwrap();

            let mut updated = entity.clone();
            updated.version = 2;
            updated.canonical_name = "versioned-renamed".to_string();
            stores.entities.update(updated).unwrap();

            // Persist to a segment and reset WAL
            stores.compact().unwrap();
        }

        {
            let stores = PersistentStores::open(dir.path(), PersistentConfig::default()).unwrap();

            let v1 = stores.entities.get_at_version(entity_id, 1).unwrap().unwrap();
            assert_eq!(v1.version, 1);
            assert_eq!(v1.canonical_name, "versioned");

            let v2 = stores.entities.get_at_version(entity_id, 2).unwrap().unwrap();
            assert_eq!(v2.version, 2);
            assert_eq!(v2.canonical_name, "versioned-renamed");

            let versions = stores.entities.list_versions(entity_id).unwrap();
            assert_eq!(versions.len(), 2);
        }
    }
}
