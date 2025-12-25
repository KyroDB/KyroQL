use std::sync::Arc;
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use uuid::Uuid;

use kyroql::engine::EngineResponse;
use kyroql::ir::{AssertPayload, ConsistencyMode, KyroIR, MonitorPayload, Operation};
use kyroql::monitor::{MonitorSystem, MonitorSystemConfig};
use kyroql::monitor::matcher::AssertObservation;
use kyroql::storage::InMemoryStores;
use kyroql::conflict::ConflictType;
use kyroql::{Confidence, Entity, EntityStore, EntityType, Pattern, PatternRule, PatternStore, Source, TimeRange, Value};

#[test]
fn monitor_confidence_shift_streams_event() {
    let stores = InMemoryStores::default();
    let entities = Arc::new(stores.entities);
    let beliefs = Arc::new(stores.beliefs);
    let patterns = Arc::new(stores.patterns);
    let conflicts = Arc::new(stores.conflicts);

    let entity = Entity::new("e", EntityType::Concept);
    entities.insert(entity.clone()).unwrap();

    let engine = kyroql::KyroEngine::new(entities, beliefs, patterns, conflicts);

    let t0 = Utc::now();
    let assert1 = KyroIR {
        version: KyroIR::CURRENT_VERSION.to_string(),
        request_id: Uuid::new_v4(),
        timestamp: t0,
        operation: Operation::Assert(AssertPayload {
            entity_id: entity.id,
            predicate: "p".to_string(),
            value: Value::Int(1),
            confidence: Confidence::from_agent(0.2, "a").unwrap(),
            source: Source::Unknown { description: None },
            valid_time: TimeRange::starting_at(t0),
            consistency_mode: ConsistencyMode::Force,
            embedding: None,
        }),
    };

    let EngineResponse::Assert { .. } = engine.execute(assert1).unwrap() else {
        panic!("expected assert response");
    };

    let monitor = KyroIR {
        version: KyroIR::CURRENT_VERSION.to_string(),
        request_id: Uuid::new_v4(),
        timestamp: t0,
        operation: Operation::Monitor(MonitorPayload {
            description: Some("confidence shift".to_string()),
            predicates: Some(vec!["p".to_string()]),
            entity_filter: Some(vec![entity.id]),
            pattern_filter: None,
            threshold: Some(Value::Float(0.5)),
            expires_at: Some(t0 + ChronoDuration::seconds(30)),
            callback: None,
        }),
    };

    let EngineResponse::Monitor { registration } = engine.execute(monitor).unwrap() else {
        panic!("expected monitor response");
    };

    let t1 = t0 + ChronoDuration::milliseconds(1);
    let assert2 = KyroIR {
        version: KyroIR::CURRENT_VERSION.to_string(),
        request_id: Uuid::new_v4(),
        timestamp: t1,
        operation: Operation::Assert(AssertPayload {
            entity_id: entity.id,
            predicate: "p".to_string(),
            value: Value::Int(1),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: Source::Unknown { description: None },
            valid_time: TimeRange::starting_at(t1),
            consistency_mode: ConsistencyMode::Force,
            embedding: None,
        }),
    };

    let EngineResponse::Assert { .. } = engine.execute(assert2).unwrap() else {
        panic!("expected assert response");
    };

    let ev = registration
        .stream
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    match ev.payload {
        kyroql::EventPayload::ConfidenceShift { delta, .. } => {
            assert!(delta > 0.5);
        }
        other => panic!("expected confidence shift event, got {other:?}"),
    }
}

#[test]
fn monitor_pattern_violation_streams_event() {
    let stores = InMemoryStores::default();
    let entities = Arc::new(stores.entities);
    let beliefs = Arc::new(stores.beliefs);
    let patterns = Arc::new(stores.patterns);
    let conflicts = Arc::new(stores.conflicts);

    let entity = Entity::new("e", EntityType::Concept);
    entities.insert(entity.clone()).unwrap();

    let pattern = Pattern::new(
        "temp-range",
        PatternRule::Range {
            predicate: "temp".to_string(),
            min: Some(0.0),
            max: Some(10.0),
        },
        Confidence::from_agent(0.99, "system").unwrap(),
    );
    let pattern_id = pattern.id;
    patterns.insert(pattern).unwrap();

    let engine = kyroql::KyroEngine::new(entities, beliefs, patterns, conflicts);

    let t0 = Utc::now();
    let monitor = KyroIR {
        version: KyroIR::CURRENT_VERSION.to_string(),
        request_id: Uuid::new_v4(),
        timestamp: t0,
        operation: Operation::Monitor(MonitorPayload {
            description: Some("pattern violation".to_string()),
            predicates: None,
            entity_filter: None,
            pattern_filter: Some(vec![pattern_id]),
            threshold: Some(Value::Null),
            expires_at: Some(t0 + ChronoDuration::seconds(30)),
            callback: None,
        }),
    };

    let EngineResponse::Monitor { registration } = engine.execute(monitor).unwrap() else {
        panic!("expected monitor response");
    };

    let t1 = t0 + ChronoDuration::milliseconds(1);
    let assert_bad = KyroIR {
        version: KyroIR::CURRENT_VERSION.to_string(),
        request_id: Uuid::new_v4(),
        timestamp: t1,
        operation: Operation::Assert(AssertPayload {
            entity_id: entity.id,
            predicate: "temp".to_string(),
            value: Value::Float(25.0),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: Source::Unknown { description: None },
            valid_time: TimeRange::starting_at(t1),
            consistency_mode: ConsistencyMode::Eventual,
            embedding: None,
        }),
    };

    let EngineResponse::Assert { conflict_ids, .. } = engine.execute(assert_bad).unwrap() else {
        panic!("expected assert response");
    };
    assert!(!conflict_ids.is_empty());

    let ev = registration
        .stream
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    match ev.payload {
        kyroql::EventPayload::PatternViolation { pattern_id: pid, .. } => {
            assert_eq!(pid, pattern_id);
        }
        other => panic!("expected pattern violation event, got {other:?}"),
    }
}

#[test]
fn monitor_expiry_disconnects_stream() {
    let stores = InMemoryStores::default();
    let entities = Arc::new(stores.entities);
    let beliefs = Arc::new(stores.beliefs);
    let patterns = Arc::new(stores.patterns);
    let conflicts = Arc::new(stores.conflicts);

    let entity = Entity::new("e", EntityType::Concept);
    entities.insert(entity.clone()).unwrap();

    let engine = kyroql::KyroEngine::new(entities, beliefs, patterns, conflicts);

    let t0 = Utc::now();
    let monitor = KyroIR {
        version: KyroIR::CURRENT_VERSION.to_string(),
        request_id: Uuid::new_v4(),
        timestamp: t0,
        operation: Operation::Monitor(MonitorPayload {
            description: Some("expiry".to_string()),
            predicates: Some(vec!["p".to_string()]),
            entity_filter: Some(vec![entity.id]),
            pattern_filter: None,
            threshold: Some(Value::Float(0.1)),
            expires_at: Some(t0 + ChronoDuration::milliseconds(500)),
            callback: None,
        }),
    };

    let EngineResponse::Monitor { registration } = engine.execute(monitor).unwrap() else {
        panic!("expected monitor response");
    };

    // Wait long enough for expiry + worker cleanup tick.
    std::thread::sleep(Duration::from_millis(700));

    // The stream should become disconnected once the subscription is removed.
    let err = registration.stream.recv_timeout(Duration::from_millis(50)).unwrap_err();
    let kyroql::KyroError::Execution(kyroql::error::ExecutionError::Disconnected { path }) = err else {
        panic!("expected disconnected, got {err:?}");
    };
    assert_eq!(path, "monitor_stream");
}

#[test]
fn monitor_drop_stream_unregisters_subscription() {
    let stores = InMemoryStores::default();
    let entities = Arc::new(stores.entities);
    let beliefs = Arc::new(stores.beliefs);
    let patterns = Arc::new(stores.patterns);
    let conflicts = Arc::new(stores.conflicts);

    let entity = Entity::new("e", EntityType::Concept);
    entities.insert(entity.clone()).unwrap();

    let engine = kyroql::KyroEngine::new(entities, beliefs, patterns, conflicts);

    let t0 = Utc::now();
    let monitor = KyroIR {
        version: KyroIR::CURRENT_VERSION.to_string(),
        request_id: Uuid::new_v4(),
        timestamp: t0,
        operation: Operation::Monitor(MonitorPayload {
            description: Some("drop stream".to_string()),
            predicates: Some(vec!["p".to_string()]),
            entity_filter: Some(vec![entity.id]),
            pattern_filter: None,
            threshold: Some(Value::Float(0.1)),
            expires_at: Some(t0 + ChronoDuration::seconds(30)),
            callback: None,
        }),
    };

    let EngineResponse::Monitor { registration } = engine.execute(monitor).unwrap() else {
        panic!("expected monitor response");
    };

    // Explicitly unsubscribe without cloning the stream.
    registration.stream.unsubscribe();

    // Give worker time to process the unregister.
    std::thread::sleep(Duration::from_millis(300));

    let err = registration
        .stream
        .recv_timeout(Duration::from_millis(50))
        .unwrap_err();
    let kyroql::KyroError::Execution(kyroql::error::ExecutionError::Disconnected { path }) = err else {
        panic!("expected disconnected, got {err:?}");
    };
    assert_eq!(path, "monitor_stream");
}

#[test]
fn monitor_backpressure_increments_dropped_events() {
    let stores = InMemoryStores::default();
    let beliefs: Arc<dyn kyroql::storage::BeliefStore> = Arc::new(stores.beliefs);

    let cfg = MonitorSystemConfig {
        observation_queue_capacity: 1024,
        control_queue_capacity: 64,
        stream_capacity: 1,
    };
    let monitor = MonitorSystem::new(cfg, Arc::clone(&beliefs));

    let triggers = vec![kyroql::Trigger::ConflictCreated {
        entity_id: None,
        conflict_types: Vec::new(),
    }];

    let reg = monitor
        .register(triggers, Some(Utc::now() + ChronoDuration::seconds(30)))
        .unwrap();

    // Intentionally do not read from the stream to force backpressure.
    // Flood with matching observations.
    for i in 0..2000u64 {
        monitor.observe_assert(AssertObservation {
            tx_time: Utc::now(),
            belief_id: kyroql::BeliefId::new(),
            entity_id: kyroql::EntityId::new(),
            predicate: "p".to_string(),
            value: Value::Int(i as i64),
            confidence: 0.5,
            conflict_types: vec![ConflictType::PatternViolation {
                pattern_id: "x".to_string(),
                pattern_name: "m".to_string(),
            }],
        });
    }

    // Allow dispatch to run.
    let mut dropped = 0u64;
    for _ in 0..50 {
        dropped = monitor.dropped_events();
        if dropped > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    // Keep registration alive so the stream sender exists; otherwise drops may turn into disconnect.
    let _keep = reg;

    assert!(dropped > 0, "expected dropped_events > 0 due to backpressure");
}
