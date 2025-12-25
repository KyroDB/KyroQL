use std::time::{Duration, Instant};
use std::{fs::OpenOptions, io::Write};
use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use serde_json::json;

use kyroql::{
    AssertBuilder, Confidence, ConsistencyMode, Entity, EntityType, EngineResponse, KyroEngine,
    KyroRuntime, KyroRuntimeConfig, ResolveBuilder, ResolveMode, Source, TimeRange, Value,
};
use kyroql::EntityStore;

fn p99(durations: &mut [Duration]) -> Duration {
    durations.sort_unstable();
    if durations.is_empty() {
        return Duration::from_nanos(0);
    }
    let idx = ((durations.len() as f64) * 0.99).ceil() as usize;
    let idx = idx.saturating_sub(1).min(durations.len() - 1);
    durations[idx]
}

fn ops_per_sec(ops: usize, elapsed: Duration) -> f64 {
    if elapsed.is_zero() {
        return f64::INFINITY;
    }
    (ops as f64) / elapsed.as_secs_f64()
}

fn resolve_workspace_target_dir() -> Option<PathBuf> {
    // Cargo may place `target/` at the workspace root (one or more parents above the crate).
    // Prefer an existing target dir, walking up a few levels.
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..6 {
        let candidate = dir.join("target");
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

fn resolve_perf_out_path(raw: &str) -> PathBuf {
    let p = Path::new(raw);
    if p.is_absolute() {
        return p.to_path_buf();
    }

    // Special-case `target/...` so it follows the actual cargo target directory.
    let mut components = p.components();
    if matches!(components.next(), Some(Component::Normal(c)) if c == "target") {
        let rest: PathBuf = components.collect();
        if let Some(target_dir) = resolve_workspace_target_dir() {
            return target_dir.join(rest);
        }
    }

    // Fallback: relative to crate dir.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(p)
}

/// Vision performance targets are only meaningful in release builds.
///
/// Run manually:
/// - `cargo test --release --test perf_targets -- --ignored --nocapture`
///
/// To enforce thresholds (may be machine-dependent):
/// - `KYRO_ENFORCE_VISION_PERF=1 cargo test --release --test perf_targets -- --ignored --nocapture`
#[test]
#[ignore]
fn vision_perf_targets_report() {
    assert!(
        !cfg!(debug_assertions),
        "perf targets must be measured in --release"
    );

    let stores = kyroql::InMemoryStores::default();

    let entity = Entity::new("perf_entity", EntityType::Artifact);
    let entity_id = entity.id;
    stores.entities.insert(entity).unwrap();

    let engine = KyroEngine::new(
        std::sync::Arc::new(stores.entities),
        std::sync::Arc::new(stores.beliefs),
        std::sync::Arc::new(stores.patterns),
        std::sync::Arc::new(stores.conflicts),
        std::sync::Arc::new(stores.derivations),
    );

    let runtime = KyroRuntime::new(
        engine,
        KyroRuntimeConfig {
            reflex_workers: 1,
            reflection_workers: 1,
            queue_capacity: 8192,
        },
    );

    // Seed beliefs so RESOLVE does non-trivial work.
    for i in 0..256u32 {
        let seed = AssertBuilder::new()
            .entity(entity_id)
            .predicate("temperature")
            .value(Value::Float(20.0 + f64::from(i) * 0.01))
            .confidence(Confidence::from_agent(0.95, "perf").unwrap())
            .source(Source::Unknown { description: None })
            .valid_time(TimeRange::from_now())
            .consistency_mode(ConsistencyMode::Force)
            .build()
            .unwrap();
        let _ = runtime.execute(seed).unwrap();
    }

    let out_path = std::env::var("KYRO_PERF_OUT").ok();
    let mut out_file = out_path.as_deref().map(|raw| {
        let path = resolve_perf_out_path(raw);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("failed to create perf output dir {}: {e}", parent.display()));
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap_or_else(|e| panic!("failed to open KYRO_PERF_OUT={raw} (resolved {}): {e}", path.display()))
    });

    let emit = |v: serde_json::Value, out_file: &mut Option<std::fs::File>| {
        let line = v.to_string();
        println!("{line}");
        if let Some(f) = out_file.as_mut() {
            writeln!(f, "{line}").expect("write perf evidence");
        }
    };

    // ---------------------
    // Reflex RESOLVE latency
    // ---------------------
    let resolve_ir = ResolveBuilder::new()
        .entity(entity_id)
        .predicate("temperature")
        .mode(ResolveMode::Simple)
        .limit(5)
        .build()
        .unwrap();

    // Warm-up to stabilize caches and JIT-less effects.
    for _ in 0..1_000 {
        let _ = runtime.execute(resolve_ir.clone()).unwrap();
    }

    let iterations = std::env::var("KYRO_PERF_RESOLVE_ITERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(20_000usize);
    let mut resolve_latencies = Vec::with_capacity(iterations);
    let start_all = Instant::now();
    for _ in 0..iterations {
        let t0 = Instant::now();
        let resp = runtime.execute(resolve_ir.clone()).unwrap();
        let EngineResponse::Resolve { .. } = resp else {
            panic!("expected resolve response");
        };
        resolve_latencies.push(t0.elapsed());
    }
    let total = start_all.elapsed();

    let mut lat_copy = resolve_latencies.clone();
    let p99_resolve = p99(&mut lat_copy);
    let resolve_rps = ops_per_sec(iterations, total);

    emit(
        json!({
            "metric": "resolve_simple",
            "p99_ns": p99_resolve.as_nanos(),
            "rps": resolve_rps,
            "iters": iterations,
        }),
        &mut out_file,
    );

    // -----------------
    // ASSERT throughput
    // -----------------
    // NOTE: this grows the in-memory store; keep iteration count bounded.
    let assert_ir = AssertBuilder::new()
        .entity(entity_id)
        .predicate("temperature")
        .value(Value::Float(22.0))
        .confidence(Confidence::from_agent(0.9, "perf").unwrap())
        .source(Source::Unknown { description: None })
        .valid_time(TimeRange::starting_at(Utc::now()))
        .consistency_mode(ConsistencyMode::Force)
        .build()
        .unwrap();

    // Warm-up.
    for _ in 0..1_000 {
        let _ = runtime.execute(assert_ir.clone()).unwrap();
    }

    let assert_iters = std::env::var("KYRO_PERF_ASSERT_ITERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(10_000usize);
    let start_assert = Instant::now();
    for _ in 0..assert_iters {
        let resp = runtime.execute(assert_ir.clone()).unwrap();
        let EngineResponse::Assert { .. } = resp else {
            panic!("expected assert response");
        };
    }
    let assert_total = start_assert.elapsed();
    let assert_rps = ops_per_sec(assert_iters, assert_total);

    emit(
        json!({
            "metric": "assert_force",
            "rps": assert_rps,
            "iters": assert_iters,
        }),
        &mut out_file,
    );

    // Optional enforcement (machine-dependent).
    if std::env::var("KYRO_ENFORCE_VISION_PERF").ok().as_deref() == Some("1") {
        let max_p99 = Duration::from_millis(5);
        let min_assert_rps = 10_000.0;
        let min_resolve_rps = 50_000.0;

        assert!(
            p99_resolve <= max_p99,
            "vision perf fail: resolve_simple p99={:?} > {:?}",
            p99_resolve,
            max_p99
        );
        assert!(
            assert_rps >= min_assert_rps,
            "vision perf fail: assert rps={:.0} < {:.0}",
            assert_rps,
            min_assert_rps
        );
        assert!(
            resolve_rps >= min_resolve_rps,
            "vision perf fail: resolve_simple rps={:.0} < {:.0}",
            resolve_rps,
            min_resolve_rps
        );
    }
}
