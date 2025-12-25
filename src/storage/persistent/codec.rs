//! Binary codec for storage entries.
//!
//! All data is serialized with:
//! - JSON for data (compatible with existing serde attributes)
//! - Length-prefixed format for framing
//! - CRC32 checksum for corruption detection
//! - Version byte for forward compatibility

use std::io::{Read, Write, Result as IoResult, Error as IoError, ErrorKind};
use crc32fast::Hasher;
use serde::{Serialize, de::DeserializeOwned};

/// Current codec version.
const CODEC_VERSION: u8 = 1;

/// Magic bytes to identify KyroQL files.
pub const MAGIC: [u8; 4] = *b"KYRO";

/// Serializes a value to bytes with checksum.
///
/// Format:
/// ```text
/// [version: 1 byte][length: 4 bytes LE][data: N bytes JSON][crc32: 4 bytes LE]
/// ```
pub fn encode<T: Serialize>(value: &T) -> IoResult<Vec<u8>> {
    let data = serde_json::to_vec(value)
        .map_err(|e| IoError::new(ErrorKind::InvalidData, format!("serialization failed: {}", e)))?;
    
    let mut hasher = Hasher::new();
    hasher.update(&data);
    let crc = hasher.finalize();
    
    let len = data.len() as u32;
    
    let mut out = Vec::with_capacity(1 + 4 + data.len() + 4);
    out.push(CODEC_VERSION);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&data);
    out.extend_from_slice(&crc.to_le_bytes());
    
    Ok(out)
}

/// Deserializes a value from bytes, verifying checksum.
///
/// # Errors
/// - Returns error if checksum fails (corruption detected)
/// - Returns error if version is unsupported
/// - Returns error if deserialization fails
pub fn decode<T: DeserializeOwned>(reader: &mut impl Read) -> IoResult<T> {
    // Read version
    let mut version = [0u8; 1];
    reader.read_exact(&mut version)?;
    
    if version[0] != CODEC_VERSION {
        return Err(IoError::new(
            ErrorKind::InvalidData,
            format!("unsupported codec version: {} (expected {})", version[0], CODEC_VERSION),
        ));
    }
    
    // Read length
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes)?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    
    // Sanity check: reject unreasonably large entries (100 MB max)
    const MAX_ENTRY_SIZE: usize = 100 * 1024 * 1024;
    if len > MAX_ENTRY_SIZE {
        return Err(IoError::new(
            ErrorKind::InvalidData,
            format!("entry size {} exceeds maximum {}", len, MAX_ENTRY_SIZE),
        ));
    }
    
    // Read data
    let mut data = vec![0u8; len];
    reader.read_exact(&mut data)?;
    
    // Read and verify CRC
    let mut crc_bytes = [0u8; 4];
    reader.read_exact(&mut crc_bytes)?;
    let stored_crc = u32::from_le_bytes(crc_bytes);
    
    let mut hasher = Hasher::new();
    hasher.update(&data);
    let computed_crc = hasher.finalize();
    
    if stored_crc != computed_crc {
        return Err(IoError::new(
            ErrorKind::InvalidData,
            format!(
                "CRC mismatch: stored={:08x}, computed={:08x} (data corrupted)",
                stored_crc, computed_crc
            ),
        ));
    }
    
    // Deserialize
    serde_json::from_slice(&data)
        .map_err(|e| IoError::new(ErrorKind::InvalidData, format!("deserialization failed: {}", e)))
}

/// Write the file header (magic + version).
pub fn write_header(writer: &mut impl Write) -> IoResult<()> {
    writer.write_all(&MAGIC)?;
    writer.write_all(&[CODEC_VERSION])?;
    Ok(())
}

/// Read and validate the file header.
pub fn read_header(reader: &mut impl Read) -> IoResult<u8> {
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;
    
    if magic != MAGIC {
        return Err(IoError::new(
            ErrorKind::InvalidData,
            format!("invalid magic bytes: expected {:?}, got {:?}", MAGIC, magic),
        ));
    }
    
    let mut version = [0u8; 1];
    reader.read_exact(&mut version)?;
    
    Ok(version[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    
    #[test]
    fn test_roundtrip_simple() {
        let value = "hello, world!".to_string();
        let encoded = encode(&value).unwrap();
        
        let mut cursor = Cursor::new(encoded);
        let decoded: String = decode(&mut cursor).unwrap();
        
        assert_eq!(value, decoded);
    }
    
    #[test]
    fn test_detects_corruption() {
        let value = "test data".to_string();
        let mut encoded = encode(&value).unwrap();
        
        // Corrupt a byte in the data section
        if encoded.len() > 10 {
            encoded[10] ^= 0xFF;
        }
        
        let mut cursor = Cursor::new(encoded);
        let result: IoResult<String> = decode(&mut cursor);
        
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("CRC") || err.to_string().contains("corrupt") || err.to_string().contains("deserialization"));
    }
    
    #[test]
    fn test_rejects_oversized_entry() {
        // Craft a malicious header claiming huge size
        let mut bad_data = vec![CODEC_VERSION];
        bad_data.extend_from_slice(&(200_000_000u32).to_le_bytes()); // 200 MB
        
        let mut cursor = Cursor::new(bad_data);
        let result: IoResult<String> = decode(&mut cursor);
        
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds maximum"));
    }
    
    #[test]
    fn test_header_roundtrip() {
        let mut buf = Vec::new();
        write_header(&mut buf).unwrap();
        
        let mut cursor = Cursor::new(buf);
        let version = read_header(&mut cursor).unwrap();
        
        assert_eq!(version, CODEC_VERSION);
    }
    
    #[test]
    fn test_walentry_roundtrip() {
        use crate::entity::{Entity, EntityType};
        use crate::storage::persistent::wal::{WalEntry, WalEntryKind};
        use chrono::Utc;
        
        let entity = Entity::new("test_entity", EntityType::Concept);
        let entry = WalEntry {
            sequence: 1,
            timestamp: Utc::now(),
            kind: WalEntryKind::EntityInsert(entity),
        };
        
        let encoded = encode(&entry).unwrap();
        
        let mut cursor = Cursor::new(encoded);
        let decoded: WalEntry = decode(&mut cursor).unwrap();
        
        assert_eq!(decoded.sequence, 1);
        assert!(matches!(decoded.kind, WalEntryKind::EntityInsert(_)));
    }
    
    #[test]
    fn test_checkpoint_roundtrip() {
        use crate::storage::persistent::wal::{WalEntry, WalEntryKind};
        use chrono::Utc;
        
        // Test with a simpler variant
        let entry = WalEntry {
            sequence: 1,
            timestamp: Utc::now(),
            kind: WalEntryKind::Checkpoint { up_to_sequence: 100 },
        };
        
        let encoded = encode(&entry).unwrap();
        
        let mut cursor = Cursor::new(encoded);
        let decoded: WalEntry = decode(&mut cursor).unwrap();
        
        assert_eq!(decoded.sequence, 1);
        assert!(matches!(decoded.kind, WalEntryKind::Checkpoint { up_to_sequence: 100 }));
    }
}
