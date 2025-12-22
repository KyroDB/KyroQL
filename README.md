# KyroQL

**The Cognitive Protocol for Superintelligence**

---

## What is KyroQL?

KyroQL is not a query language. It is a **protocol for synchronizing belief states** between intelligent agents and their memory substrate.

> **Core Thesis**: SQL and Vector Search are category errors for AGI. KyroQL provides the System 2 substrate for deliberate, logical, counterfactual reasoning.

---

## Documentation

| Document                                           | Description                                        |
| -------------------------------------------------- | -------------------------------------------------- |
| [VISION.md](./VISION.md)                           | Complete vision, philosophy, and design principles |
| [IMPLEMENTATION_PLAN.md](./IMPLEMENTATION_PLAN.md) | Phased implementation roadmap                      |
| [DATA_MODEL.md](./DATA_MODEL.md)                   | Complete type definitions                          |
| [ARCHITECTURE.md](./ARCHITECTURE.md)               | System architecture and components                 |

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
// ASSERT a belief
let belief_id = client.assert()
    .entity(entity_id)
    .predicate("is_superconductor")
    .value(false)
    .confidence(Confidence::probability(0.99, source))
    .source(Source::Paper { arxiv_id: "2308.12345" })
    .valid_time(TimeRange::from_now())
    .execute()
    .await?;

// RESOLVE a question
let frame = client.resolve()
    .question("Is LK-99 a superconductor?")
    .min_confidence(0.5)
    .conflict_policy(ConflictResolutionPolicy::HighestConfidence)
    .execute()
    .await?;

// The response is structured, not prose
println!("Answer: {:?}", frame.best_supported_claim);
println!("Confidence: {}", frame.epistemic_confidence);
println!("Gaps: {:?}", frame.gaps);

// SIMULATE a counterfactual
let sim = client.simulate().execute().await?;
sim.assert_hypothetical(hypothesis)?;
let impact = sim.query_impact().execute().await?;
drop(sim); // Reality unchanged
```

### Python

```python
# ASSERT a belief
belief_id = await client.assert_(
    entity=entity_id,
    predicate="is_superconductor",
    value=False,
    confidence=Confidence.probability(0.99),
    source=Source.paper(arxiv_id="2308.12345"),
)

# RESOLVE a question
frame = await client.resolve(
    question="Is LK-99 a superconductor?",
    min_confidence=0.5,
)

print(f"Answer: {frame.best_supported_claim}")
print(f"Gaps: {frame.gaps}")
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

See [IMPLEMENTATION_PLAN.md](./IMPLEMENTATION_PLAN.md) for detailed tasks.

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

🚧 **Design Phase** - Documentation complete, implementation starting

---

## License

Same as KyroDB (BSL)
