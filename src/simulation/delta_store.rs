//! Delta store overlay for simulations.
//!
//! Key invariants:
//! - Writes must never reach the base stores.
//! - Reads merge base + delta results (merge-on-read).
//! - The delta overlay is bounded by `SimulateConstraints`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};

use crate::belief::Belief;
use crate::confidence::BeliefId;
use crate::entity::{Entity, EntityId};
use crate::pattern::{Pattern, PatternId};
use crate::storage::{BeliefStore, ConflictStore, EntityStore, PatternStore, StorageError};
use crate::time::TimeRange;

use super::constraints::SimulateConstraints;
use super::delta_index::DeltaVectorIndex;
use super::SimulationBaseStores;

fn ro_err(op: &'static str) -> StorageError {
    StorageError::BackendError(format!("simulation store is read-only: {op}"))
}

#[derive(Debug, Default)]
struct DeltaBeliefState {
    inserted: HashMap<BeliefId, Belief>,
    affected_entities: HashSet<EntityId>,
    superseded: HashMap<BeliefId, BeliefId>,
    index: DeltaVectorIndex,
}

/// Read-only wrapper for `EntityStore`.
#[derive(Clone)]
pub struct ReadOnlyEntityStore {
    base: Arc<dyn EntityStore>,
}

impl ReadOnlyEntityStore {
    fn new(base: Arc<dyn EntityStore>) -> Self {
        Self { base }
    }
}

impl EntityStore for ReadOnlyEntityStore {
    fn insert(&self, _entity: Entity) -> Result<(), StorageError> {
        Err(ro_err("entity.insert"))
    }

    fn get(&self, id: EntityId) -> Result<Option<Entity>, StorageError> {
        self.base.get(id)
    }

    fn update(&self, _entity: Entity) -> Result<(), StorageError> {
        Err(ro_err("entity.update"))
    }

    fn delete(&self, _id: EntityId) -> Result<(), StorageError> {
        Err(ro_err("entity.delete"))
    }

    fn find_by_name(&self, name: &str) -> Result<Vec<Entity>, StorageError> {
        self.base.find_by_name(name)
    }

    fn find_by_name_fuzzy(&self, query: &str, limit: usize) -> Result<Vec<Entity>, StorageError> {
        self.base.find_by_name_fuzzy(query, limit)
    }

    fn find_by_embedding(&self, embedding: &[f32], limit: usize) -> Result<Vec<(Entity, f32)>, StorageError> {
        self.base.find_by_embedding(embedding, limit)
    }

    fn merge(&self, _primary: EntityId, _secondary: EntityId) -> Result<Entity, StorageError> {
        Err(ro_err("entity.merge"))
    }

    fn get_at_version(&self, id: EntityId, version: u64) -> Result<Option<Entity>, StorageError> {
        self.base.get_at_version(id, version)
    }

    fn list_versions(&self, id: EntityId) -> Result<Vec<Entity>, StorageError> {
        self.base.list_versions(id)
    }
}

/// Read-only wrapper for `PatternStore`.
#[derive(Clone)]
pub struct ReadOnlyPatternStore {
    base: Arc<dyn PatternStore>,
}

impl ReadOnlyPatternStore {
    fn new(base: Arc<dyn PatternStore>) -> Self {
        Self { base }
    }
}

impl PatternStore for ReadOnlyPatternStore {
    fn insert(&self, _pattern: Pattern) -> Result<(), StorageError> {
        Err(ro_err("pattern.insert"))
    }

    fn get(&self, id: PatternId) -> Result<Option<Pattern>, StorageError> {
        self.base.get(id)
    }

    fn update(&self, _pattern: Pattern) -> Result<(), StorageError> {
        Err(ro_err("pattern.update"))
    }

    fn delete(&self, _id: PatternId) -> Result<(), StorageError> {
        Err(ro_err("pattern.delete"))
    }

    fn find_by_predicate(&self, predicate: &str) -> Result<Vec<Pattern>, StorageError> {
        self.base.find_by_predicate(predicate)
    }

    fn find_active(&self) -> Result<Vec<Pattern>, StorageError> {
        self.base.find_active()
    }
}

/// Read-only wrapper for `ConflictStore`.
#[derive(Clone)]
pub struct ReadOnlyConflictStore {
    base: Arc<dyn ConflictStore>,
}

impl ReadOnlyConflictStore {
    fn new(base: Arc<dyn ConflictStore>) -> Self {
        Self { base }
    }
}

impl ConflictStore for ReadOnlyConflictStore {
    fn insert(&self, _conflict: crate::conflict::Conflict) -> Result<(), StorageError> {
        Err(ro_err("conflict.insert"))
    }

    fn get(&self, id: crate::conflict::ConflictId) -> Result<Option<crate::conflict::Conflict>, StorageError> {
        self.base.get(id)
    }

    fn update(&self, _conflict: crate::conflict::Conflict) -> Result<(), StorageError> {
        Err(ro_err("conflict.update"))
    }

    fn find_by_belief(&self, belief_id: BeliefId) -> Result<Vec<crate::conflict::Conflict>, StorageError> {
        self.base.find_by_belief(belief_id)
    }

    fn find_open(&self) -> Result<Vec<crate::conflict::Conflict>, StorageError> {
        self.base.find_open()
    }
}

/// Read-only wrapper for `BeliefStore`.
///
/// This is used to ensure simulation overlays cannot accidentally mutate their base,
/// including when nesting a simulation on top of another simulation.
#[derive(Clone)]
pub struct ReadOnlyBeliefStore {
    base: Arc<dyn BeliefStore>,
}

impl ReadOnlyBeliefStore {
    fn new(base: Arc<dyn BeliefStore>) -> Self {
        Self { base }
    }
}

impl BeliefStore for ReadOnlyBeliefStore {
    fn insert(&self, _belief: Belief) -> Result<(), StorageError> {
        Err(ro_err("belief.insert"))
    }

    fn get(&self, id: BeliefId) -> Result<Option<Belief>, StorageError> {
        self.base.get(id)
    }

    fn supersede(&self, _old_id: BeliefId, _new_id: BeliefId) -> Result<(), StorageError> {
        Err(ro_err("belief.supersede"))
    }

    fn find_by_entity_predicate(
        &self,
        entity_id: EntityId,
        predicate: &str,
    ) -> Result<Vec<Belief>, StorageError> {
        self.base.find_by_entity_predicate(entity_id, predicate)
    }

    fn find_as_of(
        &self,
        entity_id: EntityId,
        predicate: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Vec<Belief>, StorageError> {
        self.base.find_as_of(entity_id, predicate, as_of)
    }

    fn find_by_time_range(&self, range: &TimeRange) -> Result<Vec<Belief>, StorageError> {
        self.base.find_by_time_range(range)
    }

    fn find_by_embedding(
        &self,
        embedding: &[f32],
        limit: usize,
        min_confidence: Option<f32>,
    ) -> Result<Vec<(Belief, f32)>, StorageError> {
        self.base.find_by_embedding(embedding, limit, min_confidence)
    }

    fn count_by_entity(&self, entity_id: EntityId) -> Result<usize, StorageError> {
        self.base.count_by_entity(entity_id)
    }
}

/// Belief store overlay: writes land in-memory, reads merge base+delta.
pub struct DeltaBeliefStore {
    base: Arc<dyn BeliefStore>,
    constraints: SimulateConstraints,
    state: RwLock<DeltaBeliefState>,
}

impl DeltaBeliefStore {
    fn new(base: Arc<dyn BeliefStore>, constraints: SimulateConstraints) -> Self {
        Self {
            base,
            constraints,
            state: RwLock::new(DeltaBeliefState::default()),
        }
    }

    fn clear(&self) {
        let mut guard = match self.state.write() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = DeltaBeliefState::default();
    }

    fn record_affected_entity(state: &mut DeltaBeliefState, entity: EntityId, max: usize) -> Result<(), StorageError> {
        if state.affected_entities.contains(&entity) {
            return Ok(());
        }

        if state.affected_entities.len() >= max {
            return Err(StorageError::BackendError(format!(
                "simulation affected entity limit exceeded: max={} actual={} (next={})",
                max,
                state.affected_entities.len(),
                entity
            )));
        }

        state.affected_entities.insert(entity);
        Ok(())
    }

    fn merge_beliefs(&self, mut base: Vec<Belief>, predicate: &str) -> Result<Vec<Belief>, StorageError> {
        // Merge overlay beliefs that match predicate.
        let state = self
            .state
            .read()
            .map_err(|_| StorageError::BackendError("poisoned lock: delta_beliefs.read".to_string()))?;

        for belief in state.inserted.values() {
            if belief.predicate == predicate {
                base.push(belief.clone());
            }
        }

        // Apply supersede markers (best-effort; does not mutate base storage).
        for belief in &mut base {
            if let Some(new_id) = state.superseded.get(&belief.id).copied() {
                belief.superseded_by = Some(new_id);
            }
        }

        Ok(base)
    }
}

impl BeliefStore for DeltaBeliefStore {
    fn insert(&self, belief: Belief) -> Result<(), StorageError> {
        let mut guard = self
            .state
            .write()
            .map_err(|_| StorageError::BackendError("poisoned lock: delta_beliefs.insert".to_string()))?;

        if guard.inserted.contains_key(&belief.id) {
            return Err(StorageError::DuplicateKey(belief.id.to_string()));
        }

        // If the base has the ID already, treat as duplicate.
        if self.base.get(belief.id)?.is_some() {
            return Err(StorageError::DuplicateKey(belief.id.to_string()));
        }

        Self::record_affected_entity(
            &mut guard,
            belief.subject,
            self.constraints.max_affected_entities,
        )?;

        if let Some(embedding) = belief.embedding.as_ref() {
            guard
                .index
                .upsert(belief.id, embedding, belief.confidence.value())?;
        }

        guard.inserted.insert(belief.id, belief);
        Ok(())
    }

    fn get(&self, id: BeliefId) -> Result<Option<Belief>, StorageError> {
        let state = self
            .state
            .read()
            .map_err(|_| StorageError::BackendError("poisoned lock: delta_beliefs.get".to_string()))?;
        if let Some(b) = state.inserted.get(&id) {
            return Ok(Some(b.clone()));
        }

        let mut b = self.base.get(id)?;
        if let Some(ref mut belief) = b {
            if let Some(new_id) = state.superseded.get(&id).copied() {
                belief.superseded_by = Some(new_id);
            }
        }
        Ok(b)
    }

    fn supersede(&self, old_id: BeliefId, new_id: BeliefId) -> Result<(), StorageError> {
        let mut guard = self
            .state
            .write()
            .map_err(|_| StorageError::BackendError("poisoned lock: delta_beliefs.supersede".to_string()))?;
        guard.superseded.insert(old_id, new_id);
        Ok(())
    }

    fn find_by_entity_predicate(&self, entity_id: EntityId, predicate: &str) -> Result<Vec<Belief>, StorageError> {
        let base = self.base.find_by_entity_predicate(entity_id, predicate)?;
        let mut merged = self.merge_beliefs(base, predicate)?;
        merged.retain(|b| b.subject == entity_id);
        Ok(merged)
    }

    fn find_as_of(
        &self,
        entity_id: EntityId,
        predicate: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Vec<Belief>, StorageError> {
        let base = self.base.find_as_of(entity_id, predicate, as_of)?;
        let mut merged = self.merge_beliefs(base, predicate)?;
        merged.retain(|b| b.subject == entity_id && b.valid_time.contains(as_of));
        Ok(merged)
    }

    fn find_by_time_range(&self, range: &TimeRange) -> Result<Vec<Belief>, StorageError> {
        let mut out = self.base.find_by_time_range(range)?;

        let state = self
            .state
            .read()
            .map_err(|_| StorageError::BackendError("poisoned lock: delta_beliefs.find_by_time_range".to_string()))?;
        for belief in state.inserted.values() {
            if belief.valid_time.overlaps(range) {
                out.push(belief.clone());
            }
        }

        for belief in &mut out {
            if let Some(new_id) = state.superseded.get(&belief.id).copied() {
                belief.superseded_by = Some(new_id);
            }
        }

        Ok(out)
    }

    fn find_by_embedding(
        &self,
        embedding: &[f32],
        limit: usize,
        min_confidence: Option<f32>,
    ) -> Result<Vec<(Belief, f32)>, StorageError> {
        let mut out = self.base.find_by_embedding(embedding, limit, min_confidence)?;

        let state = self
            .state
            .read()
            .map_err(|_| StorageError::BackendError("poisoned lock: delta_beliefs.find_by_embedding".to_string()))?;

        let hits = state.index.search(embedding, limit, min_confidence)?;
        for (id, sim) in hits {
            if let Some(belief) = state.inserted.get(&id) {
                out.push((belief.clone(), sim));
            }
        }

        for (belief, _) in &mut out {
            if let Some(new_id) = state.superseded.get(&belief.id).copied() {
                belief.superseded_by = Some(new_id);
            }
        }

        out.sort_by(|(a, sa), (b, sb)| {
            sb.partial_cmp(sa)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.confidence.value().partial_cmp(&a.confidence.value()).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| a.id.to_string().cmp(&b.id.to_string()))
        });

        out.truncate(limit.max(1));
        Ok(out)
    }

    fn count_by_entity(&self, entity_id: EntityId) -> Result<usize, StorageError> {
        let base = self.base.count_by_entity(entity_id)?;
        let state = self
            .state
            .read()
            .map_err(|_| StorageError::BackendError("poisoned lock: delta_beliefs.count_by_entity".to_string()))?;
        let delta = state
            .inserted
            .values()
            .filter(|b| b.subject == entity_id)
            .count();
        Ok(base + delta)
    }
}

/// Bundle of overlay stores for a simulation.
pub struct DeltaStore {
    entities: Arc<ReadOnlyEntityStore>,
    beliefs: Arc<DeltaBeliefStore>,
    patterns: Arc<ReadOnlyPatternStore>,
    conflicts: Arc<ReadOnlyConflictStore>,
}

impl DeltaStore {
    /// Create a delta store overlay.
    pub fn new(base: SimulationBaseStores, constraints: SimulateConstraints) -> Self {
        let ro_beliefs: Arc<dyn BeliefStore> = Arc::new(ReadOnlyBeliefStore::new(base.beliefs));
        Self {
            entities: Arc::new(ReadOnlyEntityStore::new(base.entities)),
            beliefs: Arc::new(DeltaBeliefStore::new(ro_beliefs, constraints)),
            patterns: Arc::new(ReadOnlyPatternStore::new(base.patterns)),
            conflicts: Arc::new(ReadOnlyConflictStore::new(base.conflicts)),
        }
    }

    /// Access the entity store (read-only wrapper).
    #[must_use]
    pub fn entities(&self) -> Arc<dyn EntityStore> {
        self.entities.clone()
    }

    /// Access the belief store (delta overlay).
    #[must_use]
    pub fn beliefs(&self) -> Arc<dyn BeliefStore> {
        self.beliefs.clone()
    }

    /// Access the pattern store (read-only wrapper).
    #[must_use]
    pub fn patterns(&self) -> Arc<dyn PatternStore> {
        self.patterns.clone()
    }

    /// Access the conflict store (read-only wrapper).
    #[must_use]
    pub fn conflicts(&self) -> Arc<dyn ConflictStore> {
        self.conflicts.clone()
    }

    /// Return a snapshot of overlay impact.
    ///
    /// This is derived solely from delta state and does not touch base stores.
    pub fn impact_snapshot(&self) -> Result<(Vec<EntityId>, usize), StorageError> {
        let state = match self.beliefs.state.read() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        let mut entities: Vec<_> = state.affected_entities.iter().copied().collect();
        entities.sort_by(|a, b| a.to_string().cmp(&b.to_string()));
        Ok((entities, state.inserted.len()))
    }

    /// Return a richer snapshot of overlay impact.
    ///
    /// Includes belief-level diffs (inserted belief IDs and supersede pairs).
    pub fn impact_details(
        &self,
    ) -> Result<(Vec<EntityId>, Vec<BeliefId>, Vec<(BeliefId, BeliefId)>), StorageError> {
        let state = match self.beliefs.state.read() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        let mut entities: Vec<_> = state.affected_entities.iter().copied().collect();
        entities.sort_by(|a, b| a.to_string().cmp(&b.to_string()));

        let mut inserted_belief_ids: Vec<_> = state.inserted.keys().copied().collect();
        inserted_belief_ids.sort_by(|a, b| a.to_string().cmp(&b.to_string()));

        let mut supersedes: Vec<_> = state.superseded.iter().map(|(o, n)| (*o, *n)).collect();
        supersedes.sort_by(|(o1, n1), (o2, n2)| {
            match o1.to_string().cmp(&o2.to_string()) {
                std::cmp::Ordering::Equal => n1.to_string().cmp(&n2.to_string()),
                other => other,
            }
        });

        Ok((entities, inserted_belief_ids, supersedes))
    }

    /// Return a deterministic snapshot of the overlay beliefs.
    ///
    /// This is intended for "commit overlay" workflows that graduate a simulation's delta into
    /// base storage. The returned values are derived solely from delta state and do not touch
    /// base stores.
    pub fn overlay_snapshot(&self) -> Result<(Vec<Belief>, Vec<(BeliefId, BeliefId)>), StorageError> {
        let state = match self.beliefs.state.read() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };

        let mut beliefs: Vec<Belief> = state.inserted.values().cloned().collect();
        beliefs.sort_by(|a, b| a.id.to_string().cmp(&b.id.to_string()));

        let mut supersedes: Vec<(BeliefId, BeliefId)> = state.superseded.iter().map(|(o, n)| (*o, *n)).collect();
        supersedes.sort_by(|(o1, n1), (o2, n2)| {
            match o1.to_string().cmp(&o2.to_string()) {
                std::cmp::Ordering::Equal => n1.to_string().cmp(&n2.to_string()),
                other => other,
            }
        });

        Ok((beliefs, supersedes))
    }

    /// Clear all overlay state.
    pub fn clear(&mut self) {
        self.beliefs.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::belief::{ConsistencyStatus};
    use crate::confidence::Confidence;
    use crate::source::Source;
    use crate::storage::InMemoryStores;
    use crate::value::Value;

    #[test]
    fn delta_store_write_isolation() {
        let stores = InMemoryStores::default();

        let entity = crate::entity::Entity::new("sim_entity", crate::entity::EntityType::Artifact);
        let entity_id = entity.id;
        stores.entities.insert(entity).unwrap();

        let base = SimulationBaseStores {
            entities: Arc::new(stores.entities),
            beliefs: Arc::new(stores.beliefs),
            patterns: Arc::new(stores.patterns),
            conflicts: Arc::new(stores.conflicts),
        };

        let delta = DeltaStore::new(base.clone(), SimulateConstraints::default());

        // Read-only store refuses writes.
        let err = delta
            .entities()
            .insert(crate::entity::Entity::new("x", crate::entity::EntityType::Artifact))
            .unwrap_err();
        assert!(matches!(err, StorageError::BackendError(_)));

        let before = base.beliefs.count_by_entity(entity_id).unwrap();

        // Insert hypothetical belief into delta.
        let belief = Belief {
            id: BeliefId::new(),
            subject: entity_id,
            predicate: "temperature".to_string(),
            value: Value::Float(1.0),
            confidence: Confidence::from_agent(0.9, "sim").unwrap(),
            source: Source::Unknown { description: None },
            valid_time: TimeRange::from_now(),
            tx_time: Utc::now(),
            reason: None,
            consistency_status: ConsistencyStatus::Provisional,
            supersedes: None,
            superseded_by: None,
            embedding: None,
        };

        delta.beliefs().insert(belief.clone()).unwrap();

        // Base unchanged.
        let after = base.beliefs.count_by_entity(entity_id).unwrap();
        assert_eq!(before, after);

        // But delta read sees the belief.
        let got = delta.beliefs().get(belief.id).unwrap().unwrap();
        assert_eq!(got.id, belief.id);
    }

    #[test]
    fn delta_store_enforces_max_affected_entities() {
        let stores = InMemoryStores::default();
        let base = SimulationBaseStores {
            entities: Arc::new(stores.entities),
            beliefs: Arc::new(stores.beliefs),
            patterns: Arc::new(stores.patterns),
            conflicts: Arc::new(stores.conflicts),
        };

        let constraints = SimulateConstraints {
            max_affected_entities: 1,
            max_depth: 1,
            max_duration_ms: 500,
        };

        let delta = DeltaStore::new(base, constraints);

        let e1 = crate::entity::Entity::new("e1", crate::entity::EntityType::Artifact);
        let e2 = crate::entity::Entity::new("e2", crate::entity::EntityType::Artifact);

        // Base entity store is separate; we only need subjects for constraint counting.
        let b1 = Belief {
            id: BeliefId::new(),
            subject: e1.id,
            predicate: "p".to_string(),
            value: Value::Bool(true),
            confidence: Confidence::from_agent(0.9, "sim").unwrap(),
            source: Source::Unknown { description: None },
            valid_time: TimeRange::from_now(),
            tx_time: Utc::now(),
            reason: None,
            consistency_status: ConsistencyStatus::Provisional,
            supersedes: None,
            superseded_by: None,
            embedding: None,
        };

        delta.beliefs().insert(b1).unwrap();

        let b2 = Belief {
            id: BeliefId::new(),
            subject: e2.id,
            predicate: "p".to_string(),
            value: Value::Bool(true),
            confidence: Confidence::from_agent(0.9, "sim").unwrap(),
            source: Source::Unknown { description: None },
            valid_time: TimeRange::from_now(),
            tx_time: Utc::now(),
            reason: None,
            consistency_status: ConsistencyStatus::Provisional,
            supersedes: None,
            superseded_by: None,
            embedding: None,
        };

        let err = delta.beliefs().insert(b2).unwrap_err();
        assert!(matches!(err, StorageError::BackendError(_)));
    }
}
