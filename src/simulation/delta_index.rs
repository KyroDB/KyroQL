//! Delta vector index for simulations.
//!
//! This overlay index is intentionally simple (exact scan) and deterministic.
//! It exists to keep simulation overlays self-contained and to provide a
//! well-defined upgrade path to approximate indices if needed.

use std::collections::HashMap;

use crate::confidence::BeliefId;
use crate::storage::StorageError;

#[derive(Debug, Clone)]
struct Entry {
    embedding: Vec<f32>,
    confidence: f32,
}

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

    let mut dot = 0.0f64;
    let mut norm_a = 0.0f64;
    let mut norm_b = 0.0f64;

    for (&x, &y) in a.iter().zip(b.iter()) {
        if !x.is_finite() || !y.is_finite() {
            return Err(StorageError::BackendError(
                "non-finite value in embedding".to_string(),
            ));
        }
        let xf = f64::from(x);
        let yf = f64::from(y);
        dot += xf * yf;
        norm_a += xf * xf;
        norm_b += yf * yf;
    }

    if norm_a <= 0.0 || norm_b <= 0.0 {
        return Ok(0.0);
    }

    let sim = dot / (norm_a.sqrt() * norm_b.sqrt());
    if sim.is_finite() {
        #[allow(clippy::cast_possible_truncation)]
        Ok(sim as f32)
    } else {
        Ok(0.0)
    }
}

/// Overlay vector index for hypothetical embeddings.
#[derive(Debug, Default)]
pub struct DeltaVectorIndex {
    embedding_dim: Option<usize>,
    entries: HashMap<BeliefId, Entry>,
}

impl DeltaVectorIndex {
    /// Create an empty delta vector index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all overlay state.
    pub fn clear(&mut self) {
        self.embedding_dim = None;
        self.entries.clear();
    }

    /// Insert or update an embedding for a belief.
    pub fn upsert(&mut self, id: BeliefId, embedding: &[f32], confidence: f32) -> Result<(), StorageError> {
        if embedding.is_empty() {
            return Err(StorageError::BackendError(
                "embedding dimension must be non-zero".to_string(),
            ));
        }
        if !confidence.is_finite() {
            return Err(StorageError::BackendError(
                "non-finite confidence is not allowed".to_string(),
            ));
        }

        match self.embedding_dim {
            None => self.embedding_dim = Some(embedding.len()),
            Some(d) if d == embedding.len() => {}
            Some(d) => {
                return Err(StorageError::BackendError(format!(
                    "embedding dimension mismatch (delta_index): expected={d} actual={}",
                    embedding.len()
                )));
            }
        }

        for &v in embedding {
            if !v.is_finite() {
                return Err(StorageError::BackendError(
                    "non-finite value in embedding".to_string(),
                ));
            }
        }

        self.entries.insert(
            id,
            Entry {
                embedding: embedding.to_vec(),
                confidence,
            },
        );
        Ok(())
    }

    /// Remove an embedding entry.
    pub fn remove(&mut self, id: BeliefId) {
        self.entries.remove(&id);
    }

    /// Search the overlay for the most similar embeddings.
    pub fn search(
        &self,
        query: &[f32],
        limit: usize,
        min_confidence: Option<f32>,
    ) -> Result<Vec<(BeliefId, f32)>, StorageError> {
        if limit == 0 || query.is_empty() {
            return Ok(Vec::new());
        }

        if let Some(d) = self.embedding_dim {
            if d != query.len() {
                return Err(StorageError::BackendError(format!(
                    "embedding dimension mismatch (delta_index.search): expected={d} actual={}",
                    query.len()
                )));
            }
        }

        let mut out = Vec::new();
        for (id, entry) in &self.entries {
            if let Some(min) = min_confidence {
                if entry.confidence < min {
                    continue;
                }
            }
            let sim = cosine_similarity(query, &entry.embedding)?;
            out.push((*id, sim, entry.confidence));
        }

        out.sort_by(|(a_id, a_sim, a_conf), (b_id, b_sim, b_conf)| {
            b_sim
                .partial_cmp(a_sim)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b_conf.partial_cmp(a_conf).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| a_id.to_string().cmp(&b_id.to_string()))
        });

        out.truncate(limit);
        Ok(out.into_iter().map(|(id, sim, _)| (id, sim)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_enforces_dim_and_finite_values() {
        let mut idx = DeltaVectorIndex::new();
        let id = BeliefId::new();
        idx.upsert(id, &[1.0, 0.0], 0.9).unwrap();

        let err = idx.upsert(BeliefId::new(), &[1.0], 0.9).unwrap_err();
        assert!(matches!(err, StorageError::BackendError(_)));

        let err = idx
            .upsert(BeliefId::new(), &[f32::NAN, 0.0], 0.9)
            .unwrap_err();
        assert!(matches!(err, StorageError::BackendError(_)));

        let err = idx
            .upsert(BeliefId::new(), &[1.0, 0.0], f32::INFINITY)
            .unwrap_err();
        assert!(matches!(err, StorageError::BackendError(_)));
    }

    #[test]
    fn search_is_deterministic_and_filters_by_confidence() {
        let mut idx = DeltaVectorIndex::new();
        let a = BeliefId::new();
        let b = BeliefId::new();
        idx.upsert(a, &[1.0, 0.0, 0.0], 0.1).unwrap();
        idx.upsert(b, &[1.0, 0.0, 0.0], 0.9).unwrap();

        let hits = idx.search(&[1.0, 0.0, 0.0], 10, Some(0.5)).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, b);

        let hits = idx.search(&[1.0, 0.0, 0.0], 1, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, b);
    }
}
