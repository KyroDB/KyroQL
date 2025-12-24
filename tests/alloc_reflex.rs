use std::alloc::System;
use std::sync::Arc;

use stats_alloc::{Region, StatsAlloc, INSTRUMENTED_SYSTEM};

use kyroql::EntityStore;

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn make_runtime_with_entity() -> (kyroql::KyroRuntime, kyroql::EntityId) {
    let stores = kyroql::InMemoryStores::default();

    let entity = kyroql::Entity::new("alloc_test_entity", kyroql::EntityType::Artifact);
    let entity_id = entity.id;
    stores.entities.insert(entity).unwrap();

    let engine = kyroql::KyroEngine::new(
        Arc::new(stores.entities),
        Arc::new(stores.beliefs),
        Arc::new(stores.patterns),
        Arc::new(stores.conflicts),
    );

    let runtime = kyroql::KyroRuntime::new(
        engine,
        kyroql::KyroRuntimeConfig {
            reflex_workers: 1,
            reflection_workers: 1,
            queue_capacity: 128,
        },
    );

    (runtime, entity_id)
}

#[test]
fn reflex_resolve_simple_allocation_budget() {
    let (runtime, _entity_id) = make_runtime_with_entity();

    // Warm up: create threads/channels before measuring.
    let warm = kyroql::ResolveBuilder::new()
        .predicate("temperature")
        .mode(kyroql::ResolveMode::Simple)
        .limit(10)
        .build()
        .unwrap();
    runtime.execute(warm).expect("warm-up resolve must succeed");

    let ir = kyroql::ResolveBuilder::new()
        .predicate("temperature")
        .mode(kyroql::ResolveMode::Simple)
        .limit(10)
        .build()
        .unwrap();

    let region = Region::new(GLOBAL);
    runtime.execute(ir).expect("measured resolve must succeed");
    let stats = region.change();

    // Budgets are intentionally conservative to avoid CI flakiness.
    // The goal is to catch pathological regressions (e.g., per-call unbounded allocations).
    assert!(
        stats.allocations <= 5_000,
        "reflex resolve allocated too much: {stats:?}"
    );
    assert!(
        stats.bytes_allocated <= 2_000_000,
        "reflex resolve allocated too many bytes: {stats:?}"
    );
}

#[test]
fn reflex_assert_force_allocation_budget() {
    let (runtime, entity_id) = make_runtime_with_entity();

    // Warm up.
    let warm = kyroql::AssertBuilder::new()
        .entity(entity_id)
        .predicate("temperature")
        .value(kyroql::Value::Float(21.0))
        .confidence(kyroql::Confidence::from_agent(0.9, "alloc").unwrap())
        .source(kyroql::Source::Unknown { description: None })
        .valid_time(kyroql::TimeRange::from_now())
        .consistency_mode(kyroql::ConsistencyMode::Force)
        .build()
        .unwrap();
    runtime.execute(warm).expect("warm-up assert must succeed");

    let ir = kyroql::AssertBuilder::new()
        .entity(entity_id)
        .predicate("temperature")
        .value(kyroql::Value::Float(22.0))
        .confidence(kyroql::Confidence::from_agent(0.9, "alloc").unwrap())
        .source(kyroql::Source::Unknown { description: None })
        .valid_time(kyroql::TimeRange::from_now())
        .consistency_mode(kyroql::ConsistencyMode::Force)
        .build()
        .unwrap();

    let region = Region::new(GLOBAL);
    runtime.execute(ir).expect("measured assert must succeed");
    let stats = region.change();

    assert!(
        stats.allocations <= 10_000,
        "reflex assert(force) allocated too much: {stats:?}"
    );
    assert!(
        stats.bytes_allocated <= 5_000_000,
        "reflex assert(force) allocated too many bytes: {stats:?}"
    );
}
