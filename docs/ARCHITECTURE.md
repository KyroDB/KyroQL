# KyroQL Architecture

**Version**: 1.0  

---

## Overview

This document describes the architectural layout of KyroQL, including the system components, execution paths, and integration with the existing KyroDB storage engine.

---

## 1. Deployment Modes

KyroQL supports two deployment modes to balance **adoption friction** with **scale requirements**.

### 1.1 Embedded Mode (Zero Friction)

**The "SQLite/DuckDB" approach** - no server setup required.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           YOUR APPLICATION PROCESS                           │
│                                                                              │
│  ┌─────────────────┐     ┌─────────────────────────────────────────────┐    │
│  │   Your Agent    │────▶│              KyroQL Embedded                 │    │
│  │   (Python/Rust) │     │                                              │    │
│  └─────────────────┘     │  ┌─────────────┐  ┌─────────────────────┐   │    │
│                          │  │  Execution  │  │   Inference Layer   │   │    │
│                          │  │   Engine    │  │   (in-process)      │   │    │
│                          │  └──────┬──────┘  └──────────┬──────────┘   │    │
│                          │         │                    │              │    │
│                          │  ┌──────▼────────────────────▼──────────┐   │    │
│                          │  │         Embedded Storage              │   │    │
│                          │  │  ┌─────────┐  ┌─────────┐  ┌───────┐ │   │    │
│                          │  │  │ SQLite  │  │  Small  │  │ JSON  │ │   │    │
│                          │  │  │  (data) │  │  HNSW   │  │(config)│ │   │    │
│                          │  │  └─────────┘  └─────────┘  └───────┘ │   │    │
│                          │  └──────────────────────────────────────┘   │    │
│                          └─────────────────────────────────────────────┘    │
│                                         │                                    │
│                                         ▼                                    │
│                                  📁 brain.kyro                               │
│                              (single file on disk)                           │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Installation:**

```bash
pip install kyroql
```

**Usage:**

```python
import kyroql

# Opens or creates a local KyroQL database - NO SERVER NEEDED
db = kyroql.open("brain.kyro")

# All KyroQL operations work exactly the same
belief_id = await db.assert_(
    entity="user_123",
    predicate="preference",
    value="dark_mode",
    confidence=0.9,
)

frame = await db.resolve("What are the user preferences?")
```

**Characteristics:**

| Aspect           | Embedded Mode                                 |
| ---------------- | --------------------------------------------- |
| **Setup**        | Zero - just `pip install`                     |
| **Storage**      | Single file (`*.kyro`)                        |
| **Vector Index** | Small in-memory HNSW (up to ~100K vectors)    |
| **Concurrency**  | Single-process (file lock)                    |
| **SIMULATE**     | Fully supported (in-memory overlays)          |
| **MONITOR**      | In-process callbacks only                     |
| **Max Beliefs**  | ~1M (practical limit for SQLite)              |
| **Use Case**     | Prototyping, single-agent apps, local testing |

**Limitations:**

- Single process only (no multi-agent sharing)
- No gRPC streaming (MONITOR uses callbacks)
- Vector search slower for large datasets
- No horizontal scaling

---

### 1.2 Server Mode (Production Scale)

**The "PostgreSQL" approach** - full distributed deployment.

```
┌──────────────────────────────────────────────────────────────────┐
│                     MULTI-AGENT DEPLOYMENT                        │
│                                                                   │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐               │
│  │   Agent A   │  │   Agent B   │  │   Agent C   │               │
│  │  (Python)   │  │   (Rust)    │  │    (JS)     │               │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘               │
│         │                │                │                       │
│         └────────────────┼────────────────┘                       │
│                          │                                        │
│                     gRPC / HTTP                                   │
│                          │                                        │
└──────────────────────────┼────────────────────────────────────────┘
                           ▼
┌──────────────────────────────────────────────────────────────────┐
│                      KYROQL SERVER CLUSTER                        │
│   (Load Balanced, Horizontally Scalable)                          │
│                                                                   │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │                    Full KyroQL Engine                        │ │
│  │  • Multi-threaded execution pools                            │ │
│  │  • gRPC streaming for MONITOR                                │ │
│  │  • Distributed simulations                                   │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                          │                                        │
│                          ▼                                        │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │                    KyroDB Storage Engine                     │ │
│  │  • Full HNSW (billions of vectors)                          │ │
│  │  • Hybrid Semantic Cache                                     │ │
│  │  • WAL + Persistence                                         │ │
│  │  • Tiered Storage (hot/cold)                                 │ │
│  └─────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────┘
```

**Usage:**

```python
import kyroql

# Connect to KyroQL server - exact same API as embedded!
db = kyroql.connect("kyroql://localhost:50051")

# Identical operations
belief_id = await db.assert_(...)
frame = await db.resolve(...)
```

**Characteristics:**

| Aspect           | Server Mode                                 |
| ---------------- | ------------------------------------------- |
| **Setup**        | Requires KyroDB + KyroQL server             |
| **Storage**      | Distributed, persistent                     |
| **Vector Index** | Full HNSW (billions of vectors)             |
| **Concurrency**  | Multi-agent, multi-process                  |
| **SIMULATE**     | Fully supported with resource limits        |
| **MONITOR**      | gRPC streaming to any subscriber            |
| **Max Beliefs**  | Unlimited (horizontal scaling)              |
| **Use Case**     | Production, multi-agent systems, enterprise |

---

### 1.3 Migration Path: Embedded → Server

The API is **identical** between modes. Migration requires only changing the connection string:

```python
# Development (Embedded)
db = kyroql.open("brain.kyro")

# Production (Server)
db = kyroql.connect("kyroql://prod-cluster.example.com:50051")

# ALL OTHER CODE REMAINS UNCHANGED
```

**Data Migration:**

```bash
# Export from embedded
kyroql export brain.kyro --format=kyroql-dump

# Import to server
kyroql import --server=localhost:50051 < brain.kyro.dump
```

This allows developers to:

1. **Start instantly** with embedded mode
2. **Prototype** without infrastructure overhead
3. **Scale** to server mode when needed
4. **Never rewrite** their agent code

---

### 1.4 Deployment Decision Matrix

| Need                        | Recommended Mode |
| --------------------------- | ---------------- |
| Just trying KyroQL          | **Embedded**     |
| Single agent prototype      | **Embedded**     |
| Local CI/CD testing         | **Embedded**     |
| Multi-agent system          | **Server**       |
| > 100K beliefs              | **Server**       |
| Production workload         | **Server**       |
| Team/shared memory          | **Server**       |
| Real-time MONITOR streaming | **Server**       |

---

## 2. System Architecture (Server Mode)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              AGENT LAYER                                     │
│                     (LLMs, Autonomous Agents, Applications)                  │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    │ KyroQL Operations
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           CLIENT SDK LAYER                                   │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────────────────┐  │
│  │   Rust Client   │  │  Python Client  │  │      JavaScript Client      │  │
│  │                 │  │                 │  │                             │  │
│  │  Fluent Builder │  │  Fluent Builder │  │       Fluent Builder        │  │
│  │       API       │  │       API       │  │           API               │  │
│  └────────┬────────┘  └────────┬────────┘  └─────────────┬───────────────┘  │
│           │                    │                         │                  │
│           └────────────────────┼─────────────────────────┘                  │
│                                │                                            │
│                    ┌───────────▼───────────┐                                │
│                    │      IR Generator     │                                │
│                    │  (KyroIR Serializer)  │                                │
│                    └───────────┬───────────┘                                │
└────────────────────────────────┼────────────────────────────────────────────┘
                                 │
                                 │ gRPC / HTTP
                                 ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           KYROQL SERVER                                      │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                         Query Router                                  │   │
│  │     ┌─────────────────────────┬─────────────────────────┐            │   │
│  │     │                         │                         │            │   │
│  │     ▼                         ▼                         ▼            │   │
│  │ ┌────────────┐          ┌────────────┐          ┌────────────┐       │   │
│  │ │  REFLEX    │          │ REFLECTION │          │  MONITOR   │       │   │
│  │ │   PATH     │          │    PATH    │          │   PATH     │       │   │
│  │ │            │          │            │          │            │       │   │
│  │ │ • Simple   │          │ • SIMULATE │          │ • Triggers │       │   │
│  │ │   RESOLVE  │          │ • Temporal │          │ • Streaming│       │   │
│  │ │ • Fast     │          │   RESOLVE  │          │ • Events   │       │   │
│  │ │   ASSERT   │          │ • Conflict │          │            │       │   │
│  │ │            │          │   Analysis │          │            │       │   │
│  │ └─────┬──────┘          └─────┬──────┘          └─────┬──────┘       │   │
│  └───────┼───────────────────────┼───────────────────────┼──────────────┘   │
│          │                       │                       │                  │
│  ┌───────▼───────────────────────▼───────────────────────▼──────────────┐   │
│  │                        EXECUTION ENGINE                               │   │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  │   │
│  │  │   Assert    │  │   Resolve   │  │  Simulate   │  │   Monitor   │  │   │
│  │  │  Executor   │  │  Executor   │  │  Executor   │  │  Executor   │  │   │
│  │  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  │   │
│  └─────────┼────────────────┼────────────────┼────────────────┼─────────┘   │
│            │                │                │                │             │
│  ┌─────────▼────────────────▼────────────────▼────────────────▼─────────┐   │
│  │                      INFERENCE LAYER                                  │   │
│  │  ┌─────────────────────────────────────────────────────────────────┐ │   │
│  │  │                   Conflict Resolution                            │ │   │
│  │  │  • LatestWins  • HighestConfidence  • SourcePriority  • Bayesian │ │   │
│  │  └─────────────────────────────────────────────────────────────────┘ │   │
│  │  ┌─────────────────────────────────────────────────────────────────┐ │   │
│  │  │                       Trust Weighting                            │ │   │
│  │  │  • Source trust weights (global + domain overrides)              │ │   │
│  │  │  • Applied at RESOLVE time (does not mutate stored confidence)   │ │   │
│  │  └─────────────────────────────────────────────────────────────────┘ │   │
│  │  ┌─────────────────────────────────────────────────────────────────┐ │   │
│  │  │                     Pattern Checker                              │ │   │
│  │  │     • Constraint Validation  • Invariant Enforcement             │ │   │
│  │  └─────────────────────────────────────────────────────────────────┘ │   │
│  │  ┌─────────────────────────────────────────────────────────────────┐ │   │
│  │  │                    Conflict Detector                             │ │   │
│  │  │   • Value  • Temporal  • Source  • Pattern Violations            │ │   │
│  │  └─────────────────────────────────────────────────────────────────┘ │   │
│  │  ┌─────────────────────────────────────────────────────────────────┐ │   │
│  │  │                      Meta-Knowledge                              │ │   │
│  │  │  • Coverage / gap analysis                                       │ │   │
│  │  │  • Confidence calibration summaries                               │ │   │
│  │  └─────────────────────────────────────────────────────────────────┘ │   │
│  └──────────────────────────────────┬───────────────────────────────────┘   │
│                                     │                                       │
│  ┌──────────────────────────────────▼───────────────────────────────────┐   │
│  │                      SIMULATION LAYER                                 │   │
│  │  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────────┐   │   │
│  │  │   Delta Store   │  │  Delta Vector   │  │   Impact Analyzer   │   │   │
│  │  │   (HashMap)     │  │     Index       │  │                     │   │   │
│  │  └─────────────────┘  └─────────────────┘  └─────────────────────┘   │   │
│  └──────────────────────────────────┬───────────────────────────────────┘   │
└─────────────────────────────────────┼───────────────────────────────────────┘
                                      │
                                      ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         KYRODB STORAGE ENGINE                                │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                          Entity Store                                │    │
│  │         • Entity CRUD  • Resolution  • Versioning                    │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                          Belief Store                                │    │
│  │    • Temporal Index  • Predicate Index  • Entity-Belief Links        │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         Pattern Store                                │    │
│  │              • Pattern CRUD  • Domain Index                          │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                         Conflict Store                               │    │
│  │          • Conflict Records  • Resolution History                    │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                     EXISTING KYRODB COMPONENTS                       │    │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌────────────┐  │    │
│  │  │ HNSW Vector │  │   Hybrid    │  │   Tiered    │  │ Persistence│  │    │
│  │  │    Index    │  │  Semantic   │  │   Engine    │  │   (WAL)    │  │    │
│  │  │             │  │   Cache     │  │             │  │            │  │    │
│  │  └─────────────┘  └─────────────┘  └─────────────┘  └────────────┘  │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Execution Paths

### 3.1 Path Definitions

| Path           | Latency Target  | Characteristics                           |
| -------------- | --------------- | ----------------------------------------- |
| **Reflex**     | < 5ms P99       | Fast, bounded, no allocations on hot path |
| **Reflection** | < 500ms P99     | Slower, may involve complex operations    |
| **Monitor**    | N/A (streaming) | Long-lived connections, event-driven      |

---

## 4. Trust and Meta-Knowledge

### 4.1 Trust Weighting (RESOLVE-time)

KyroQL models **trust separately from epistemic confidence**.

- Stored belief confidence remains immutable.
- During RESOLVE, the engine may scale a belief's confidence by a trust weight derived from its source.
- Trust weights can be scoped by a `trust_domain` (for example, a predicate or topic). If `trust_domain` is omitted, RESOLVE defaults it to the resolved predicate when available.

This makes trust an explicit, auditable assumption of the query rather than a silent rewrite of belief confidence.

### 4.2 Meta-Knowledge APIs

The engine exposes a meta-knowledge surface for inspection and tooling (coverage maps, gap analysis, and confidence calibration summaries). These are computed from the stores and do not change the stored beliefs.

### 3.2 Path Routing Logic

```rust
impl QueryRouter {
    pub fn route(&self, op: &Operation) -> ExecutionPath {
        match op {
            // ASSERT routing
            Operation::Assert(payload) => {
                match payload.consistency_mode {
                    ConsistencyMode::Force => ExecutionPath::Reflex,
                    ConsistencyMode::Strict | ConsistencyMode::Eventual => ExecutionPath::Reflection,
                }
            }

            // RESOLVE routing
            Operation::Resolve(payload) => {
                match payload.mode {
                    ResolveMode::Simple => ExecutionPath::Reflex,
                    ResolveMode::Aggregate | ResolveMode::Temporal => ExecutionPath::Reflection,
                }
            }

            // SIMULATE always reflection
            Operation::Simulate(_) => ExecutionPath::Reflection,

            // MONITOR always monitor path
            Operation::Monitor(_) => ExecutionPath::Monitor,

            // DERIVE is reflection (involves reasoning)
            Operation::Derive(_) => ExecutionPath::Reflection,

            // Entity operations are typically fast
            Operation::CreateEntity(_) => ExecutionPath::Reflex,
            Operation::GetEntity(_) => ExecutionPath::Reflex,
            Operation::UpdateEntity(_) => ExecutionPath::Reflex,

            // Pattern operations
            Operation::CreatePattern(_) => ExecutionPath::Reflection,
            Operation::GetPattern(_) => ExecutionPath::Reflex,
        }
    }
}
```

### 3.3 Thread Pool Design

```
┌─────────────────────────────────────────────────────────────────┐
│                        THREAD POOLS                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │                  REFLEX POOL                               │  │
│  │  • CPU cores / 2 threads (minimum 4)                       │  │
│  │  • High priority                                           │  │
│  │  • Never blocked by Reflection work                        │  │
│  │  • Simple RESOLVE, Fast ASSERT, Entity lookups             │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │                  REFLECTION POOL                           │  │
│  │  • CPU cores threads (minimum 4)                           │  │
│  │  • Normal priority                                         │  │
│  │  • SIMULATE, Temporal RESOLVE, Pattern checking            │  │
│  │  • May spawn subtasks                                      │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │                  BACKGROUND POOL                           │  │
│  │  • 2-4 threads                                             │  │
│  │  • Low priority                                            │  │
│  │  • Async consistency checks, Index maintenance             │  │
│  │  • Trigger evaluation                                      │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                 │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │                  MONITOR POOL                              │  │
│  │  • Tokio async runtime                                     │  │
│  │  • Handles gRPC streaming                                  │  │
│  │  • Event dispatch                                          │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## 5. Component Details

### 5.1 Query Router

**Responsibility**: Route incoming operations to appropriate execution path.

```rust
pub struct QueryRouter {
    reflex_executor: Arc<ReflexExecutor>,
    reflection_executor: Arc<ReflectionExecutor>,
    monitor_executor: Arc<MonitorExecutor>,
}

impl QueryRouter {
    pub async fn execute(&self, ir: KyroIR) -> Result<Response, KyroError> {
        let path = self.route(&ir.operation);

        match path {
            ExecutionPath::Reflex => self.reflex_executor.execute(ir).await,
            ExecutionPath::Reflection => self.reflection_executor.execute(ir).await,
            ExecutionPath::Monitor => self.monitor_executor.execute(ir).await,
        }
    }
}
```

### 5.2 Inference Layer

**Responsibility**: Apply conflict resolution policies and pattern checking.

```rust
pub struct InferenceLayer {
    pattern_store: Arc<PatternStore>,
    conflict_detector: Arc<ConflictDetector>,
    policy_executor: Arc<PolicyExecutor>,
}

impl InferenceLayer {
    /// Check a belief against stored patterns
    pub async fn check_patterns(&self, belief: &Belief) -> Result<Vec<Conflict>, KyroError> {
        let relevant_patterns = self.pattern_store
            .get_by_entity_type(&belief.subject)
            .await?;

        let mut violations = Vec::new();
        for pattern in relevant_patterns {
            if let Some(violation) = pattern.check(belief)? {
                violations.push(violation);
            }
        }
        Ok(violations)
    }

    /// Resolve conflicts according to policy
    pub async fn resolve_conflicts(
        &self,
        conflicts: &[Conflict],
        policy: &ConflictResolutionPolicy,
    ) -> Result<Option<BeliefId>, KyroError> {
        self.policy_executor.resolve(conflicts, policy).await
    }
}
```

### 5.3 Simulation Layer

**Responsibility**: Manage simulation contexts with delta stores.

```rust
pub struct SimulationLayer {
    base_engine: Arc<dyn StorageEngine>,
    active_simulations: RwLock<HashMap<SimulationId, Arc<SimulationContext>>>,
}

impl SimulationLayer {
    pub async fn create_simulation(
        &self,
        constraints: SimulateConstraints,
    ) -> Result<SimulationId, KyroError> {
        let sim_id = SimulationId::new();

        let context = Arc::new(SimulationContext::new(
            self.base_engine.clone(),
            constraints,
        ));

        self.active_simulations.write().await.insert(sim_id, context);

        Ok(sim_id)
    }

    pub async fn get_simulation(
        &self,
        id: SimulationId,
    ) -> Result<Arc<SimulationContext>, KyroError> {
        self.active_simulations
            .read()
            .await
            .get(&id)
            .cloned()
            .ok_or(KyroError::SimulationNotFound { id })
    }

    pub async fn end_simulation(&self, id: SimulationId) -> Result<(), KyroError> {
        self.active_simulations.write().await.remove(&id);
        Ok(())
    }
}
```

### 5.4 Monitor Layer

**Responsibility**: Manage triggers and event dispatch.

```rust
pub struct MonitorLayer {
    trigger_store: Arc<TriggerStore>,
    trigger_matcher: Arc<TriggerMatcher>,
    event_dispatcher: Arc<EventDispatcher>,
}

impl MonitorLayer {
    /// Called on every ASSERT to check triggers
    pub async fn on_belief_created(&self, belief: &Belief) -> Result<(), KyroError> {
        let matching_triggers = self.trigger_matcher.match_belief(belief).await?;

        for trigger in matching_triggers {
            let event = MonitorEvent {
                event_id: Uuid::new_v4(),
                trigger_id: trigger.id,
                trigger_type: trigger.trigger_type.clone(),
                timestamp: Utc::now(),
                payload: EventPayload::BeliefCreated {
                    belief_id: belief.id
                },
            };

            self.event_dispatcher.dispatch(event).await?;
        }

        Ok(())
    }

    /// Called on conflict detection
    pub async fn on_conflict_created(&self, conflict: &Conflict) -> Result<(), KyroError> {
        let matching_triggers = self.trigger_matcher.match_conflict(conflict).await?;

        for trigger in matching_triggers {
            let event = MonitorEvent {
                event_id: Uuid::new_v4(),
                trigger_id: trigger.id,
                trigger_type: trigger.trigger_type.clone(),
                timestamp: Utc::now(),
                payload: EventPayload::ConflictCreated {
                    conflict_id: conflict.id
                },
            };

            self.event_dispatcher.dispatch(event).await?;
        }

        Ok(())
    }
}
```

---

## 6. Storage Integration

### 6.1 New Stores Required

| Store             | Purpose                        | Primary Key  | Secondary Indexes                               |
| ----------------- | ------------------------------ | ------------ | ----------------------------------------------- |
| **EntityStore**   | Entity CRUD and resolution     | `EntityId`   | `name`, `aliases`, `entity_type`                |
| **BeliefStore**   | Belief storage with bitemporal | `BeliefId`   | `(subject, predicate)`, `valid_time`, `tx_time` |
| **PatternStore**  | Pattern definitions            | `PatternId`  | `domain`, `entity_type`                         |
| **ConflictStore** | Conflict tracking              | `ConflictId` | `status`, `entity_id`                           |
| **TriggerStore**  | Monitor subscriptions          | `TriggerId`  | `trigger_type`, `entity_id`                     |

### 6.2 Integration with Existing KyroDB

```rust
/// Wrapper that integrates KyroQL stores with existing KyroDB engine
pub struct KyroQLStorageEngine {
    // New KyroQL stores
    entity_store: EntityStore,
    belief_store: BeliefStore,
    pattern_store: PatternStore,
    conflict_store: ConflictStore,
    trigger_store: TriggerStore,

    // Existing KyroDB components (reused)
    hnsw_index: Arc<HnswBackend>,
    semantic_cache: Arc<LearnedCache>,
    tiered_engine: Arc<TieredEngine>,
    persistence: Arc<PersistenceManager>,
}

impl KyroQLStorageEngine {
    /// Use existing HNSW for vector search
    pub async fn vector_search(&self, query: &[f32], k: usize) -> Vec<BeliefId> {
        let results = self.hnsw_index.search(query, k).await;
        // Map internal IDs to BeliefIds
        results.iter().map(|r| self.id_mapping.get(r.id)).collect()
    }

    /// Use existing semantic cache for query caching
    pub async fn cached_resolve(&self, query_hash: u64) -> Option<BeliefFrame> {
        self.semantic_cache.get(query_hash).await
    }
}
```

---

## 7. Wire Protocol

### 7.1 gRPC Service Definition

```protobuf
syntax = "proto3";

package kyroql.v1;

service KyroQL {
    // Core operations
    rpc Assert(AssertRequest) returns (AssertResponse);
    rpc Resolve(ResolveRequest) returns (ResolveResponse);
    rpc Simulate(SimulateRequest) returns (SimulateResponse);

    // Simulation operations (use simulation_id from SimulateResponse)
    rpc SimulateAssert(SimulateAssertRequest) returns (SimulateAssertResponse);
    rpc SimulateResolve(SimulateResolveRequest) returns (SimulateResolveResponse);
    rpc EndSimulation(EndSimulationRequest) returns (EndSimulationResponse);

    // Monitor (streaming)
    rpc Monitor(MonitorRequest) returns (stream MonitorEvent);

    // Entity operations
    rpc CreateEntity(CreateEntityRequest) returns (CreateEntityResponse);
    rpc GetEntity(GetEntityRequest) returns (GetEntityResponse);
    rpc ResolveEntity(ResolveEntityRequest) returns (ResolveEntityResponse);

    // Pattern operations
    rpc CreatePattern(CreatePatternRequest) returns (CreatePatternResponse);
    rpc GetPattern(GetPatternRequest) returns (GetPatternResponse);

    // Meta-knowledge
    rpc GetCoverage(GetCoverageRequest) returns (GetCoverageResponse);
    rpc GetGaps(GetGapsRequest) returns (GetGapsResponse);
}

message AssertRequest {
    bytes entity_id = 1;  // UUID bytes
    string predicate = 2;
    Value value = 3;
    Confidence confidence = 4;
    Source source = 5;
    TimeRange valid_time = 6;
    ConsistencyMode consistency_mode = 7;
}

message AssertResponse {
    bytes belief_id = 1;  // UUID bytes
    repeated Conflict conflicts = 2;  // Any detected conflicts
}

message ResolveRequest {
    oneof query {
        bytes embedding = 1;
        string text_question = 2;
        EntityPredicateQuery entity_predicate = 3;
    }
    optional float min_confidence = 4;
    optional int64 as_of_timestamp_ms = 5;
    ConflictResolutionPolicy conflict_policy = 6;
    uint32 limit = 7;
    ResolveMode mode = 8;
}

message ResolveResponse {
    BeliefFrame frame = 1;
}

// ... additional message definitions
```

---

## 8. Crate Structure

```
KyroQL/
├── Cargo.toml              # Workspace definition
├── kyroql-core/            # Core types and traits
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── entity.rs       # Entity types
│       ├── belief.rs       # Belief types
│       ├── confidence.rs   # Confidence types
│       ├── value.rs        # Value enum
│       ├── time.rs         # Temporal types
│       ├── conflict.rs     # Conflict types
│       ├── pattern.rs      # Pattern types
│       ├── frame.rs        # BeliefFrame
│       └── error.rs        # Error types
├── kyroql-ir/              # Intermediate Representation
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── operations.rs   # Operation enum
│       ├── payloads.rs     # Request payloads
│       └── serialization.rs
├── kyroql-client/          # Client SDKs
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── client.rs       # Main client
│       ├── builders/       # Fluent builders
│       │   ├── mod.rs
│       │   ├── assert.rs
│       │   ├── resolve.rs
│       │   ├── simulate.rs
│       │   └── monitor.rs
│       └── transport.rs    # gRPC transport
├── kyroql-server/          # Server implementation
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── router.rs       # Query router
│       ├── executors/      # Path executors
│       │   ├── mod.rs
│       │   ├── reflex.rs
│       │   ├── reflection.rs
│       │   └── monitor.rs
│       ├── inference/      # Inference layer
│       │   ├── mod.rs
│       │   ├── policies.rs
│       │   └── checker.rs
│       ├── simulation/     # Simulation layer
│       │   ├── mod.rs
│       │   ├── context.rs
│       │   ├── delta_store.rs
│       │   └── delta_index.rs
│       └── grpc.rs         # gRPC service impl
├── kyroql-storage/         # Storage implementations
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── entity_store.rs
│       ├── belief_store.rs
│       ├── pattern_store.rs
│       ├── conflict_store.rs
│       └── trigger_store.rs
├── kyroql-embedded/        # Embedded mode (zero-friction)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs          # Main entry point
│       ├── database.rs     # KyroDatabase struct
│       ├── sqlite_store.rs # SQLite-backed storage
│       ├── small_hnsw.rs   # In-memory vector index
│       └── file_format.rs  # .kyro file format
└── kyroql-python/          # Python bindings
    ├── Cargo.toml
    ├── pyproject.toml
    └── src/
        └── lib.rs          # PyO3 bindings (both embedded & client)
```

---

## 9. Performance Constraints

### 9.1 Reflex Path Constraints

```rust
/// Reflex path performance requirements
pub struct ReflexConstraints {
    /// Maximum latency for P99
    pub max_latency_p99_ms: u64, // 5ms

    /// Maximum heap allocations per operation
    pub max_allocations: usize, // 0 on hot path

    /// Maximum operations before yielding
    pub max_ops_before_yield: usize, // 100
}

impl Default for ReflexConstraints {
    fn default() -> Self {
        Self {
            max_latency_p99_ms: 5,
            max_allocations: 0,
            max_ops_before_yield: 100,
        }
    }
}
```

### 9.2 Reflection Path Constraints

```rust
/// Reflection path performance requirements
pub struct ReflectionConstraints {
    /// Maximum latency for P99
    pub max_latency_p99_ms: u64, // 500ms

    /// Maximum concurrent reflection operations
    pub max_concurrent: usize, // CPU cores * 2

    /// Maximum memory per operation
    pub max_memory_mb: usize, // 256MB
}
```

### 9.3 Simulation Constraints

```rust
/// Simulation constraints (enforced)
pub struct SimulateConstraints {
    /// Maximum entities affected
    pub max_affected_entities: usize, // 1000

    /// Maximum depth of impact analysis
    pub max_depth: usize, // 2

    /// Maximum duration before timeout
    pub max_duration_ms: u64, // 500

    /// Maximum memory for delta store
    pub max_memory_mb: usize, // 64MB
}
```

Enforcement notes:
- Quotas are per-simulation; exceeding any bound must abort the run and release delta stores.
- Delta overlays and vector buffers must be torn down on `end_simulation` or any failure path to prevent unbounded memory growth.

---

## 10. Observability

### 10.1 Metrics

```rust
/// Metrics exported by KyroQL
pub struct KyroQLMetrics {
    // Operation counts
    pub assert_total: Counter,
    pub resolve_total: Counter,
    pub simulate_total: Counter,
    pub monitor_total: Counter,

    // Latencies (histograms)
    pub assert_latency_ms: Histogram,
    pub resolve_latency_ms: Histogram,
    pub simulate_latency_ms: Histogram,

    // Path routing
    pub reflex_operations: Counter,
    pub reflection_operations: Counter,

    // Conflicts
    pub conflicts_detected: Counter,
    pub conflicts_resolved: Counter,

    // Simulations
    pub active_simulations: Gauge,
    pub simulation_memory_bytes: Gauge,

    // Errors
    pub validation_errors: Counter,
    pub execution_errors: Counter,
}
```

### 10.2 Logging

All operations log:

1. Request ID (for tracing)
2. Operation type
3. Execution path
4. Latency
5. Result status

IR can be logged for full replay capability.

Redact sensitive payload fields (PII, embeddings, raw prompts) before emission; full IR logging should be gated by an explicit audit level and scoped retention policy.

---

## 11. Security Considerations

### 11.1 Access Control

```rust
/// Access control for KyroQL operations
pub struct AccessControl {
    /// Who can read from which entities
    pub read_permissions: HashMap<AgentId, EntityScope>,

    /// Who can write to which entities
    pub write_permissions: HashMap<AgentId, EntityScope>,

    /// Who can create simulations
    pub simulate_permissions: HashSet<AgentId>,

    /// Who can monitor which triggers
    pub monitor_permissions: HashMap<AgentId, TriggerScope>,
}

pub enum EntityScope {
    All,
    ByType(Vec<EntityType>),
    ById(Vec<EntityId>),
    ByDomain(Vec<String>),
}
```

### 11.2 Audit Trail

Every operation is logged with:

- Agent ID
- Timestamp
- Full IR
- Result
- Affected entities

This enables compliance and forensics.
