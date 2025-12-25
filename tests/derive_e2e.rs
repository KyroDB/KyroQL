use kyroql::{
    AssertBuilder, Confidence, ConsistencyMode, DeriveBuilder, Entity, EntityType,
    EntityStore, KyroEngine, Source, TimeRange, Value, ValidationError,
    InMemoryEntityStore, InMemoryBeliefStore, InMemoryPatternStore, InMemoryConflictStore, InMemoryDerivationStore,
};
use std::sync::Arc;

fn engine_with_entities() -> (KyroEngine, Arc<InMemoryEntityStore>) {
    let entities = Arc::new(InMemoryEntityStore::new());
    let beliefs = Arc::new(InMemoryBeliefStore::new());
    let patterns = Arc::new(InMemoryPatternStore::new());
    let conflicts = Arc::new(InMemoryConflictStore::new());
    let derivations = Arc::new(InMemoryDerivationStore::new());

    let engine = KyroEngine::new(
        entities.clone(),
        beliefs,
        patterns,
        conflicts,
        derivations,
    );
    (engine, entities)
}

#[test]
fn derive_simple_inference_chain() {
    let (engine, entities) = engine_with_entities();
    
    // 1. Setup Entities
    let socrates = Entity::new("Socrates", EntityType::Person);
    let socrates_id = socrates.id;
    entities.insert(socrates).unwrap();

    // Assert: Socrates is Human
    let human_op = AssertBuilder::new()
        .entity(socrates_id)
        .predicate("type")
        .value(Value::String("human".to_string()))
        .confidence(Confidence::one()) // Axiomatic
        .source(Source::unknown_with_description("definition"))
        .valid_time(TimeRange::forever())
        .consistency_mode(ConsistencyMode::Strict)
        .build()
        .unwrap();

    let resp = engine.execute(human_op).unwrap();
    let kyroql::EngineResponse::Assert { belief_id: premise_id, .. } = resp else {
        panic!("expected assert response");
    };

    // 2. Derive: "Socrates is mortal" from "Socrates is human"
    // Assert Conclusion
    let conclusion_op = AssertBuilder::new()
        .entity(socrates_id)
        .predicate("property")
        .value(Value::String("mortal".to_string()))
        .confidence(Confidence::from_agent(0.99, "inference_logic").unwrap())
        .source(Source::derived(vec![premise_id], "modus_ponens"))
        .valid_time(TimeRange::forever())
        .build()
        .unwrap();

    let resp = engine.execute(conclusion_op).unwrap();
    let kyroql::EngineResponse::Assert { belief_id: conclusion_id, .. } = resp else {
        panic!("expected assert response");
    };

    // 3. Record Derivation
    let derive_op = DeriveBuilder::new()
        .rule("all_humans_are_mortal")
        .add_source(premise_id)
        .derived_belief(conclusion_id)
        .add_step("check(Socrates.type == Human)")
        .add_step("apply(Human => Mortal)")
        .confidence(0.99)
        .build()
        .unwrap();

    let resp = engine.execute(derive_op).unwrap();
    let kyroql::EngineResponse::Derive { derivation_id } = resp else {
        panic!("expected derive response");
    };

    println!("Derivation recorded: {}", derivation_id);
}

#[test]
fn derive_detects_cycles() {
    let (engine, entities) = engine_with_entities();
    let entity = Entity::new("CycleTest", EntityType::Concept);
    let id = entity.id;
    entities.insert(entity).unwrap();

    // Assert P1
    let p1 = AssertBuilder::new()
        .entity(id)
        .predicate("p")
        .value(Value::Int(1))
        .confidence(Confidence::from_agent(0.9, "A").unwrap())
        .source(Source::agent("A", None::<String>))
        .valid_time(TimeRange::from_now())
        .build()
        .unwrap();
    let kyroql::EngineResponse::Assert { belief_id: b1, .. } = engine.execute(p1).unwrap() else {
        panic!("Using let else to satisfy irrefutable pattern");
    };

    // Try to derive B1 from B1 (Cycle)
    let cycle_build_result = DeriveBuilder::new()
        .rule("bad_rule")
        .add_source(b1)
        .derived_belief(b1)
        .build(); // Should fail validation here
    
    assert!(cycle_build_result.is_err());
    match cycle_build_result.unwrap_err() {
        ValidationError::InvalidField { field, reason } => {
            assert_eq!(field, "derived_belief_id");
            assert!(reason.contains("must not appear in sources"));
        }
        err => panic!("Expected InvalidField error, got {:?}", err),
    }
}
