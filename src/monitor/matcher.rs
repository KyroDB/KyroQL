//! Trigger matching for the MONITOR subsystem.
//!
//! The matcher evaluates triggers against committed ASSERT observations.
//! Expensive lookups are performed off the ASSERT path.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::confidence::BeliefId;
use crate::conflict::ConflictType;
use crate::error::{ExecutionError, KyroError, KyroResult};
use crate::pattern::PatternId;
use crate::storage::BeliefStore;
use crate::value::Value;

use super::triggers::{EventPayload, Trigger};

#[allow(missing_docs)]
#[derive(Debug, Clone)]
pub struct AssertObservation {
    pub tx_time: DateTime<Utc>,
    pub belief_id: BeliefId,
    pub entity_id: crate::entity::EntityId,
    pub predicate: String,
    pub value: Value,
    pub confidence: f32,
    pub conflict_types: Vec<ConflictType>,
}

#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq)]
pub enum MatchOutput {
    NoMatch,
    Match(EventPayload),
}

#[allow(missing_docs)]
pub struct TriggerMatcher {
    beliefs: std::sync::Arc<dyn BeliefStore>,
}

impl TriggerMatcher {
    #[must_use]
    pub fn new(beliefs: std::sync::Arc<dyn BeliefStore>) -> Self {
        Self { beliefs }
    }

    pub fn evaluate(&self, trigger: &Trigger, obs: &AssertObservation) -> KyroResult<MatchOutput> {
        match trigger {
            Trigger::ConfidenceShift {
                entity_id,
                predicate,
                threshold,
            } => self.match_confidence_shift(*entity_id, predicate.as_deref(), *threshold, obs),

            Trigger::ConflictCreated {
                entity_id,
                conflict_types,
            } => self.match_conflict_created(*entity_id, conflict_types, obs),

            Trigger::PatternViolation { pattern_id } => {
                self.match_pattern_violation(*pattern_id, obs)
            }

            Trigger::EntropySpike { domain, threshold } => {
                self.match_entropy_spike(domain, *threshold, obs)
            }

            Trigger::GapFilled { entity_id, predicate } => {
                self.match_gap_filled(*entity_id, predicate, obs)
            }
        }
    }

    fn match_confidence_shift(
        &self,
        entity_id_filter: Option<crate::entity::EntityId>,
        predicate_filter: Option<&str>,
        threshold: f32,
        obs: &AssertObservation,
    ) -> KyroResult<MatchOutput> {
        if threshold <= 0.0 {
            return Ok(MatchOutput::NoMatch);
        }

        if let Some(eid) = entity_id_filter {
            if eid != obs.entity_id {
                return Ok(MatchOutput::NoMatch);
            }
        }
        if let Some(pred) = predicate_filter {
            let pred = pred.trim();
            if pred.is_empty() || pred != obs.predicate {
                return Ok(MatchOutput::NoMatch);
            }
        }

        let existing = self
            .beliefs
            .find_by_entity_predicate(obs.entity_id, &obs.predicate)
            .map_err(|e| KyroError::Execution(ExecutionError::Storage {
                message: e.to_string(),
            }))?;

        let mut prev: Option<f32> = None;
        for b in existing {
            if b.id == obs.belief_id {
                continue;
            }
            if b.tx_time >= obs.tx_time {
                continue;
            }
            let v = b.confidence.value();
            prev = Some(match prev {
                None => v,
                Some(p) => p.max(v),
            });
        }

        let Some(previous) = prev else {
            return Ok(MatchOutput::NoMatch);
        };

        let current = obs.confidence;
        let delta = (current - previous).abs();
        if delta > threshold {
            Ok(MatchOutput::Match(EventPayload::ConfidenceShift {
                belief_id: obs.belief_id,
                entity_id: obs.entity_id,
                predicate: obs.predicate.clone(),
                previous,
                current,
                delta,
            }))
        } else {
            Ok(MatchOutput::NoMatch)
        }
    }

    fn match_conflict_created(
        &self,
        entity_id_filter: Option<crate::entity::EntityId>,
        conflict_types_filter: &[ConflictType],
        obs: &AssertObservation,
    ) -> KyroResult<MatchOutput> {
        if obs.conflict_types.is_empty() {
            return Ok(MatchOutput::NoMatch);
        }

        if let Some(eid) = entity_id_filter {
            if eid != obs.entity_id {
                return Ok(MatchOutput::NoMatch);
            }
        }

        let matches = if conflict_types_filter.is_empty() {
            true
        } else {
            obs.conflict_types
                .iter()
                .any(|c| conflict_types_filter.contains(c))
        };

        if matches {
            Ok(MatchOutput::Match(EventPayload::ConflictCreated {
                belief_id: obs.belief_id,
                entity_id: obs.entity_id,
                predicate: obs.predicate.clone(),
                conflict_types: obs.conflict_types.clone(),
            }))
        } else {
            Ok(MatchOutput::NoMatch)
        }
    }

    fn match_pattern_violation(
        &self,
        pattern_id: PatternId,
        obs: &AssertObservation,
    ) -> KyroResult<MatchOutput> {
        for c in &obs.conflict_types {
            let ConflictType::PatternViolation { pattern_id: pid, .. } = c else {
                continue;
            };
            if pid == &pattern_id.to_string() {
                return Ok(MatchOutput::Match(EventPayload::PatternViolation {
                    belief_id: obs.belief_id,
                    entity_id: obs.entity_id,
                    predicate: obs.predicate.clone(),
                    pattern_id,
                }));
            }
        }
        Ok(MatchOutput::NoMatch)
    }

    fn match_gap_filled(
        &self,
        entity_id: crate::entity::EntityId,
        predicate: &str,
        obs: &AssertObservation,
    ) -> KyroResult<MatchOutput> {
        if entity_id != obs.entity_id {
            return Ok(MatchOutput::NoMatch);
        }
        if predicate.trim() != obs.predicate {
            return Ok(MatchOutput::NoMatch);
        }
        if matches!(obs.value, Value::Null) {
            return Ok(MatchOutput::NoMatch);
        }

        let existing = self
            .beliefs
            .find_by_entity_predicate(obs.entity_id, &obs.predicate)
            .map_err(|e| KyroError::Execution(ExecutionError::Storage {
                message: e.to_string(),
            }))?;

        let mut had_data = false;
        for b in existing {
            if b.id == obs.belief_id {
                continue;
            }
            if b.tx_time >= obs.tx_time {
                continue;
            }
            if !matches!(b.value, Value::Null) {
                had_data = true;
                break;
            }
        }

        if !had_data {
            Ok(MatchOutput::Match(EventPayload::GapFilled {
                belief_id: obs.belief_id,
                entity_id: obs.entity_id,
                predicate: obs.predicate.clone(),
            }))
        } else {
            Ok(MatchOutput::NoMatch)
        }
    }

    fn match_entropy_spike(
        &self,
        domain: &str,
        threshold_bits: f32,
        obs: &AssertObservation,
    ) -> KyroResult<MatchOutput> {
        let domain = domain.trim();
        if domain.is_empty() {
            return Ok(MatchOutput::NoMatch);
        }
        if obs.predicate != domain {
            return Ok(MatchOutput::NoMatch);
        }
        if threshold_bits <= 0.0 {
            return Ok(MatchOutput::NoMatch);
        }

        let mut beliefs = self
            .beliefs
            .find_as_of(obs.entity_id, &obs.predicate, obs.tx_time)
            .map_err(|e| KyroError::Execution(ExecutionError::Storage {
                message: e.to_string(),
            }))?;

        // Ensure we include the new belief if the store backend excludes it due to
        // timestamp granularity; we still treat `obs` as part of the AS-OF set.
        if !beliefs.iter().any(|b| b.id == obs.belief_id) {
            // Fall back to entity+predicate scan if needed.
            let all = self
                .beliefs
                .find_by_entity_predicate(obs.entity_id, &obs.predicate)
                .map_err(|e| KyroError::Execution(ExecutionError::Storage {
                    message: e.to_string(),
                }))?;
            for b in all {
                if b.id == obs.belief_id {
                    beliefs.push(b);
                    break;
                }
            }
        }

        let mut mass_by_value: HashMap<String, f64> = HashMap::new();
        for b in beliefs {
            if !b.is_valid_at(obs.tx_time) {
                continue;
            }
            if matches!(b.value, Value::Null) {
                continue;
            }
            let key = b.value.to_string();
            let entry = mass_by_value.entry(key).or_insert(0.0);
            *entry += b.confidence.value() as f64;
        }

        if mass_by_value.len() < 2 {
            return Ok(MatchOutput::NoMatch);
        }

        let total: f64 = mass_by_value.values().sum();
        if total <= 0.0 {
            return Ok(MatchOutput::NoMatch);
        }

        let mut entropy_bits = 0.0f64;
        for &m in mass_by_value.values() {
            if m <= 0.0 {
                continue;
            }
            let p = m / total;
            entropy_bits -= p * p.log2();
        }

        if !entropy_bits.is_finite() {
            return Ok(MatchOutput::NoMatch);
        }

        let entropy_bits_f32 = entropy_bits as f32;
        if entropy_bits_f32 > threshold_bits {
            Ok(MatchOutput::Match(EventPayload::EntropySpike {
                belief_id: obs.belief_id,
                entity_id: obs.entity_id,
                predicate: obs.predicate.clone(),
                entropy_bits: entropy_bits_f32,
                threshold_bits,
            }))
        } else {
            Ok(MatchOutput::NoMatch)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::belief::Belief;
    use crate::confidence::Confidence;
    use crate::source::Source;
    use crate::storage::InMemoryBeliefStore;
    use crate::time::TimeRange;

    fn belief_with(
        id: BeliefId,
        entity_id: crate::entity::EntityId,
        predicate: &str,
        value: Value,
        conf: f32,
        tx_time: DateTime<Utc>,
    ) -> Belief {
        Belief {
            id,
            subject: entity_id,
            predicate: predicate.to_string(),
            value,
            confidence: Confidence::from_agent(conf, "t").unwrap(),
            source: Source::Unknown { description: None },
            valid_time: TimeRange::from_now(),
            tx_time,
            reason: None,
            consistency_status: crate::belief::ConsistencyStatus::Verified,
            supersedes: None,
            superseded_by: None,
            embedding: None,
        }
    }

    #[test]
    fn confidence_shift_requires_prior() {
        let store: Arc<dyn BeliefStore> = Arc::new(InMemoryBeliefStore::new());
        let matcher = TriggerMatcher::new(Arc::clone(&store));

        let eid = crate::entity::EntityId::new();
        let now = Utc::now();
        let obs = AssertObservation {
            tx_time: now,
            belief_id: BeliefId::new(),
            entity_id: eid,
            predicate: "p".to_string(),
            value: Value::Int(1),
            confidence: 0.9,
            conflict_types: Vec::new(),
        };

        let out = matcher
            .evaluate(
                &Trigger::ConfidenceShift {
                    entity_id: Some(eid),
                    predicate: Some("p".to_string()),
                    threshold: 0.1,
                },
                &obs,
            )
            .unwrap();

        assert_eq!(out, MatchOutput::NoMatch);
    }

    #[test]
    fn confidence_shift_fires_on_delta() {
        let store: Arc<dyn BeliefStore> = Arc::new(InMemoryBeliefStore::new());
        let matcher = TriggerMatcher::new(Arc::clone(&store));

        let eid = crate::entity::EntityId::new();
        let t0 = Utc::now();
        let old = belief_with(
            BeliefId::new(),
            eid,
            "p",
            Value::Int(1),
            0.2,
            t0,
        );
        store.insert(old).unwrap();

        let t1 = t0 + chrono::Duration::milliseconds(1);
        let new_id = BeliefId::new();
        let new_belief = belief_with(new_id, eid, "p", Value::Int(1), 0.9, t1);
        store.insert(new_belief).unwrap();

        let obs = AssertObservation {
            tx_time: t1,
            belief_id: new_id,
            entity_id: eid,
            predicate: "p".to_string(),
            value: Value::Int(1),
            confidence: 0.9,
            conflict_types: Vec::new(),
        };

        let out = matcher
            .evaluate(
                &Trigger::ConfidenceShift {
                    entity_id: Some(eid),
                    predicate: Some("p".to_string()),
                    threshold: 0.5,
                },
                &obs,
            )
            .unwrap();

        match out {
            MatchOutput::Match(EventPayload::ConfidenceShift { delta, .. }) => {
                assert!(delta > 0.5);
            }
            other => panic!("expected match, got {other:?}"),
        }
    }
}
