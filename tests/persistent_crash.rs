//! Crash recovery tests for persistent storage.
//!
//! These tests verify that the storage layer correctly handles:
//! - Partial writes (simulated crash mid-write)
//! - WAL replay idempotency
//! - CRC corruption detection

#![cfg(feature = "persistent")]

use kyroql::entity::{Entity, EntityType};
use kyroql::storage::{open_database, EntityStore};

use std::fs;
use std::io::{Read, Write};
use tempfile::tempdir;

/// Test that partial/corrupted WAL entries are detected and safely skipped.
#[test]
fn test_partial_wal_entry_recovery() {
    let dir = tempdir().unwrap();
    let wal_path = dir.path().join("kyro.wal");
    
    // Write some valid entries
    {
        let stores = open_database(dir.path(), None).unwrap();
        for i in 0..5 {
            let entity = Entity::new(format!("entity_{}", i), EntityType::Concept);
            stores.entities.insert(entity).unwrap();
        }
    }
    
    // Corrupt the WAL by truncating mid-entry
    {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&wal_path)
            .unwrap();
        
        // Get current size
        let size = file.metadata().unwrap().len();
        
        // Truncate ~20% off the end (simulating crash mid-write)
        file.set_len(size * 4 / 5).unwrap();
    }
    
    // Reopen - should recover partial data
    let stores = open_database(dir.path(), None).unwrap();
    
    // Expected WAL layout: 5 entity inserts, then truncation mid-entry. Depending on alignment,
    // recovery may yield any non-zero subset that fully parsed before the truncation point.
    // Accept any count in [1,4] to tolerate encoding layout variation.
    let count = stores.entities.find_by_name_fuzzy("entity_", 10).unwrap().len();
    assert!((1..=4).contains(&count), "Recovered count should be between 1 and 4, got {count}");
}

/// Test that WAL replay is idempotent (replaying twice gives same result).
#[test]
fn test_wal_replay_idempotency() {
    let dir = tempdir().unwrap();
    let entity_id;
    
    // Create database with some data
    {
        let stores = open_database(dir.path(), None).unwrap();
        let entity = Entity::new("unique_entity", EntityType::Person);
        entity_id = entity.id;
        stores.entities.insert(entity).unwrap();
    }
    
    // First reopen
    {
        let stores = open_database(dir.path(), None).unwrap();
        let entity = stores.entities.get(entity_id).unwrap();
        assert!(entity.is_some());
        assert_eq!(entity.unwrap().canonical_name, "unique_entity");
    }
    
    // Second reopen (simulates multiple restarts)
    {
        let stores = open_database(dir.path(), None).unwrap();
        let entity = stores.entities.get(entity_id).unwrap();
        assert!(entity.is_some());
        assert_eq!(entity.unwrap().canonical_name, "unique_entity");
        
        // Should still have exactly one entity with this name
        let matches = stores.entities.find_by_name("unique_entity").unwrap();
        assert_eq!(matches.len(), 1);
    }
}

/// Test that CRC corruption in WAL entries is detected.
#[test]
fn test_crc_corruption_detection() {
    let dir = tempdir().unwrap();
    let wal_path = dir.path().join("kyro.wal");
    
    // Write valid data
    {
        let stores = open_database(dir.path(), None).unwrap();
        let entity = Entity::new("test_crc", EntityType::Concept);
        stores.entities.insert(entity).unwrap();
    }
    
    // Corrupt a byte in the middle of the WAL (flip a bit)
    {
        let mut content = Vec::new();
        let mut file = fs::File::open(&wal_path).unwrap();
        file.read_to_end(&mut content).unwrap();
        
        // Skip header (KYRO + version = 5 bytes), corrupt data area
        let idx = std::cmp::max(5, content.len() / 2);
        content[idx] ^= 0xFF; // Flip bits mid-file to force CRC failure
        
        let mut file = fs::File::create(&wal_path).unwrap();
        file.write_all(&content).unwrap();
    }
    
    // Reopen - should detect corruption via CRC mismatch
    // The system correctly reports corruption as an error
    let result = open_database(dir.path(), None);
    
    // CRC corruption should cause open to fail
    assert!(result.is_err(), "CRC corruption should be detected and reported");
    
    // Verify error mentions CRC or corruption
    if let Err(e) = result {
        let err_str = e.to_string();
        assert!(
            err_str.contains("CRC") || err_str.contains("corrupt"),
            "Error should mention CRC or corruption: {}", err_str
        );
    }
}

/// Test recovery after successful compaction.
#[test]
fn test_compaction_recovery() {
    let dir = tempdir().unwrap();
    let mut entity_ids = Vec::new();
    
    // Create data and compact
    {
        let mut stores = open_database(dir.path(), None).unwrap();
        
        for i in 0..10 {
            let entity = Entity::new(format!("compacted_{}", i), EntityType::Artifact);
            entity_ids.push(entity.id);
            stores.entities.insert(entity).unwrap();
        }
        
        // Compact
        let result = stores.compact().unwrap();
        assert_eq!(result.entries_compacted, 10);
        
        // Add more data after compaction
        for i in 10..15 {
            let entity = Entity::new(format!("after_compact_{}", i), EntityType::Artifact);
            entity_ids.push(entity.id);
            stores.entities.insert(entity).unwrap();
        }
    }
    
    // Reopen and verify all data
    {
        let stores = open_database(dir.path(), None).unwrap();
        
        // All 15 entities should exist
        for (i, id) in entity_ids.iter().enumerate() {
            let entity = stores.entities.get(*id).unwrap();
            assert!(entity.is_some(), "Entity {} should exist", i);
        }
    }
}

/// Test that multiple compactions work correctly.
#[test]
fn test_multiple_compactions() {
    let dir = tempdir().unwrap();
    
    {
        let mut stores = open_database(dir.path(), None).unwrap();
        
        // First batch
        for i in 0..5 {
            let entity = Entity::new(format!("batch1_{}", i), EntityType::Concept);
            stores.entities.insert(entity).unwrap();
        }
        stores.compact().unwrap();
        
        // Second batch
        for i in 0..5 {
            let entity = Entity::new(format!("batch2_{}", i), EntityType::Concept);
            stores.entities.insert(entity).unwrap();
        }
        stores.compact().unwrap();
        
        // Should have 2 segments
        assert_eq!(stores.segment_count(), 2);
    }
    
    // Reopen and verify
    {
        let stores = open_database(dir.path(), None).unwrap();
        
        let batch1 = stores.entities.find_by_name_fuzzy("batch1_", 10).unwrap();
        let batch2 = stores.entities.find_by_name_fuzzy("batch2_", 10).unwrap();
        
        assert_eq!(batch1.len(), 5);
        assert_eq!(batch2.len(), 5);
    }
}
