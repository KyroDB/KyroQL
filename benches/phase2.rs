use std::sync::Arc;
use std::time::Instant;

use criterion::{criterion_group, criterion_main, Criterion};

use kyroql::{
    AssertBuilder, Belief, BeliefStore, Confidence, ConsistencyMode, EntityStore, KyroEngine,
    KyroRuntime, KyroRuntimeConfig, ResolveBuilder, ResolveMode, Source, TimeRange, Value,
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

criterion_group!(phase2, bench_reflex_assert_force, bench_reflex_resolve_simple);
criterion_main!(phase2);
