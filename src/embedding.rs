//! Deterministic lexical embedding.
//!
//! Deterministic embedding generation for simple semantic retrieval.
//! This implementation is deterministic, offline, and dependency-free beyond `blake3`.
//!
//! It is *not* a neural embedding model. It provides a stable baseline using feature
//! hashing over tokens, sufficient for top-k similarity search in embedded mode.

use blake3::Hasher;

/// Default embedding dimensionality for lexical embeddings.
///
/// Keep this modest to control memory usage in embedded mode.
pub const DEFAULT_EMBEDDING_DIM: usize = 64;

fn tokenize(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
}

/// Create a deterministic lexical embedding for a piece of text.
#[must_use]
pub fn lexical_embedding(text: &str) -> Vec<f32> {
    lexical_embedding_with_dim(text, DEFAULT_EMBEDDING_DIM)
}

/// Create a deterministic lexical embedding with a custom dimension.
#[must_use]
pub fn lexical_embedding_with_dim(text: &str, dim: usize) -> Vec<f32> {
    if dim == 0 {
        return Vec::new();
    }

    let mut vec = vec![0.0f32; dim];
    let mut count = 0u32;

    for token in tokenize(&text.to_ascii_lowercase()) {
        let mut h = Hasher::new();
        h.update(token.as_bytes());
        let hash = h.finalize();

        let bytes = hash.as_bytes();
        // Deterministically map to a bucket.
        let mut bucket = 0u64;
        bucket |= u64::from(bytes[0]);
        bucket |= u64::from(bytes[1]) << 8;
        bucket |= u64::from(bytes[2]) << 16;
        bucket |= u64::from(bytes[3]) << 24;
        bucket |= u64::from(bytes[4]) << 32;
        bucket |= u64::from(bytes[5]) << 40;
        bucket |= u64::from(bytes[6]) << 48;
        bucket |= u64::from(bytes[7]) << 56;

        let idx = (bucket as usize) % dim;
        let sign = if (bytes[8] & 1) == 0 { 1.0f32 } else { -1.0f32 };
        vec[idx] += sign;
        count = count.saturating_add(1);
    }

    if count == 0 {
        return vec;
    }

    // L2-normalize.
    let mut norm2 = 0.0f64;
    for &x in &vec {
        norm2 += f64::from(x) * f64::from(x);
    }
    if norm2 > 0.0 {
        let inv = (norm2.sqrt()).recip();
        #[allow(clippy::cast_possible_truncation)]
        let invf = inv as f32;
        for x in &mut vec {
            *x *= invf;
        }
    }

    vec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_embedding_is_deterministic() {
        let a = lexical_embedding("hello world");
        let b = lexical_embedding("hello world");
        assert_eq!(a, b);
    }

    #[test]
    fn lexical_embedding_dim_is_respected() {
        let v = lexical_embedding_with_dim("x", 13);
        assert_eq!(v.len(), 13);
    }
}
