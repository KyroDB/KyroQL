use std::sync::Arc;

use kyroql::{
    AssertBuilder, Belief, Confidence, EngineResponse, Entity, EntityType, KyroEngine,
    ResolveBuilder, SimulateBuilder, SimulateConstraints, Source, TimeRange, Value,
};

use kyroql::{BeliefStore, EntityStore};

#[test]
fn simulate_creates_isolated_overlay_and_does_not_mutate_base() {
    let entities = Arc::new(kyroql::InMemoryEntityStore::default());
    let beliefs = Arc::new(kyroql::InMemoryBeliefStore::default());
    let patterns = Arc::new(kyroql::InMemoryPatternStore::default());
    let conflicts = Arc::new(kyroql::InMemoryConflictStore::default());

    let engine = KyroEngine::new(
        entities.clone(),
        beliefs.clone(),
        patterns.clone(),
        conflicts.clone(),
    );

    let entity = Entity::new("sim_target", EntityType::Artifact);
    let entity_id = entity.id;
    entities.insert(entity).unwrap();

    let before = beliefs.count_by_entity(entity_id).unwrap();

    let assert_ir = AssertBuilder::new()
        .entity(entity_id)
        .predicate("baseline")
        .value(Value::Bool(true))
        .confidence(Confidence::from_agent(0.9, "test").unwrap())
        .source(Source::Unknown { description: None })
        .valid_time(TimeRange::from_now())
        .build()
        .unwrap();

    let _ = engine.execute(assert_ir).unwrap();
    let after_assert = beliefs.count_by_entity(entity_id).unwrap();
    assert_eq!(after_assert, before + 1);

    let simulate_ir = SimulateBuilder::new()
        .constraints(SimulateConstraints {
            max_affected_entities: 10,
            max_depth: 2,
            max_duration_ms: 500,
        })
        .build()
        .unwrap();

    let EngineResponse::Simulate { simulation } = engine.execute(simulate_ir).unwrap() else {
        panic!("expected EngineResponse::Simulate");
    };

    let hypothetical = Belief::builder()
        .subject(entity_id)
        .predicate("hypothetical")
        .value(Value::Int(42))
        .confidence(Confidence::from_agent(0.8, "sim").unwrap())
        .source(Source::Unknown { description: None })
        .valid_time(TimeRange::from_now())
        .build()
        .unwrap();

    simulation.assert_hypothetical(hypothetical).unwrap();

    let impact = simulation.query_impact().unwrap();
    assert_eq!(impact.inserted_beliefs, 1);
    assert_eq!(impact.affected_entities, vec![entity_id]);
    assert_eq!(impact.inserted_belief_ids.len(), 1);

    // Base RESOLVE cannot see the hypothetical.
    let base_resolve = ResolveBuilder::new()
        .entity(entity_id)
        .predicate("hypothetical")
        .build()
        .unwrap();
    let EngineResponse::Resolve { frame } = engine.execute(base_resolve).unwrap() else {
        panic!("expected EngineResponse::Resolve");
    };
    assert!(!frame.has_answer());

    // Simulation RESOLVE sees base+delta (including hypotheticals).
    let sim_resolve = ResolveBuilder::new()
        .entity(entity_id)
        .predicate("hypothetical")
        .build()
        .unwrap();
    let frame = simulation.resolve_ir(sim_resolve).unwrap();
    assert!(frame.has_answer());
    assert_eq!(
        frame.best_supported_claim.unwrap().belief.value,
        Value::Int(42)
    );

    // Base is unchanged.
    let during = beliefs.count_by_entity(entity_id).unwrap();
    assert_eq!(during, after_assert);

    drop(simulation);

    // Still unchanged after simulation drops.
    let after = beliefs.count_by_entity(entity_id).unwrap();
    assert_eq!(after, after_assert);
}
