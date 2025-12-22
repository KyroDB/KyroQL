# KyroQL

**The Cognitive Protocol for Superintelligence**

---

## What is KyroQL?

KyroQL is not a query language. It is a **protocol for synchronizing belief states** between intelligent agents and their memory substrate.

> **Core Thesis**: SQL and Vector Search are category errors for AGI. KyroQL provides the System 2 substrate for deliberate, logical, counterfactual reasoning.

---


## Quick Overview

### The Paradigm Shift

| Old Paradigm        | KyroQL Paradigm                                                      |
| ------------------- | -------------------------------------------------------------------- |
| `INSERT` / `UPDATE` | **ASSERT** - Integrate belief with confidence, provenance, validity  |
| `SELECT`            | **RESOLVE** - Synthesize answer from evidence with conflict handling |
| _(none)_            | **SIMULATE** - Branch reality for hypothesis testing                 |
| `TRIGGER`           | **MONITOR** - Subscribe to belief changes, conflicts, patterns       |

### Core Principles

1. **Certainty is a Variable** - Every belief has explicit confidence
2. **Time is a Dimension** - Bitemporal: valid_time + tx_time
3. **Absence is Information** - Gaps are structured responses, not NULL
4. **Queries are Counterfactual** - SIMULATE enables "what if" reasoning

---

## API Preview

### Rust

```rust
// NOTE: Networked client APIs (ASSERT/RESOLVE/SIMULATE) are planned.
// Today, this crate provides the core data model + validation.

use kyroql::{Belief, Confidence, Entity, EntityType, Source, TimeRange, Value};

let entity = Entity::new("LK-99", EntityType::Concept);
let belief = Belief::builder()
    .subject(entity.id)
    .predicate("is_superconductor")
    .value(Value::Bool(false))
    .confidence(Confidence::probability(0.99, "researcher-1")?)
    .source(Source::paper("2308.12345", "LK-99 report"))
    .valid_time(TimeRange::from_now())
    .build()?;

println!("belief: {}", belief.id);
```

### Python

> Python client APIs are planned; the current repository ships a Rust crate.

```python
raise NotImplementedError("Python client is not yet shipped in this repo")
```

---

## Implementation Phases

| Phase              | Focus                               | Timeline    |
| ------------------ | ----------------------------------- | ----------- |
| **0: Foundations** | Entity layer, Belief schema, IR     | Weeks 1-3   |
| **1: Core Ops**    | ASSERT, RESOLVE, Conflict detection | Weeks 4-7   |
| **2: Inference**   | Resolution policies, Pattern store  | Weeks 8-10  |
| **3: Simulation**  | Delta stores, SIMULATE              | Weeks 11-14 |
| **4: Monitoring**  | MONITOR, Event streaming            | Weeks 15-17 |
| **5: Trust**       | Trust model, Meta-cognition         | Weeks 18-20 |


---

## Key Differentiators

### From SQL

-   Open world assumption (not closed world)
-   Probabilistic truth (not binary)
-   Temporal semantics built-in
-   Contradiction handling

### From Vector DBs

-   Structured beliefs, not just embeddings
-   Confidence and provenance tracking
-   Conflict detection and resolution
-   Counterfactual simulation

### From Knowledge Graphs

-   Epistemic metadata (confidence, source)
-   Bitemporal querying
-   Gap detection ("what's missing?")
-   Reactive monitoring

---

## Design Constraints (Non-Negotiable)

1. **Reflex path < 5ms P99** - Never block fast operations
2. **Confidence is required** - No "vibes" allowed
3. **Provenance is required** - Know where beliefs come from
4. **SIMULATE is bounded** - Hard limits on resources
5. **Policy, not reasoning** - DB applies policies, agents reason

---

## Status

**Active Development** - Core data model is implemented; client/server protocol APIs are planned.

---

## License

Same as KyroDB (BSL)
