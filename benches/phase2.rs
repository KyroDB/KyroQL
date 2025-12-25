use std::sync::Arc;
use std::time::Instant;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

use kyroql::{
    AssertBuilder, Belief, BeliefStore, Confidence, ConsistencyMode, DeriveBuilder,
    Entity, EntityStore, EntityType, KyroEngine, KyroRuntime, KyroRuntimeConfig, ResolveBuilder,
    ResolveMode, SimulateBuilder, Source, TimeRange, Value,
};

fn make_engine_with_data() -> (KyroEngine, kyroql::EntityId) {
    let stores = kyroql::InMemoryStores::default();

    let entity = kyroql::Entity::new("bench_entity", kyroql::EntityType::Artifact);
    let entity_id = entity.id;
    stores.entities.insert(entity).unwrap();

    // Seed beliefs so RESOLVE measures realistic work.
    // 256 beliefs with temperatures from 20.0 to 22.55 in steps of 0.01.
    for i in 0..256u32 {
        let belief = Belief::builder()
            .subject(entity_id)
            .predicate("temperature")
            .value(Value::Float(20.0 + f64::from(i) * 0.01))
            .confidence(Confidence::from_agent(0.8, "seeder").unwrap())
            .source(Source::Unknown { description: None })
            .valid_time(TimeRange::from_now())
            .build()
            .unwrap();
        stores.beliefs.insert(belief).unwrap();
    }

    let engine = KyroEngine::new(
        Arc::new(stores.entities),
        Arc::new(stores.beliefs),
        Arc::new(stores.patterns),
        Arc::new(stores.conflicts),
        Arc::new(stores.derivations),
    );

    (engine, entity_id)
}

fn bench_reflex_assert_force(c: &mut Criterion) {
    c.bench_function("phase2/reflex_assert_force", |b| {
        b.iter_custom(|iters| {
            // Fresh state per sample so accumulation does not leak between samples.
            let (engine, entity_id) = make_engine_with_data();
            let runtime = KyroRuntime::new(
                engine,
                KyroRuntimeConfig {
                    reflex_workers: 1,
                    reflection_workers: 1,
                    queue_capacity: 1024,
                },
            );

            let ir = AssertBuilder::new()
                .entity(entity_id)
                .predicate("temperature")
                .value(Value::Float(21.0))
                .confidence(Confidence::from_agent(0.9, "bench").unwrap())
                .source(Source::Unknown { description: None })
                .valid_time(TimeRange::from_now())
                .consistency_mode(ConsistencyMode::Force)
                .build()
                .unwrap();

            let start = Instant::now();
            for _ in 0..iters {
                let _ = runtime.execute(ir.clone()).unwrap();
            }
            start.elapsed()
        });
    });
}

fn bench_reflex_resolve_simple(c: &mut Criterion) {
    c.bench_function("phase2/reflex_resolve_simple", |b| {
        // Fresh runtime per sample, but exclude setup from timing.
        b.iter_custom(|iters| {
            let (engine, _entity_id) = make_engine_with_data();
            let runtime = KyroRuntime::new(
                engine,
                KyroRuntimeConfig {
                    reflex_workers: 1,
                    reflection_workers: 1,
                    queue_capacity: 1024,
                },
            );

            let ir = ResolveBuilder::new()
                .predicate("temperature")
                .mode(ResolveMode::Simple)
                .limit(10)
                .build()
                .unwrap();

            let start = Instant::now();
            for _ in 0..iters {
                let _ = runtime.execute(ir.clone()).unwrap();
            }
            start.elapsed()
        });
    });
}

fn bench_derive_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("derive_throughput");
    group.throughput(Throughput::Elements(1));

    group.bench_function("derive_10_premises", |b| {
        b.iter_custom(|iters| {
            let stores = kyroql::InMemoryStores::new();
            let e = Entity::new("BenchEntity", EntityType::Concept);
            let eid = e.id;
            stores.entities.insert(e).unwrap();

            let engine = KyroEngine::new(
                Arc::new(stores.entities),
                Arc::new(stores.beliefs),
                Arc::new(stores.patterns),
                Arc::new(stores.conflicts),
                Arc::new(stores.derivations),
            );

            let mut premises = Vec::new();
            for _ in 0..10 {
                let op = AssertBuilder::new()
                    .entity(eid)
                    .predicate("p")
                    .value(Value::Bool(true))
                    .confidence(Confidence::one())
                    .source(Source::unknown_with_description("setup"))
                    .valid_time(TimeRange::from_now())
                    .build()
                    .unwrap();
                if let kyroql::EngineResponse::Assert { belief_id, .. } = engine.execute(op).unwrap() {
                    premises.push(belief_id);
                }
            }

            let start = Instant::now();
            for _ in 0..iters {
                let op = DeriveBuilder::new()
                    .rule("bench_rule")
                    .sources(premises.clone())
                    .confidence(0.9)
                    .add_step("step1")
                    .add_step("step2")
                    .build()
                    .unwrap();
                engine.execute(op).unwrap();
            }
            start.elapsed()
        })
    });
    group.finish();
}

fn bench_simulate_spawn(c: &mut Criterion) {
    let mut group = c.benchmark_group("simulate_overhead");
    group.throughput(Throughput::Elements(1));

    group.bench_function("spawn_simulation", |b| {
        b.iter_custom(|iters| {
            let stores = kyroql::InMemoryStores::new();
            let e = Entity::new("SimEntity", EntityType::Concept);
            stores.entities.insert(e).unwrap();

            let engine = KyroEngine::new(
                Arc::new(stores.entities),
                Arc::new(stores.beliefs),
                Arc::new(stores.patterns),
                Arc::new(stores.conflicts),
                Arc::new(stores.derivations),
            );

            let start = Instant::now();
            for _ in 0..iters {
                let op = SimulateBuilder::new().build().unwrap();
                engine.execute(op).unwrap();
            }
            start.elapsed()
        })
    });

    group.finish();
}

criterion_group!(
    phase2,
    bench_reflex_assert_force,
    bench_reflex_resolve_simple,
    bench_derive_throughput,
    bench_simulate_spawn
);
criterion_main!(phase2);
