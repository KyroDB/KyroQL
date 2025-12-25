//! Routed execution runtime for KyroQL.
//!
//! The `KyroEngine` is a synchronous executor for `KyroIR`. The Vision requires that
//! Reflection work must not block Reflex. This module provides a small, bounded,
//! thread-based runtime that routes requests into separate worker pools.

use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};

use crate::error::{ExecutionError, KyroError, KyroResult};
use crate::ir::{ConsistencyMode, KyroIR, Operation, ResolveMode};
use crate::engine::{EngineResponse, KyroEngine};

/// Execution path selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionPath {
    /// Fast, bounded operations.
    Reflex,
    /// Slow, deliberative operations.
    Reflection,
}

/// Routes operations to an execution path.
pub trait OperationRouter: Send + Sync {
    /// Selects the execution path for the given operation.
    fn route(&self, op: &Operation) -> ExecutionPath;
}

/// Default Vision-aligned router.
///
/// Policy:
/// - `Resolve(Simple)` is Reflex.
/// - `Resolve(Aggregate|Temporal)` is Reflection.
/// - `Assert(Force)` is Reflex; all other consistency modes are Reflection.
/// - `Retract` is Reflex.
/// - `DefinePattern`, `Simulate`, `Monitor`, `Derive` are Reflection.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultRouter;

impl OperationRouter for DefaultRouter {
    fn route(&self, op: &Operation) -> ExecutionPath {
        match op {
            Operation::Resolve(payload) => match payload.mode {
                ResolveMode::Simple => ExecutionPath::Reflex,
                ResolveMode::Aggregate | ResolveMode::Temporal => ExecutionPath::Reflection,
            },
            Operation::Assert(payload) => match payload.consistency_mode {
                ConsistencyMode::Force => ExecutionPath::Reflex,
                ConsistencyMode::Strict | ConsistencyMode::Eventual => ExecutionPath::Reflection,
            },
            Operation::Retract(_) => ExecutionPath::Reflex,
            Operation::DefinePattern(_) => ExecutionPath::Reflection,
            Operation::Simulate(_) | Operation::Monitor(_) | Operation::Derive(_) => {
                ExecutionPath::Reflection
            }
        }
    }
}

/// Runtime configuration.
#[derive(Debug, Clone)]
pub struct KyroRuntimeConfig {
    /// Number of Reflex workers.
    pub reflex_workers: usize,
    /// Number of Reflection workers.
    pub reflection_workers: usize,
    /// Maximum queued jobs per pool.
    pub queue_capacity: usize,
}

impl Default for KyroRuntimeConfig {
    fn default() -> Self {
        Self {
            reflex_workers: 2,
            reflection_workers: 2,
            queue_capacity: 1024,
        }
    }
}

enum Job {
    Execute {
        ir: KyroIR,
        reply: Sender<KyroResult<EngineResponse>>,
    },

    #[cfg(test)]
    Sleep {
        duration: Duration,
        reply: Sender<()>,
    },
}

struct WorkerPool {
    tx: Sender<Job>,
    workers: Vec<JoinHandle<()>>,
    queue_capacity: usize,
}

impl WorkerPool {
    fn start(name: &'static str, workers: usize, queue_capacity: usize, engine: Arc<KyroEngine>) -> Self {
        let workers = workers.max(1);
        let queue_capacity = queue_capacity.max(1);
        let (tx, rx) = bounded::<Job>(queue_capacity);

        let mut handles = Vec::with_capacity(workers);
        for idx in 0..workers {
            let rx: Receiver<Job> = rx.clone();
            let engine = Arc::clone(&engine);
            let thread_name = format!("kyroql-{name}-{idx}");
            let handle = thread::Builder::new()
                .name(thread_name)
                .spawn(move || loop {
                    match rx.recv() {
                        Ok(Job::Execute { ir, reply }) => {
                            let result = engine.execute(ir);
                            let _ = reply.send(result);
                        }
                        Err(_) => break,

                        #[cfg(test)]
                        Ok(Job::Sleep { duration, reply }) => {
                            thread::sleep(duration);
                            let _ = reply.send(());
                        }
                    }
                })
                .expect("failed to spawn kyroql worker");
            handles.push(handle);
        }

        Self {
            tx,
            workers: handles,
            queue_capacity,
        }
    }

    fn try_submit(&self, job: Job, path: ExecutionPath) -> Result<(), KyroError> {
        let path_s = match path {
            ExecutionPath::Reflex => "reflex".to_string(),
            ExecutionPath::Reflection => "reflection".to_string(),
        };
        match self.tx.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(KyroError::Execution(ExecutionError::QueueFull {
                path: path_s,
                capacity: self.queue_capacity,
            })),
            Err(TrySendError::Disconnected(_)) => Err(KyroError::Execution(ExecutionError::Disconnected {
                path: path_s,
            })),
        }
    }

    fn shutdown(self) {
        // Close the channel: workers will drain queued jobs then exit.
        drop(self.tx);
        for handle in self.workers {
            let _ = handle.join();
        }
    }
}

/// Handle returned by `execute_async`.
pub struct ExecutionHandle {
    path: ExecutionPath,
    rx: Receiver<KyroResult<EngineResponse>>,
}

impl ExecutionHandle {
    /// Returns the path selected by the router.
    #[must_use]
    pub const fn path(&self) -> ExecutionPath {
        self.path
    }

    /// Waits for the execution to complete.
    pub fn join(self) -> KyroResult<EngineResponse> {
        let path_s = match self.path {
            ExecutionPath::Reflex => "reflex".to_string(),
            ExecutionPath::Reflection => "reflection".to_string(),
        };
        self.rx
            .recv()
            .map_err(|_| KyroError::Execution(ExecutionError::Disconnected { path: path_s }))?
    }

    /// Waits for the execution to complete with a timeout.
    pub fn join_timeout(self, timeout: Duration) -> KyroResult<EngineResponse> {
        let path_s = match self.path {
            ExecutionPath::Reflex => "reflex".to_string(),
            ExecutionPath::Reflection => "reflection".to_string(),
        };
        self.rx
            .recv_timeout(timeout)
            .map_err(|err| match err {
                crossbeam_channel::RecvTimeoutError::Timeout => {
                    KyroError::Execution(ExecutionError::Timeout {
                        duration_ms: timeout.as_millis().min(u128::from(u64::MAX)) as u64,
                    })
                }
                crossbeam_channel::RecvTimeoutError::Disconnected => {
                    KyroError::Execution(ExecutionError::Disconnected { path: path_s })
                }
            })?
    }
}

/// A routed runtime that enforces Reflex/Reflection isolation.
pub struct KyroRuntime<R: OperationRouter = DefaultRouter> {
    router: R,
    engine: Arc<KyroEngine>,
    reflex: WorkerPool,
    reflection: WorkerPool,
}

impl KyroRuntime<DefaultRouter> {
    /// Create a runtime with the default router.
    pub fn new(engine: KyroEngine, config: KyroRuntimeConfig) -> Self {
        Self::with_router(engine, DefaultRouter, config)
    }
}

impl<R: OperationRouter> KyroRuntime<R> {
    /// Create a runtime with a custom router.
    pub fn with_router(engine: KyroEngine, router: R, config: KyroRuntimeConfig) -> Self {
        let engine = Arc::new(engine);
        let reflex = WorkerPool::start("reflex", config.reflex_workers, config.queue_capacity, Arc::clone(&engine));
        let reflection =
            WorkerPool::start("reflection", config.reflection_workers, config.queue_capacity, Arc::clone(&engine));
        Self {
            router,
            engine,
            reflex,
            reflection,
        }
    }

    /// Execute an IR request asynchronously on the routed path.
    pub fn execute_async(&self, ir: KyroIR) -> Result<ExecutionHandle, KyroError> {
        let path = self.router.route(&ir.operation);
        let (tx, rx) = bounded::<KyroResult<EngineResponse>>(1);
        let job = Job::Execute { ir, reply: tx };
        match path {
            ExecutionPath::Reflex => self.reflex.try_submit(job, path)?,
            ExecutionPath::Reflection => self.reflection.try_submit(job, path)?,
        }
        Ok(ExecutionHandle { path, rx })
    }

    /// Execute an IR request synchronously on the routed path.
    pub fn execute(&self, ir: KyroIR) -> KyroResult<EngineResponse> {
        self.execute_async(ir)?.join()
    }

    /// Returns a shared reference to the underlying engine.
    #[must_use]
    pub fn engine(&self) -> &KyroEngine {
        &self.engine
    }

    #[cfg(test)]
    fn submit_sleep(&self, path: ExecutionPath, duration: Duration) -> Result<Receiver<()>, KyroError> {
        let (tx, rx) = bounded::<()>(1);
        let job = Job::Sleep { duration, reply: tx };
        match path {
            ExecutionPath::Reflex => self.reflex.try_submit(job, path)?,
            ExecutionPath::Reflection => self.reflection.try_submit(job, path)?,
        }
        Ok(rx)
    }
}

impl<R: OperationRouter> Drop for KyroRuntime<R> {
    fn drop(&mut self) {
        // Deterministic shutdown: stop workers and join threads.
        // This should be fast because worker loops are blocking on `recv()`.
        let reflex = std::mem::replace(
            &mut self.reflex,
            WorkerPool {
                tx: bounded::<Job>(1).0,
                workers: Vec::new(),
                queue_capacity: 1,
            },
        );
        let reflection = std::mem::replace(
            &mut self.reflection,
            WorkerPool {
                tx: bounded::<Job>(1).0,
                workers: Vec::new(),
                queue_capacity: 1,
            },
        );

        reflex.shutdown();
        reflection.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;

    use crate::belief::Belief;
    use crate::confidence::Confidence;
    use crate::operations::{AssertBuilder, ResolveBuilder};
    use crate::source::Source;
    use crate::storage::InMemoryStores;
    use crate::EntityStore;
    use crate::time::TimeRange;
    use crate::value::Value;

    fn engine_with_data() -> KyroEngine {
        let stores = InMemoryStores::default();
        let entity = crate::entity::Entity::new("sensor", crate::entity::EntityType::Artifact);
        let entity_id = entity.id;
        stores.entities.insert(entity).unwrap();

        let engine = KyroEngine::new(
            Arc::new(stores.entities),
            Arc::new(stores.beliefs),
            Arc::new(stores.patterns),
            Arc::new(stores.conflicts),
            Arc::new(stores.derivations),
        );

        let belief = Belief::builder()
            .subject(entity_id)
            .predicate("temperature")
            .value(Value::Float(20.0))
            .confidence(Confidence::from_agent(0.9, "test").unwrap())
            .source(Source::Unknown { description: None })
            .valid_time(TimeRange::from_now())
            .build()
            .unwrap();

        engine
            .execute(crate::ir::KyroIR::new(crate::ir::Operation::Assert(
                crate::ir::AssertPayload {
                    entity_id,
                    predicate: belief.predicate,
                    value: belief.value,
                    confidence: belief.confidence,
                    source: belief.source,
                    valid_time: belief.valid_time,
                    consistency_mode: ConsistencyMode::Force,
                    embedding: belief.embedding,
                },
            )))
            .unwrap();

        engine
    }

    #[test]
    fn router_routes_as_expected() {
        let router = DefaultRouter;

        let resolve_simple = ResolveBuilder::new()
            .predicate("temperature")
            .mode(ResolveMode::Simple)
            .build()
            .unwrap();
        assert_eq!(router.route(&resolve_simple.operation), ExecutionPath::Reflex);

        let resolve_temporal = ResolveBuilder::new()
            .predicate("temperature")
            .as_of(Utc::now())
            .mode(ResolveMode::Temporal)
            .build()
            .unwrap();
        assert_eq!(router.route(&resolve_temporal.operation), ExecutionPath::Reflection);

        let assert_force = AssertBuilder::new()
            .entity(crate::entity::EntityId::new())
            .predicate("p")
            .value(Value::Bool(true))
            .confidence(Confidence::from_agent(0.9, "test").unwrap())
            .source(Source::Unknown { description: None })
            .valid_time(TimeRange::from_now())
            .consistency_mode(ConsistencyMode::Force)
            .build()
            .unwrap();
        assert_eq!(router.route(&assert_force.operation), ExecutionPath::Reflex);

        let simulate = crate::operations::SimulateBuilder::new().build().unwrap();
        assert_eq!(router.route(&simulate.operation), ExecutionPath::Reflection);
    }

    #[test]
    fn reflection_work_does_not_starve_reflex() {
        let engine = engine_with_data();
        let runtime = KyroRuntime::new(
            engine,
            KyroRuntimeConfig {
                reflex_workers: 1,
                reflection_workers: 1,
                queue_capacity: 16,
            },
        );

        // Occupy the reflection worker.
        let sleep = runtime
            .submit_sleep(ExecutionPath::Reflection, Duration::from_millis(200))
            .unwrap();

        // Submit a reflex RESOLVE and ensure it completes quickly.
        let ir = ResolveBuilder::new()
            .predicate("temperature")
            .mode(ResolveMode::Simple)
            .build()
            .unwrap();

        let started = std::time::Instant::now();
        let handle = runtime.execute_async(ir).unwrap();
        assert_eq!(handle.path(), ExecutionPath::Reflex);
        let _ = handle.join_timeout(Duration::from_millis(50)).unwrap();
        assert!(started.elapsed() < Duration::from_millis(100));

        // Ensure the reflection sleep job completes too.
        sleep.recv_timeout(Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn join_reports_disconnected_when_reply_sender_dropped() {
        let (_tx, rx) = bounded::<KyroResult<EngineResponse>>(1);
        // Drop sender without sending, so recv() must see Disconnected.
        drop(_tx);

        let handle = ExecutionHandle {
            path: ExecutionPath::Reflex,
            rx,
        };

        let err = handle.join().unwrap_err();
        let KyroError::Execution(ExecutionError::Disconnected { path }) = err else {
            panic!("expected Disconnected, got {err:?}");
        };
        assert_eq!(path, "reflex");
    }

    #[test]
    fn join_timeout_reports_disconnected_not_timeout_when_reply_sender_dropped() {
        let (_tx, rx) = bounded::<KyroResult<EngineResponse>>(1);
        drop(_tx);

        let handle = ExecutionHandle {
            path: ExecutionPath::Reflection,
            rx,
        };

        let err = handle.join_timeout(Duration::from_millis(10)).unwrap_err();
        let KyroError::Execution(ExecutionError::Disconnected { path }) = err else {
            panic!("expected Disconnected, got {err:?}");
        };
        assert_eq!(path, "reflection");
    }
}
