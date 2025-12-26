//! Segmented storage for read-optimized data.
//!
//! Segments are immutable files containing checkpointed data from the WAL.
//! This provides fast reads while the WAL provides durability.
//!
//! # Design
//! - Segments are numbered sequentially (segment_001.seg, segment_002.seg)
//! - Each segment contains a header, index, and data section
//! - Compaction merges WAL entries into new segments

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write, Result as IoResult};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Serialize, Deserialize};
use uuid::Uuid;

use crate::belief::Belief;
use crate::confidence::BeliefId;
use crate::conflict::{Conflict, ConflictId};
use crate::derivation::{DerivationId, DerivationRecord};
use crate::entity::{Entity, EntityId};
use crate::pattern::{Pattern, PatternId};

use super::codec;

fn normalize_key(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

/// A single segment file.
#[derive(Debug)]
pub struct Segment {
    path: PathBuf,
    /// Sequence range covered by this segment [inclusive, inclusive].
    pub sequence_range: (u64, u64),
}

impl Segment {
    /// Create a new segment file.
    pub fn create(path: &Path, sequence_start: u64) -> IoResult<Self> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        
        let mut writer = BufWriter::new(file);
        codec::write_header(&mut writer)?;
        let header = SegmentHeader {
            sequence_start,
            sequence_end: sequence_start,
            entry_count: 0,
            created_at: Utc::now(),
        };
        let header_bytes = codec::encode(&header)?;
        writer.write_all(&header_bytes)?;
        writer.flush()?;
        
        Ok(Self {
            path: path.to_path_buf(),
            sequence_range: (sequence_start, sequence_start),
        })
    }
    
    /// Open an existing segment.
    pub fn open(path: &Path) -> IoResult<Self> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        
        let _version = codec::read_header(&mut reader)?;
        
        // Read the segment header to get sequence range
        let header: SegmentHeader = codec::decode(&mut reader)?;
        
        Ok(Self {
            path: path.to_path_buf(),
            sequence_range: (header.sequence_start, header.sequence_end),
        })
    }
    
    /// Get the path to this segment.
    pub fn path(&self) -> &Path {
        &self.path
    }
    
    /// Read all data from this segment.
    pub fn read_all(&self) -> IoResult<SegmentData> {
        let file = File::open(&self.path)?;
        let mut reader = BufReader::new(file);
        
        let _version = codec::read_header(&mut reader)?;
        let _header: SegmentHeader = codec::decode(&mut reader)?;
        let data: SegmentData = codec::decode(&mut reader)?;
        
        Ok(data)
    }
}

/// Segment file header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentHeader {
    /// First sequence number in this segment.
    pub sequence_start: u64,
    /// Last sequence number in this segment.
    pub sequence_end: u64,
    /// Number of entries in this segment.
    pub entry_count: u64,
    /// Timestamp when this segment was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// All data stored in a segment.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SegmentData {
    pub entities: EntityIndex,
    pub beliefs: HashMap<BeliefId, Belief>,
    pub patterns: HashMap<PatternId, Pattern>,
    pub conflicts: HashMap<ConflictId, Conflict>,
    pub derivations: HashMap<DerivationId, DerivationRecord>,
}

/// Entity index snapshot persisted inside a segment.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntityIndex {
    pub by_id: HashMap<EntityId, Entity>,
    pub by_name: HashMap<String, HashSet<EntityId>>,
    pub versions: HashMap<EntityId, BTreeMap<u64, Entity>>,
    pub merged_into: HashMap<EntityId, EntityId>,
    pub merged_from: HashMap<EntityId, HashSet<EntityId>>,
    pub embedding_dim: Option<usize>,
}

impl SegmentData {
    /// Create empty segment data.
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Get total entry count.
    pub fn entry_count(&self) -> u64 {
        (self.entities.by_id.len() 
            + self.beliefs.len() 
            + self.patterns.len() 
            + self.conflicts.len() 
            + self.derivations.len()) as u64
    }
}

/// Builder for creating segment files atomically.
///
/// Uses write-to-temp-then-rename pattern for crash safety.
pub struct SegmentWriter {
    temp_path: Option<PathBuf>,
    final_path: PathBuf,
    writer: Option<BufWriter<File>>,
    sequence_start: u64,
    sequence_end: u64,
    data_written: bool,
}

impl SegmentWriter {
    /// Create a new segment writer.
    ///
    /// Writes to a temporary file first, then atomically renames on finalize.
    pub fn new(final_path: PathBuf, sequence_start: u64) -> IoResult<Self> {
        let temp_path = final_path.with_extension(format!("seg.tmp.{}", Uuid::new_v4()));
        
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)?;
        
        let mut writer = BufWriter::new(file);
        codec::write_header(&mut writer)?;
        
        Ok(Self {
            temp_path: Some(temp_path),
            final_path,
            writer: Some(writer),
            sequence_start,
            sequence_end: sequence_start,
            data_written: false,
        })
    }
    
    /// Write segment data.
    pub fn write_data(&mut self, data: &SegmentData, sequence_end: u64) -> IoResult<()> {
        if self.data_written {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "write_data can only be called once",
            ));
        }
        self.sequence_end = sequence_end;
        
        let writer = self.writer.as_mut().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::Other, "writer already consumed")
        })?;
        
        // Write header first
        let header = SegmentHeader {
            sequence_start: self.sequence_start,
            sequence_end: self.sequence_end,
            entry_count: data.entry_count(),
            created_at: Utc::now(),
        };
        
        let header_bytes = codec::encode(&header)?;
        writer.write_all(&header_bytes)?;
        
        // Write data
        let data_bytes = codec::encode(data)?;
        writer.write_all(&data_bytes)?;
        self.data_written = true;
        
        Ok(())
    }
    
    /// Finalize the segment (flush, fsync, rename).
    ///
    /// This is the commit point - after this returns successfully,
    /// the segment is guaranteed to be durable.
    pub fn finalize(mut self) -> IoResult<Segment> {
        let mut writer = self.writer.take().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::Other, "writer already consumed")
        })?;
        let temp_path = self.temp_path.take().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::Other, "temp_path already consumed")
        })?;
        
        // Flush buffer
        writer.flush()?;
        
        // Fsync to ensure durability
        writer.get_ref().sync_all()?;
        
        // Atomic rename
        let final_path = self.final_path.clone();
        fs::rename(&temp_path, &final_path)?;
        
        Ok(Segment {
            path: final_path,
            sequence_range: (self.sequence_start, self.sequence_end),
        })
    }
    
    /// Abort the write (cleanup temp file).
    pub fn abort(mut self) -> IoResult<()> {
        self.writer.take();
        if let Some(ref temp_path) = self.temp_path {
            if temp_path.exists() {
                fs::remove_file(temp_path)?;
            }
        }
        Ok(())
    }
}

impl Drop for SegmentWriter {
    fn drop(&mut self) {
        // Best-effort cleanup of temp file if not finalized
        if let Some(ref temp_path) = self.temp_path {
            if temp_path.exists() {
                let _ = fs::remove_file(temp_path);
            }
        }
    }
}

/// Manages segment files for a database.
#[derive(Debug)]
pub struct SegmentManager {
    dir: PathBuf,
    segments: Vec<Segment>,
    next_segment_id: u32,
}

impl SegmentManager {
    /// Open or create a segment manager for the given directory.
    pub fn open(dir: &Path) -> IoResult<Self> {
        fs::create_dir_all(dir)?;
        
        let mut segments = Vec::new();
        let mut next_segment_id = 1u32;
        
        // Scan for existing segments
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            
            if path.extension().map_or(false, |e| e == "seg") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Ok(id) = stem.strip_prefix("segment_").unwrap_or("").parse::<u32>() {
                        next_segment_id = next_segment_id.max(id + 1);
                        
                        match Segment::open(&path) {
                            Ok(seg) => segments.push(seg),
                            Err(e) => eprintln!("Warning: failed to open segment {:?}: {}", path, e),
                        }
                    }
                }
            }
            
            // Clean up any stale temp files
            if path.extension().map_or(false, |e| e == "tmp") {
                let _ = fs::remove_file(&path);
            }
        }
        
        // Sort segments by sequence range
        segments.sort_by_key(|s| s.sequence_range.0);
        
        Ok(Self {
            dir: dir.to_path_buf(),
            segments,
            next_segment_id,
        })
    }
    
    /// Get the directory containing segments.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    
    /// Get all segments, ordered by sequence.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }
    
    /// Get the highest persisted sequence number.
    pub fn persisted_sequence(&self) -> u64 {
        self.segments.last().map_or(0, |s| s.sequence_range.1)
    }
    
    /// Allocate a path for a new segment.
    pub fn next_segment_path(&mut self) -> PathBuf {
        let name = format!("segment_{:05}.seg", self.next_segment_id);
        self.next_segment_id += 1;
        self.dir.join(name)
    }
    
    /// Create a new segment writer.
    pub fn create_segment_writer(&mut self, sequence_start: u64) -> IoResult<SegmentWriter> {
        let path = self.next_segment_path();
        SegmentWriter::new(path, sequence_start)
    }
    
    /// Register a newly created segment.
    pub fn add_segment(&mut self, segment: Segment) {
        self.segments.push(segment);
        self.segments.sort_by_key(|s| s.sequence_range.0);
    }
    
    /// Load all data from all segments.
    pub fn load_all_data(&self) -> IoResult<SegmentData> {
        let mut combined = SegmentData::new();
        
        for segment in &self.segments {
            let data = segment.read_all()?;
            
            // Merge data (later segments override earlier ones)
            combined.entities.by_id.extend(data.entities.by_id);
            for (id, versions) in data.entities.versions {
                let entry = combined.entities.versions.entry(id).or_default();
                for (ver, entity) in versions {
                    entry.insert(ver, entity);
                }
            }

            combined.entities.merged_into.extend(data.entities.merged_into);

            for (id, merged) in data.entities.merged_from {
                combined
                    .entities
                    .merged_from
                    .entry(id)
                    .or_default()
                    .extend(merged);
            }

            if let Some(dim) = data.entities.embedding_dim {
                combined.entities.embedding_dim = Some(dim);
            }
            combined.beliefs.extend(data.beliefs);
            combined.patterns.extend(data.patterns);
            combined.conflicts.extend(data.conflicts);
            combined.derivations.extend(data.derivations);
        }

        // Rebuild name index from final entity state to avoid stale aliases.
        combined.entities.by_name.clear();
        for (id, entity) in &combined.entities.by_id {
            let canon = normalize_key(&entity.canonical_name);
            if !canon.is_empty() {
                combined
                    .entities
                    .by_name
                    .entry(canon)
                    .or_default()
                    .insert(*id);
            }

            for alias in &entity.aliases {
                let key = normalize_key(alias);
                if !key.is_empty() {
                    combined
                        .entities
                        .by_name
                        .entry(key)
                        .or_default()
                        .insert(*id);
                }
            }
        }
        
        Ok(combined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityType;
    use tempfile::tempdir;
    
    #[test]
    fn test_segment_manager_open_empty() {
        let dir = tempdir().unwrap();
        let manager = SegmentManager::open(dir.path()).unwrap();
        
        assert!(manager.segments().is_empty());
        assert_eq!(manager.persisted_sequence(), 0);
    }
    
    #[test]
    fn test_segment_path_allocation() {
        let dir = tempdir().unwrap();
        let mut manager = SegmentManager::open(dir.path()).unwrap();
        
        let path1 = manager.next_segment_path();
        let path2 = manager.next_segment_path();
        
        assert_ne!(path1, path2);
        assert!(path1.to_string_lossy().contains("segment_00001"));
        assert!(path2.to_string_lossy().contains("segment_00002"));
    }
    
    #[test]
    fn test_segment_writer_roundtrip() {
        let dir = tempdir().unwrap();
        let mut manager = SegmentManager::open(dir.path()).unwrap();
        
        // Create test data
        let mut data = SegmentData::new();
        let entity = Entity::new("test", EntityType::Concept);
        data.entities.by_id.insert(entity.id, entity.clone());
        
        // Write segment
        let mut writer = manager.create_segment_writer(1).unwrap();
        writer.write_data(&data, 10).unwrap();
        let segment = writer.finalize().unwrap();
        
        assert_eq!(segment.sequence_range, (1, 10));
        
        // Read back
        let read_data = segment.read_all().unwrap();
        assert_eq!(read_data.entities.by_id.len(), 1);
        assert!(read_data.entities.by_id.contains_key(&entity.id));
    }
    
    #[test]
    fn test_segment_writer_abort() {
        let dir = tempdir().unwrap();
        let mut manager = SegmentManager::open(dir.path()).unwrap();
        
        let writer = manager.create_segment_writer(1).unwrap();
        let temp_path = writer.temp_path.clone().unwrap();
        
        // Abort should clean up
        writer.abort().unwrap();
        
        assert!(!temp_path.exists());
    }
}
