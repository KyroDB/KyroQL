//! In-memory storage backend.
//!
//! This module provides thread-safe in-memory implementations of the storage traits.
//! It is intended for embedded usage, tests, and as a reference implementation.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::RwLock;

use chrono::{DateTime, Duration, Utc};

use crate::belief::Belief;
use crate::confidence::BeliefId;
use crate::conflict::{Conflict, ConflictId, ConflictStatus};
use crate::entity::{Entity, EntityId};
use crate::pattern::{Pattern, PatternId};
use crate::storage::traits::{BeliefStore, ConflictStore, EntityStore, PatternStore, StorageError};
use crate::time::TimeRange;

fn lock_err(context: &'static str) -> StorageError {
    StorageError::BackendError(format!("poisoned lock: {context}"))
}

fn normalize_key(s: &str) -> String {
    s.trim().to_ascii_lowercase()
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

#[derive(Debug, Default)]
struct EntityState {
    by_id: HashMap<EntityId, Entity>,
    by_name: HashMap<String, HashSet<EntityId>>,
    versions: HashMap<EntityId, BTreeMap<u64, Entity>>,
    merged_into: HashMap<EntityId, EntityId>,
    merged_from: HashMap<EntityId, HashSet<EntityId>>,
    embedding_dim: Option<usize>,
}

fn resolve_canonical_id(state: &EntityState, id: EntityId) -> Result<EntityId, StorageError> {
    let mut current = id;
    for _ in 0..128 {
        let Some(next) = state.merged_into.get(&current).copied() else {
            return Ok(current);
        };
        if next == current {
            return Err(StorageError::BackendError(
                "entity merge map contains a self-cycle".to_string(),
            ));
        }
        current = next;
    }

    Err(StorageError::BackendError(
        "entity merge map resolution exceeded hop limit".to_string(),
    ))
}

fn record_entity_version(
    state: &mut EntityState,
    entity: &Entity,
    context: &'static str,
) -> Result<(), StorageError> {
    let versions = state.versions.entry(entity.id).or_default();
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

fn ensure_embedding_dim(
    expected: &mut Option<usize>,
    actual: usize,
    context: &'static str,
) -> Result<(), StorageError> {
    if actual == 0 {
        return Err(StorageError::BackendError(format!(
            "embedding dimension must be non-zero ({context})"
        )));
    }

    match expected {
        None => {
            *expected = Some(actual);
            Ok(())
        }
        Some(exp) if *exp == actual => Ok(()),
        Some(exp) => Err(StorageError::BackendError(format!(
            "embedding dimension mismatch ({context}): expected={exp} actual={actual}"
        ))),
    }
}

/// Thread-safe in-memory entity store.
#[derive(Debug, Default)]
pub struct InMemoryEntityStore {
    state: RwLock<EntityState>,
}

impl InMemoryEntityStore {
    /// Create a new empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl EntityStore for InMemoryEntityStore {
    fn insert(&self, entity: Entity) -> Result<(), StorageError> {
        let mut state = self.state.write().map_err(|_| lock_err("entity.insert"))?;
        if state.by_id.contains_key(&entity.id) || state.merged_into.contains_key(&entity.id) {
            return Err(StorageError::DuplicateKey(entity.id.to_string()));
        }

        if let Some(emb) = entity.embedding.as_ref() {
            ensure_embedding_dim(&mut state.embedding_dim, emb.len(), "entity.insert")?;
        }

        record_entity_version(&mut state, &entity, "entity.insert")?;

        let name_key = normalize_key(&entity.canonical_name);
        state.by_name.entry(name_key).or_default().insert(entity.id);
        state.by_id.insert(entity.id, entity);
        Ok(())
    }

    fn get(&self, id: EntityId) -> Result<Option<Entity>, StorageError> {
        let state = self.state.read().map_err(|_| lock_err("entity.get"))?;
        let canonical = resolve_canonical_id(&state, id)?;
        Ok(state.by_id.get(&canonical).cloned())
    }

    fn update(&self, entity: Entity) -> Result<(), StorageError> {
        let mut state = self.state.write().map_err(|_| lock_err("entity.update"))?;
        let canonical = resolve_canonical_id(&state, entity.id)?;
        if canonical != entity.id {
            return Err(StorageError::BackendError(
                "cannot update an entity that has been merged".to_string(),
            ));
        }
        let prev = state
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
            ensure_embedding_dim(&mut state.embedding_dim, emb.len(), "entity.update")?;
        }

        let prev_key = normalize_key(&prev.canonical_name);
        let new_key = normalize_key(&entity.canonical_name);
        if prev_key != new_key {
            if let Some(set) = state.by_name.get_mut(&prev_key) {
                set.remove(&entity.id);
                if set.is_empty() {
                    state.by_name.remove(&prev_key);
                }
            }
            state.by_name.entry(new_key).or_default().insert(entity.id);
        }

        record_entity_version(&mut state, &entity, "entity.update")?;
        state.by_id.insert(entity.id, entity);
        Ok(())
    }

    fn delete(&self, id: EntityId) -> Result<(), StorageError> {
        let mut state = self.state.write().map_err(|_| lock_err("entity.delete"))?;
        let canonical = resolve_canonical_id(&state, id)?;
        if canonical != id {
            return Err(StorageError::BackendError(
                "cannot delete an entity that has been merged".to_string(),
            ));
        }

        if state
            .merged_from
            .get(&id)
            .map_or(false, |s| !s.is_empty())
        {
            return Err(StorageError::BackendError(
                "cannot delete an entity that has other entities merged into it".to_string(),
            ));
        }

        let prev = state
            .by_id
            .remove(&id)
            .ok_or(StorageError::EntityNotFound(id))?;

        let prev_key = normalize_key(&prev.canonical_name);
        if let Some(set) = state.by_name.get_mut(&prev_key) {
            set.remove(&id);
            if set.is_empty() {
                state.by_name.remove(&prev_key);
            }
        }

        Ok(())
    }

    fn find_by_name(&self, name: &str) -> Result<Vec<Entity>, StorageError> {
        let name_key = normalize_key(name);
        let state = self.state.read().map_err(|_| lock_err("entity.find_by_name"))?;
        let Some(ids) = state.by_name.get(&name_key) else {
            return Ok(Vec::new());
        };

        let mut results: Vec<Entity> = ids
            .iter()
            .filter_map(|id| state.by_id.get(id).cloned())
            .collect();
        results.sort_by(|a, b| a.canonical_name.cmp(&b.canonical_name));
        Ok(results)
    }

    fn find_by_name_fuzzy(&self, query: &str, limit: usize) -> Result<Vec<Entity>, StorageError> {
        let query_key = normalize_key(query);
        if query_key.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let state = self.state.read().map_err(|_| lock_err("entity.find_by_name_fuzzy"))?;

        let mut scored: Vec<(u8, Entity)> = Vec::new();
        for entity in state.by_id.values() {
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

    fn find_by_embedding(
        &self,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(Entity, f32)>, StorageError> {
        if embedding.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let state = self.state.read().map_err(|_| lock_err("entity.find_by_embedding"))?;
        if let Some(exp) = state.embedding_dim {
            if exp != embedding.len() {
                return Err(StorageError::BackendError(format!(
                    "embedding dimension mismatch (entity.find_by_embedding): expected={exp} actual={}"
                    , embedding.len()
                )));
            }
        }
        let mut scored: Vec<(Entity, f32)> = Vec::new();
        for entity in state.by_id.values() {
            let Some(stored) = entity.embedding.as_ref() else {
                continue;
            };

            let sim = cosine_similarity(embedding, stored)?;

            if sim > 0.0 {
                scored.push((entity.clone(), sim));
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

        let mut state = self.state.write().map_err(|_| lock_err("entity.merge"))?;

        let primary_canonical = resolve_canonical_id(&state, primary)?;
        let secondary_canonical = resolve_canonical_id(&state, secondary)?;
        if primary_canonical == secondary_canonical {
            return Err(StorageError::BackendError(
                "cannot merge: both IDs resolve to the same canonical entity".to_string(),
            ));
        }

        let mut primary_entity = state
            .by_id
            .get(&primary_canonical)
            .cloned()
            .ok_or(StorageError::EntityNotFound(primary_canonical))?;
        let secondary_entity = state
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

        primary_entity.metadata = merge_metadata(&primary_entity.metadata, &secondary_entity.metadata);

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
            ensure_embedding_dim(&mut state.embedding_dim, emb.len(), "entity.merge")?;
        }

        record_entity_version(&mut state, &primary_entity, "entity.merge")?;
        state.by_id.insert(primary_canonical, primary_entity.clone());

        let prev_key = normalize_key(&secondary_entity.canonical_name);
        if let Some(set) = state.by_name.get_mut(&prev_key) {
            set.remove(&secondary_canonical);
            if set.is_empty() {
                state.by_name.remove(&prev_key);
            }
        }
        state.by_id.remove(&secondary_canonical);

        state
            .merged_into
            .insert(secondary_canonical, primary_canonical);
        state
            .merged_from
            .entry(primary_canonical)
            .or_default()
            .insert(secondary_canonical);

        Ok(primary_entity)
    }

    fn get_at_version(&self, id: EntityId, version: u64) -> Result<Option<Entity>, StorageError> {
        let state = self.state.read().map_err(|_| lock_err("entity.get_at_version"))?;
        Ok(state
            .versions
            .get(&id)
            .and_then(|m| m.get(&version))
            .cloned())
    }

    fn list_versions(&self, id: EntityId) -> Result<Vec<Entity>, StorageError> {
        let state = self.state.read().map_err(|_| lock_err("entity.list_versions"))?;
        let Some(map) = state.versions.get(&id) else {
            return Ok(Vec::new());
        };
        Ok(map.values().cloned().collect())
    }
}

#[derive(Debug, Default)]
struct BeliefState {
    by_id: HashMap<BeliefId, Belief>,
    by_entity: HashMap<EntityId, Vec<BeliefId>>,
    by_entity_predicate: HashMap<(EntityId, String), Vec<BeliefId>>,
    embedding_dim: Option<usize>,
}

/// Thread-safe in-memory belief store.
#[derive(Debug, Default)]
pub struct InMemoryBeliefStore {
    state: RwLock<BeliefState>,
}

impl InMemoryBeliefStore {
    /// Create a new empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl InMemoryBeliefStore {
    fn index_insert(state: &mut BeliefState, belief: &Belief) {
        state.by_entity.entry(belief.subject).or_default().push(belief.id);
        state
            .by_entity_predicate
            .entry((belief.subject, belief.predicate.clone()))
            .or_default()
            .push(belief.id);
    }
}

impl BeliefStore for InMemoryBeliefStore {
    fn insert(&self, belief: Belief) -> Result<(), StorageError> {
        let mut state = self.state.write().map_err(|_| lock_err("belief.insert"))?;
        if state.by_id.contains_key(&belief.id) {
            return Err(StorageError::DuplicateKey(belief.id.to_string()));
        }

        if let Some(emb) = belief.embedding.as_ref() {
            ensure_embedding_dim(&mut state.embedding_dim, emb.len(), "belief.insert")?;
        }

        Self::index_insert(&mut state, &belief);
        state.by_id.insert(belief.id, belief);
        Ok(())
    }

    fn get(&self, id: BeliefId) -> Result<Option<Belief>, StorageError> {
        let state = self.state.read().map_err(|_| lock_err("belief.get"))?;
        Ok(state.by_id.get(&id).cloned())
    }

    fn supersede(&self, old_id: BeliefId, new_id: BeliefId) -> Result<(), StorageError> {
        if old_id == new_id {
            return Err(StorageError::BackendError(
                "cannot supersede a belief with itself".to_string(),
            ));
        }

        let mut state = self.state.write().map_err(|_| lock_err("belief.supersede"))?;
        let new_tx = state
            .by_id
            .get(&new_id)
            .ok_or(StorageError::BeliefNotFound(new_id))?
            .tx_time;

        {
            let new_belief = state
                .by_id
                .get(&new_id)
                .ok_or(StorageError::BeliefNotFound(new_id))?;
            if let Some(existing) = new_belief.supersedes {
                if existing != old_id {
                    return Err(StorageError::BackendError(format!(
                        "belief {new_id} already supersedes {existing}; cannot also supersede {old_id}"
                    )));
                }
            }
        }

        let old_belief = state
            .by_id
            .get_mut(&old_id)
            .ok_or(StorageError::BeliefNotFound(old_id))?;

        if let Some(existing) = old_belief.superseded_by {
            if existing == new_id {
                // Idempotent supersession.
                return Ok(());
            }
            return Err(StorageError::BackendError(format!(
                "belief {old_id} is already superseded by {existing}"
            )));
        }

        old_belief.superseded_by = Some(new_id);

        // Close the old belief's valid time at the superseding belief's transaction time.
        // Clamp to ensure we never create an invalid (empty) interval.
        let end = if new_tx > old_belief.valid_time.from() {
            new_tx
        } else {
            old_belief.valid_time.from() + Duration::microseconds(1)
        };

        let end = match old_belief.valid_time.to() {
            Some(existing) => existing.min(end),
            None => end,
        };

        old_belief.valid_time.set_to_clamped(end);

        // Update the forward link on the new belief.
        let new_belief = state
            .by_id
            .get_mut(&new_id)
            .ok_or(StorageError::BeliefNotFound(new_id))?;
        if new_belief.supersedes.is_none() {
            new_belief.supersedes = Some(old_id);
        }

        Ok(())
    }

    fn find_by_entity_predicate(
        &self,
        entity_id: EntityId,
        predicate: &str,
    ) -> Result<Vec<Belief>, StorageError> {
        let predicate = predicate.trim();
        let state = self
            .state
            .read()
            .map_err(|_| lock_err("belief.find_by_entity_predicate"))?;
        let key = (entity_id, predicate.to_string());
        let Some(ids) = state.by_entity_predicate.get(&key) else {
            return Ok(Vec::new());
        };

        let mut beliefs: Vec<Belief> = ids
            .iter()
            .filter_map(|id| state.by_id.get(id).cloned())
            .collect();
        beliefs.sort_by(|a, b| b.tx_time.cmp(&a.tx_time));
        Ok(beliefs)
    }

    fn find_as_of(
        &self,
        entity_id: EntityId,
        predicate: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Vec<Belief>, StorageError> {
        let beliefs = self.find_by_entity_predicate(entity_id, predicate)?;
        Ok(beliefs
            .into_iter()
            .filter(|b| b.is_valid_at(as_of))
            .collect())
    }

    fn find_by_time_range(&self, range: &TimeRange) -> Result<Vec<Belief>, StorageError> {
        let state = self
            .state
            .read()
            .map_err(|_| lock_err("belief.find_by_time_range"))?;

        let mut beliefs: Vec<Belief> = state
            .by_id
            .values()
            .filter(|b| b.valid_time.overlaps(range))
            .cloned()
            .collect();

        beliefs.sort_by(|a, b| b.tx_time.cmp(&a.tx_time));
        Ok(beliefs)
    }

    fn find_by_embedding(
        &self,
        embedding: &[f32],
        limit: usize,
        min_confidence: Option<f32>,
    ) -> Result<Vec<(Belief, f32)>, StorageError> {
        if embedding.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let min_confidence = min_confidence.unwrap_or(0.0).clamp(0.0, 1.0);
        let state = self
            .state
            .read()
            .map_err(|_| lock_err("belief.find_by_embedding"))?;

        if let Some(exp) = state.embedding_dim {
            if exp != embedding.len() {
                return Err(StorageError::BackendError(format!(
                    "embedding dimension mismatch (belief.find_by_embedding): expected={exp} actual={}"
                    , embedding.len()
                )));
            }
        }

        let mut scored: Vec<(Belief, f32)> = Vec::new();
        for belief in state.by_id.values() {
            let Some(stored) = belief.embedding.as_ref() else {
                continue;
            };
            if belief.confidence.value() < min_confidence {
                continue;
            }

            let sim = cosine_similarity(embedding, stored)?;

            if sim > 0.0 {
                scored.push((belief.clone(), sim));
            }
        }

        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(limit);
        Ok(scored)
    }

    fn count_by_entity(&self, entity_id: EntityId) -> Result<usize, StorageError> {
        let state = self
            .state
            .read()
            .map_err(|_| lock_err("belief.count_by_entity"))?;
        Ok(state.by_entity.get(&entity_id).map_or(0, Vec::len))
    }
}

#[derive(Debug, Default)]
struct ConflictState {
    by_id: HashMap<ConflictId, Conflict>,
    by_belief: HashMap<BeliefId, Vec<ConflictId>>,
}

/// Thread-safe in-memory conflict store.
#[derive(Debug, Default)]
pub struct InMemoryConflictStore {
    state: RwLock<ConflictState>,
}

impl InMemoryConflictStore {
    /// Create a new empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ConflictStore for InMemoryConflictStore {
    fn insert(&self, conflict: Conflict) -> Result<(), StorageError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| lock_err("conflict.insert"))?;

        if state.by_id.contains_key(&conflict.id) {
            return Err(StorageError::DuplicateKey(conflict.id.to_string()));
        }

        for belief_id in &conflict.belief_ids {
            state
                .by_belief
                .entry(*belief_id)
                .or_default()
                .push(conflict.id);
        }

        state.by_id.insert(conflict.id, conflict);
        Ok(())
    }

    fn get(&self, id: ConflictId) -> Result<Option<Conflict>, StorageError> {
        let state = self
            .state
            .read()
            .map_err(|_| lock_err("conflict.get"))?;
        Ok(state.by_id.get(&id).cloned())
    }

    fn update(&self, conflict: Conflict) -> Result<(), StorageError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| lock_err("conflict.update"))?;

        let old = state
            .by_id
            .get(&conflict.id)
            .cloned()
            .ok_or(StorageError::ConflictNotFound(conflict.id))?;

        let old_ids: HashSet<BeliefId> = old.belief_ids.iter().copied().collect();
        let new_ids: HashSet<BeliefId> = conflict.belief_ids.iter().copied().collect();

        if old_ids != new_ids {
            for removed in old_ids.difference(&new_ids) {
                if let Some(list) = state.by_belief.get_mut(removed) {
                    list.retain(|cid| *cid != conflict.id);
                    if list.is_empty() {
                        state.by_belief.remove(removed);
                    }
                }
            }

            for added in new_ids.difference(&old_ids) {
                let entry = state.by_belief.entry(*added).or_default();
                if !entry.iter().any(|cid| *cid == conflict.id) {
                    entry.push(conflict.id);
                }
            }
        }

        state.by_id.insert(conflict.id, conflict);
        Ok(())
    }

    fn find_by_belief(&self, belief_id: BeliefId) -> Result<Vec<Conflict>, StorageError> {
        let state = self
            .state
            .read()
            .map_err(|_| lock_err("conflict.find_by_belief"))?;

        let Some(ids) = state.by_belief.get(&belief_id) else {
            return Ok(Vec::new());
        };

        Ok(ids
            .iter()
            .filter_map(|id| state.by_id.get(id).cloned())
            .collect())
    }

    fn find_open(&self) -> Result<Vec<Conflict>, StorageError> {
        let state = self
            .state
            .read()
            .map_err(|_| lock_err("conflict.find_open"))?;
        Ok(state
            .by_id
            .values()
            .filter(|c| c.status == ConflictStatus::Open)
            .cloned()
            .collect())
    }
}

#[derive(Debug, Default)]
struct PatternState {
    by_id: HashMap<PatternId, Pattern>,
    by_predicate: HashMap<String, Vec<PatternId>>,
}

/// Thread-safe in-memory pattern store.
#[derive(Debug, Default)]
pub struct InMemoryPatternStore {
    state: RwLock<PatternState>,
}

impl InMemoryPatternStore {
    /// Create a new empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn add_index(state: &mut PatternState, pattern: &Pattern) {
        let mut keys: HashSet<String> = HashSet::new();
        for pred in pattern.rule.indexed_predicates() {
            let pred = pred.trim();
            if pred.is_empty() {
                continue;
            }
            keys.insert(pred.to_string());
        }

        for key in keys {
            state.by_predicate.entry(key).or_default().push(pattern.id);
        }
    }

    fn remove_index(state: &mut PatternState, pattern: &Pattern) {
        let mut keys: HashSet<String> = HashSet::new();
        for pred in pattern.rule.indexed_predicates() {
            let pred = pred.trim();
            if pred.is_empty() {
                continue;
            }
            keys.insert(pred.to_string());
        }

        for key in keys {
            if let Some(ids) = state.by_predicate.get_mut(&key) {
                ids.retain(|id| *id != pattern.id);
                if ids.is_empty() {
                    state.by_predicate.remove(&key);
                }
            }
        }
    }
}

impl PatternStore for InMemoryPatternStore {
    fn insert(&self, pattern: Pattern) -> Result<(), StorageError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| lock_err("pattern.insert"))?;
        if state.by_id.contains_key(&pattern.id) {
            return Err(StorageError::DuplicateKey(pattern.id.to_string()));
        }

        Self::add_index(&mut state, &pattern);
        state.by_id.insert(pattern.id, pattern);
        Ok(())
    }

    fn get(&self, id: PatternId) -> Result<Option<Pattern>, StorageError> {
        let state = self
            .state
            .read()
            .map_err(|_| lock_err("pattern.get"))?;
        Ok(state.by_id.get(&id).cloned())
    }

    fn update(&self, pattern: Pattern) -> Result<(), StorageError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| lock_err("pattern.update"))?;
        let prev = state
            .by_id
            .get(&pattern.id)
            .cloned()
            .ok_or(StorageError::PatternNotFound(pattern.id))?;

        Self::remove_index(&mut state, &prev);
        Self::add_index(&mut state, &pattern);
        state.by_id.insert(pattern.id, pattern);
        Ok(())
    }

    fn delete(&self, id: PatternId) -> Result<(), StorageError> {
        let mut state = self
            .state
            .write()
            .map_err(|_| lock_err("pattern.delete"))?;
        let prev = state
            .by_id
            .remove(&id)
            .ok_or(StorageError::PatternNotFound(id))?;
        Self::remove_index(&mut state, &prev);
        Ok(())
    }

    fn find_by_predicate(&self, predicate: &str) -> Result<Vec<Pattern>, StorageError> {
        let predicate = predicate.trim();
        let state = self
            .state
            .read()
            .map_err(|_| lock_err("pattern.find_by_predicate"))?;
        let Some(ids) = state.by_predicate.get(predicate) else {
            return Ok(Vec::new());
        };

        Ok(ids
            .iter()
            .filter_map(|id| state.by_id.get(id).cloned())
            .collect())
    }

    fn find_active(&self) -> Result<Vec<Pattern>, StorageError> {
        let state = self
            .state
            .read()
            .map_err(|_| lock_err("pattern.find_active"))?;
        Ok(state.by_id.values().filter(|p| p.active).cloned().collect())
    }
}

/// Convenience bundle of in-memory stores.
#[derive(Debug, Default)]
pub struct InMemoryStores {
    /// Entity store.
    pub entities: InMemoryEntityStore,
    /// Belief store.
    pub beliefs: InMemoryBeliefStore,
    /// Pattern store.
    pub patterns: InMemoryPatternStore,
    /// Conflict store.
    pub conflicts: InMemoryConflictStore,
}

impl InMemoryStores {
    /// Create a new bundle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Duration;

    use crate::confidence::Confidence;
    use crate::conflict::ConflictResolution;
    use crate::pattern::{MonotonicDirection, PatternRule};
    use crate::source::Source;
    use crate::value::Value;

    #[test]
    fn entity_insert_get_update_delete_and_name_index() {
        let store = InMemoryEntityStore::new();

        let mut e = Entity::new("Acme Corp", crate::entity::EntityType::Organization);
        e.add_alias("ACME");
        e.add_alias("Acme Corporation");
        e.embedding = Some(vec![1.0, 0.0, 0.0]);
        let id = e.id;

        store.insert(e.clone()).unwrap();
        assert!(matches!(store.insert(e.clone()), Err(StorageError::DuplicateKey(_))));

        let got = store.get(id).unwrap().unwrap();
        assert_eq!(got, e);

        // Exact name lookup is normalized.
        let exact = store.find_by_name("  acme corp ").unwrap();
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].id, id);

        // Fuzzy lookup works against canonical and aliases.
        let fuzzy1 = store.find_by_name_fuzzy("acm", 10).unwrap();
        assert!(fuzzy1.iter().any(|x| x.id == id));
        let fuzzy2 = store.find_by_name_fuzzy("corporation", 10).unwrap();
        assert!(fuzzy2.iter().any(|x| x.id == id));

        // Embedding search matches.
        let emb = store.find_by_embedding(&[1.0, 0.0, 0.0], 10).unwrap();
        assert_eq!(emb.len(), 1);
        assert_eq!(emb[0].0.id, id);
        assert!(emb[0].1 > 0.99);

        // Dimension mismatch is rejected (caller must provide correct dimensionality).
        assert!(matches!(
            store.find_by_embedding(&[1.0, 0.0], 10),
            Err(StorageError::BackendError(_))
        ));

        // Update reindexes by canonical name.
        let mut e2 = got.clone();
        e2.set_canonical_name("Acme Incorporated");
        store.update(e2.clone()).unwrap();
        assert!(store.find_by_name("acme corp").unwrap().is_empty());
        assert_eq!(store.find_by_name("acme incorporated").unwrap()[0].id, id);

        // Delete removes from indexes.
        store.delete(id).unwrap();
        assert!(store.get(id).unwrap().is_none());
        assert!(store.find_by_name("acme incorporated").unwrap().is_empty());
        assert!(matches!(store.delete(id), Err(StorageError::EntityNotFound(_))));
    }

    #[test]
    fn entity_version_history_is_recorded_and_queryable() {
        let store = InMemoryEntityStore::new();

        let mut e = Entity::new("Acme Corp", crate::entity::EntityType::Organization);
        let id = e.id;
        store.insert(e.clone()).unwrap();

        let v1 = store.get_at_version(id, 1).unwrap().unwrap();
        assert_eq!(v1.version, 1);

        e.add_alias("ACME");
        store.update(e.clone()).unwrap();

        let v2 = store.get_at_version(id, 2).unwrap().unwrap();
        assert_eq!(v2.version, 2);

        let versions = store.list_versions(id).unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[1].version, 2);
    }

    #[test]
    fn entity_merge_preserves_history_and_resolves_secondary_id() {
        let store = InMemoryEntityStore::new();

        let mut primary = Entity::new("Acme Corp", crate::entity::EntityType::Organization);
        primary.add_alias("ACME");
        let primary_id = primary.id;

        let mut secondary = Entity::new("Acme Corporation", crate::entity::EntityType::Organization);
        secondary.add_alias("Acme Co.");
        let secondary_id = secondary.id;

        store.insert(primary.clone()).unwrap();
        store.insert(secondary.clone()).unwrap();

        let merged = store.merge(primary_id, secondary_id).unwrap();
        assert_eq!(merged.id, primary_id);
        assert!(merged.aliases.iter().any(|a| a.eq_ignore_ascii_case("acme corporation")));
        assert!(merged.aliases.iter().any(|a| a.eq_ignore_ascii_case("acme co.")));

        let primary_current = store.get(primary_id).unwrap().unwrap();
        assert_eq!(primary_current.id, primary_id);

        let secondary_resolved = store.get(secondary_id).unwrap().unwrap();
        assert_eq!(secondary_resolved.id, primary_id);

        let secondary_versions = store.list_versions(secondary_id).unwrap();
        assert!(!secondary_versions.is_empty());
        let earliest = secondary_versions[0].version;
        assert!(store
            .get_at_version(secondary_id, earliest)
            .unwrap()
            .is_some());
    }

    fn mk_belief(entity_id: EntityId, predicate: &str, value: Value, tx_time: DateTime<Utc>) -> Belief {
        Belief {
            id: BeliefId::new(),
            subject: entity_id,
            predicate: predicate.trim().to_string(),
            value,
            confidence: Confidence::from_agent(0.9, "agent").unwrap(),
            source: Source::agent("agent", Option::<String>::None),
            valid_time: TimeRange::from_now(),
            tx_time,
            reason: None,
            consistency_status: crate::belief::ConsistencyStatus::Provisional,
            supersedes: None,
            superseded_by: None,
            embedding: None,
        }
    }

    #[test]
    fn belief_find_as_of_filters_by_tx_and_valid_time_only() {
        let entities = InMemoryEntityStore::new();
        let beliefs = InMemoryBeliefStore::new();

        let e = Entity::new("Thermometer", crate::entity::EntityType::Artifact);
        let eid = e.id;
        entities.insert(e).unwrap();

        let t0 = Utc::now();
        let t1 = t0 + Duration::seconds(10);
        let t2 = t0 + Duration::seconds(20);

        let mut b1 = mk_belief(eid, "temperature", Value::Float(20.0), t1);
        b1.valid_time = TimeRange::new(t0, t2).unwrap();
        beliefs.insert(b1.clone()).unwrap();

        // As-of before tx_time should not see it.
        let as_of_early = beliefs.find_as_of(eid, "temperature", t0 + Duration::seconds(1)).unwrap();
        assert!(as_of_early.is_empty());

        // As-of within valid_time and after tx_time sees it.
        let as_of_mid = beliefs.find_as_of(eid, "temperature", t1 + Duration::seconds(1)).unwrap();
        assert_eq!(as_of_mid.len(), 1);
        assert_eq!(as_of_mid[0].id, b1.id);

        // As-of after valid_time end does not see it.
        let as_of_late = beliefs.find_as_of(eid, "temperature", t2 + Duration::seconds(1)).unwrap();
        assert!(as_of_late.is_empty());
    }

    #[test]
    fn belief_supersede_invariants_and_valid_time_clamp() {
        let beliefs = InMemoryBeliefStore::new();
        let eid = EntityId::new();
        let base = Utc::now();

        let mut old = mk_belief(eid, "status", Value::String("old".to_string()), base);
        old.valid_time = TimeRange::starting_at(base);
        let old_id = old.id;
        beliefs.insert(old).unwrap();

        let new = mk_belief(eid, "status", Value::String("new".to_string()), base + Duration::seconds(10));
        let new_id = new.id;
        beliefs.insert(new).unwrap();

        // Self supersede is rejected.
        assert!(beliefs.supersede(old_id, old_id).is_err());

        // Supersede is applied.
        beliefs.supersede(old_id, new_id).unwrap();
        let old_after = beliefs.get(old_id).unwrap().unwrap();
        assert_eq!(old_after.superseded_by, Some(new_id));
        assert!(old_after.valid_time.to().is_some());

        let new_after = beliefs.get(new_id).unwrap().unwrap();
        assert_eq!(new_after.supersedes, Some(old_id));

        // Idempotent supersession.
        beliefs.supersede(old_id, new_id).unwrap();

        // Different superseding belief is rejected.
        let newer = mk_belief(eid, "status", Value::String("newer".to_string()), base + Duration::seconds(20));
        let newer_id = newer.id;
        beliefs.insert(newer).unwrap();
        assert!(beliefs.supersede(old_id, newer_id).is_err());

        // Attempting to make a belief supersede two different olds is rejected.
        let other_old = mk_belief(eid, "status", Value::String("other".to_string()), base);
        let other_old_id = other_old.id;
        beliefs.insert(other_old).unwrap();
        assert!(beliefs.supersede(other_old_id, new_id).is_err());
    }

    #[test]
    fn conflict_store_indexes_and_find_open() {
        let store = InMemoryConflictStore::new();
        let b1 = BeliefId::new();
        let b2 = BeliefId::new();
        let eid = EntityId::new();

        let mut c = Conflict::value_contradiction(vec![b1, b2], eid, "predicate");
        let cid = c.id;

        store.insert(c.clone()).unwrap();
        assert!(matches!(store.insert(c.clone()), Err(StorageError::DuplicateKey(_))));

        let by_b1 = store.find_by_belief(b1).unwrap();
        assert_eq!(by_b1.len(), 1);
        assert_eq!(by_b1[0].id, cid);

        let open = store.find_open().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, cid);

        c.resolve(ConflictResolution::Accepted {
            reason: "coexistence allowed".to_string(),
        });
        store.update(c).unwrap();
        assert!(store.find_open().unwrap().is_empty());
    }

    #[test]
    fn conflict_update_reindexes_by_belief_ids() {
        let store = InMemoryConflictStore::new();
        let b1 = BeliefId::new();
        let b2 = BeliefId::new();
        let eid = EntityId::new();

        let mut c = Conflict::value_contradiction(vec![b1], eid, "p");
        let cid = c.id;
        store.insert(c.clone()).unwrap();

        assert_eq!(store.find_by_belief(b1).unwrap().len(), 1);
        assert!(store.find_by_belief(b2).unwrap().is_empty());

        c.belief_ids = vec![b2];
        store.update(c).unwrap();

        assert!(store.find_by_belief(b1).unwrap().is_empty());
        let by_b2 = store.find_by_belief(b2).unwrap();
        assert_eq!(by_b2.len(), 1);
        assert_eq!(by_b2[0].id, cid);
    }

    #[test]
    fn pattern_store_primary_predicate_index_update_delete() {
        let store = InMemoryPatternStore::new();

        let mut p = Pattern::new(
            "temp_range",
            PatternRule::Range {
                predicate: "temperature".to_string(),
                min: Some(-50.0),
                max: Some(150.0),
            },
            Confidence::from_agent(0.8, "agent").unwrap(),
        );
        p.active = true;
        let pid = p.id;
        store.insert(p.clone()).unwrap();

        let found = store.find_by_predicate("temperature").unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, pid);
        assert_eq!(store.find_active().unwrap().len(), 1);

        // Update should move index when primary predicate changes.
        p.rule = PatternRule::Monotonic {
            predicate: "pressure".to_string(),
            direction: MonotonicDirection::Increasing,
        };
        store.update(p.clone()).unwrap();
        assert!(store.find_by_predicate("temperature").unwrap().is_empty());
        let found2 = store.find_by_predicate("pressure").unwrap();
        assert_eq!(found2.len(), 1);
        assert_eq!(found2[0].id, pid);

        // Delete removes index.
        store.delete(pid).unwrap();
        assert!(store.find_by_predicate("pressure").unwrap().is_empty());
        assert!(matches!(store.delete(pid), Err(StorageError::PatternNotFound(_))));
    }
}
