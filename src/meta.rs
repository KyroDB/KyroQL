//! Meta-knowledge utilities: coverage maps, gap analysis, and calibration summaries.

use std::collections::HashMap;
use std::sync::Arc;

use crate::entity::EntityId;
use crate::error::{ExecutionError, KyroError, KyroResult};
use crate::storage::{BeliefStore, EntityStore};

/// Coverage statistics for an entity.
#[derive(Debug, Clone)]
pub struct CoverageReport {
    pub entity_id: EntityId,
    pub total_beliefs: usize,
    pub predicates: HashMap<String, PredicateCoverage>,
}

/// Coverage and confidence summary for a predicate.
#[derive(Debug, Clone)]
pub struct PredicateCoverage {
    pub count: usize,
    pub avg_confidence: f32,
}

/// Gap analysis result for expected predicates.
#[derive(Debug, Clone)]
pub struct GapAnalysisResult {
    pub missing_predicates: Vec<String>,
    pub covered_predicates: Vec<String>,
}

/// Simple calibration summary (distribution of confidence values).
#[derive(Debug, Clone)]
pub struct CalibrationSummary {
    pub mean: f32,
    pub min: f32,
    pub max: f32,
    pub count: usize,
}

/// Meta-knowledge analyzer backed by Kyro stores.
#[derive(Clone)]
pub struct MetaAnalyzer {
    entities: Arc<dyn EntityStore>,
    beliefs: Arc<dyn BeliefStore>,
}

impl MetaAnalyzer {
    #[must_use]
    pub fn new(entities: Arc<dyn EntityStore>, beliefs: Arc<dyn BeliefStore>) -> Self {
        Self { entities, beliefs }
    }

    fn ensure_entity_exists(&self, entity_id: EntityId) -> KyroResult<()> {
        let entity_exists = self
            .entities
            .get(entity_id)
            .map_err(|e| KyroError::Execution(ExecutionError::Storage {
                message: e.to_string(),
            }))?;
        if entity_exists.is_some() {
            Ok(())
        } else {
            Err(KyroError::Execution(ExecutionError::EntityNotFound { id: entity_id }))
        }
    }

    /// Compute coverage for an entity across predicates.
    pub fn coverage(&self, entity_id: EntityId) -> KyroResult<CoverageReport> {
        self.ensure_entity_exists(entity_id)?;

        let beliefs = self
            .beliefs
            .find_by_entity(entity_id)
            .map_err(|e| KyroError::Execution(ExecutionError::Storage {
                message: e.to_string(),
            }))?;

        let mut predicates: HashMap<String, Vec<f32>> = HashMap::new();
        let total = beliefs.len();
        for b in beliefs.iter() {
            predicates
                .entry(b.predicate.clone())
                .or_default()
                .push(b.confidence.value());
        }

        let predicate_stats = predicates
            .into_iter()
            .map(|(pred, confs)| {
                let count = confs.len();
                let sum: f32 = confs.iter().copied().sum();
                let avg = if count == 0 { 0.0 } else { sum / count as f32 };
                (
                    pred,
                    PredicateCoverage {
                        count,
                        avg_confidence: avg.clamp(0.0, 1.0),
                    },
                )
            })
            .collect();

        Ok(CoverageReport {
            entity_id,
            total_beliefs: total,
            predicates: predicate_stats,
        })
    }

    /// Identify missing predicates relative to an expected list.
    pub fn gap_analysis(
        &self,
        entity_id: EntityId,
        expected_predicates: &[String],
    ) -> KyroResult<GapAnalysisResult> {
        let coverage = self.coverage(entity_id)?;
        let mut missing = Vec::new();
        let mut covered = Vec::new();
        for pred in expected_predicates {
            if coverage.predicates.contains_key(pred) {
                covered.push(pred.clone());
            } else {
                missing.push(pred.clone());
            }
        }
        Ok(GapAnalysisResult {
            missing_predicates: missing,
            covered_predicates: covered,
        })
    }

    /// Summarize confidence distribution for an entity.
    pub fn calibration_summary(&self, entity_id: EntityId) -> KyroResult<CalibrationSummary> {
        self.ensure_entity_exists(entity_id)?;

        let beliefs = self
            .beliefs
            .find_by_entity(entity_id)
            .map_err(|e| KyroError::Execution(ExecutionError::Storage {
                message: e.to_string(),
            }))?;

        let mut min = 1.0_f32;
        let mut max = 0.0_f32;
        let mut sum = 0.0_f32;
        let mut count = 0usize;

        for b in beliefs.iter() {
            let c = b.confidence.value().clamp(0.0, 1.0);
            min = min.min(c);
            max = max.max(c);
            sum += c;
            count += 1;
        }

        let mean = if count == 0 { 0.0 } else { sum / count as f32 };

        Ok(CalibrationSummary {
            mean,
            min: if count == 0 { 0.0 } else { min },
            max: if count == 0 { 0.0 } else { max },
            count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::belief::Belief;
    use crate::confidence::Confidence;
    use crate::entity::{Entity, EntityType};
    use crate::source::Source;
    use crate::storage::memory::InMemoryStores;
    use crate::time::TimeRange;
    use crate::value::Value;

    #[test]
    fn coverage_unknown_entity_is_entity_not_found() {
        let stores = InMemoryStores::new();
        let analyzer = MetaAnalyzer::new(Arc::new(stores.entities), Arc::new(stores.beliefs));
        let id = EntityId::new();

        let err = analyzer.coverage(id).unwrap_err();
        match err {
            KyroError::Execution(ExecutionError::EntityNotFound { id: got }) => {
                assert_eq!(got, id);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn empty_entity_has_zero_coverage_and_zero_calibration() {
        let stores = InMemoryStores::new();
        let entities: Arc<dyn EntityStore> = Arc::new(stores.entities);
        let beliefs: Arc<dyn BeliefStore> = Arc::new(stores.beliefs);
        let analyzer = MetaAnalyzer::new(Arc::clone(&entities), Arc::clone(&beliefs));

        let entity = Entity::new("E", EntityType::Concept);
        let id = entity.id;
        entities.insert(entity).unwrap();

        let coverage = analyzer.coverage(id).unwrap();
        assert_eq!(coverage.total_beliefs, 0);
        assert!(coverage.predicates.is_empty());

        let calib = analyzer.calibration_summary(id).unwrap();
        assert_eq!(calib.count, 0);
        assert_eq!(calib.mean, 0.0);
        assert_eq!(calib.min, 0.0);
        assert_eq!(calib.max, 0.0);
    }

    #[test]
    fn coverage_and_gap_analysis_compute_expected_stats() {
        let stores = InMemoryStores::new();
        let entities: Arc<dyn EntityStore> = Arc::new(stores.entities);
        let beliefs: Arc<dyn BeliefStore> = Arc::new(stores.beliefs);
        let analyzer = MetaAnalyzer::new(Arc::clone(&entities), Arc::clone(&beliefs));

        let entity = Entity::new("E", EntityType::Concept);
        let id = entity.id;
        entities.insert(entity).unwrap();

        let b1 = Belief::builder()
            .subject(id)
            .predicate("p1")
            .value(Value::String("v1".to_string()))
            .confidence(Confidence::from_agent(0.5, "a").unwrap())
            .source(Source::agent("a", Option::<String>::None))
            .valid_time(TimeRange::forever())
            .build()
            .unwrap();
        let b2 = Belief::builder()
            .subject(id)
            .predicate("p1")
            .value(Value::String("v2".to_string()))
            .confidence(Confidence::from_agent(1.0, "a").unwrap())
            .source(Source::agent("a", Option::<String>::None))
            .valid_time(TimeRange::forever())
            .build()
            .unwrap();
        let b3 = Belief::builder()
            .subject(id)
            .predicate("p2")
            .value(Value::Bool(true))
            .confidence(Confidence::from_agent(0.25, "b").unwrap())
            .source(Source::agent("b", Option::<String>::None))
            .valid_time(TimeRange::forever())
            .build()
            .unwrap();

        beliefs.insert(b1).unwrap();
        beliefs.insert(b2).unwrap();
        beliefs.insert(b3).unwrap();

        let coverage = analyzer.coverage(id).unwrap();
        assert_eq!(coverage.total_beliefs, 3);
        let p1 = coverage.predicates.get("p1").unwrap();
        assert_eq!(p1.count, 2);
        assert!((p1.avg_confidence - 0.75).abs() < 1e-6);
        let p2 = coverage.predicates.get("p2").unwrap();
        assert_eq!(p2.count, 1);
        assert!((p2.avg_confidence - 0.25).abs() < 1e-6);

        let gap = analyzer
            .gap_analysis(id, &vec!["p1".to_string(), "p3".to_string()])
            .unwrap();
        assert_eq!(gap.covered_predicates, vec!["p1".to_string()]);
        assert_eq!(gap.missing_predicates, vec!["p3".to_string()]);

        let calib = analyzer.calibration_summary(id).unwrap();
        assert_eq!(calib.count, 3);
        assert!((calib.mean - (0.5 + 1.0 + 0.25) / 3.0).abs() < 1e-6);
        assert!((calib.min - 0.25).abs() < 1e-6);
        assert!((calib.max - 1.0).abs() < 1e-6);
    }

    #[cfg(feature = "persistent")]
    #[test]
    fn persistent_meta_analyzer_smoke_test() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let stores = crate::storage::open_database(dir.path(), None).unwrap();

        let crate::storage::PersistentStores { entities, beliefs, .. } = stores;

        let entities: Arc<dyn EntityStore> = Arc::new(entities);
        let beliefs: Arc<dyn BeliefStore> = Arc::new(beliefs);
        let analyzer = MetaAnalyzer::new(Arc::clone(&entities), Arc::clone(&beliefs));

        let entity = Entity::new("E", EntityType::Concept);
        let id = entity.id;
        entities.insert(entity).unwrap();

        let belief = Belief::builder()
            .subject(id)
            .predicate("p1")
            .value(Value::String("v".to_string()))
            .confidence(Confidence::from_agent(0.7, "a").unwrap())
            .source(Source::agent("a", Option::<String>::None))
            .valid_time(TimeRange::forever())
            .build()
            .unwrap();
        beliefs.insert(belief).unwrap();

        let coverage = analyzer.coverage(id).unwrap();
        assert_eq!(coverage.total_beliefs, 1);
        assert!(coverage.predicates.contains_key("p1"));
    }
}
