//! gRPC transport layer for KyroQL.
//!
//! Vision constraint: the canonical protocol surface is `KyroIR`.
//! This transport therefore carries `KyroIR` as JSON bytes and returns
//! JSON-serialized response objects.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::RwLock;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use std::time::Duration;

use crate::belief::{Belief, ConsistencyStatus};
use crate::confidence::BeliefId;
use crate::engine::{EngineResponse, KyroEngine};
use crate::error::{ExecutionError, KyroError, ValidationError};
use crate::ir::{ConsistencyMode, KyroIR, Operation};
use crate::monitor::MonitorStream;
use crate::simulation::{SimulationCommitResult, SimulationContext, SimulationImpact};

pub mod proto {
    tonic::include_proto!("kyroql");
}

use proto::kyro_service_server::{KyroService, KyroServiceServer};

// ----------------------------------------------------------------------------
// Limits (DoS protection)
// ----------------------------------------------------------------------------

/// Maximum size of a KyroIR JSON payload.
const MAX_IR_JSON_BYTES: usize = 1024 * 1024; // 1 MiB

/// Maximum size of a response JSON payload.
const MAX_RESPONSE_JSON_BYTES: usize = 4 * 1024 * 1024; // 4 MiB

/// Maximum size of a monitor event JSON payload.
const MAX_EVENT_JSON_BYTES: usize = 1024 * 1024; // 1 MiB

/// In-memory simulation registry cap (server-side safety).
const MAX_OPEN_SIMULATIONS: usize = 4096;

/// gRPC service implementation for KyroQL.
pub struct KyroServiceImpl {
    engine: Arc<KyroEngine>,
    simulations: RwLock<HashMap<uuid::Uuid, Arc<SimulationContext>>>,
}

impl KyroServiceImpl {
    #[must_use]
    pub fn new(engine: Arc<KyroEngine>) -> Self {
        Self {
            engine,
            simulations: RwLock::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn into_server(self) -> KyroServiceServer<Self> {
        KyroServiceServer::new(self)
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TransportResponse {
    Assert {
        belief_id: BeliefId,
        conflict_ids: Vec<crate::conflict::ConflictId>,
    },
    Resolve {
        frame: crate::frame::BeliefFrame,
    },
    Retract {
        retraction_belief_id: BeliefId,
    },
    DefinePattern {
        pattern_id: crate::pattern::PatternId,
    },
    Derive {
        derivation_id: crate::derivation::DerivationId,
    },
}

fn invalid_argument(msg: impl Into<String>) -> Status {
    Status::invalid_argument(msg.into())
}

fn parse_uuid(s: &str) -> Result<uuid::Uuid, Status> {
    if s.len() > 64 {
        return Err(invalid_argument("uuid string too long"));
    }
    s.parse().map_err(|_| invalid_argument("invalid UUID format"))
}

fn parse_ir(bytes: &[u8]) -> Result<KyroIR, Status> {
    if bytes.is_empty() {
        return Err(invalid_argument("ir_json is required"));
    }
    if bytes.len() > MAX_IR_JSON_BYTES {
        return Err(invalid_argument("ir_json exceeds maximum size"));
    }

    let ir: KyroIR = serde_json::from_slice(bytes)
        .map_err(|e| invalid_argument(format!("invalid KyroIR JSON: {e}")))?;
    Ok(ir)
}

fn encode_json<T: Serialize>(value: &T, max: usize) -> Result<Vec<u8>, Status> {
    let bytes = serde_json::to_vec(value)
        .map_err(|e| Status::internal(format!("failed to serialize response JSON: {e}")))?;
    if bytes.len() > max {
        return Err(Status::resource_exhausted("serialized JSON exceeds size limit"));
    }
    Ok(bytes)
}

fn status_from_kyro_error(err: KyroError) -> Status {
    match err {
        KyroError::Validation(v) => Status::invalid_argument(v.to_string()),
        KyroError::Transport(t) => Status::unavailable(t.to_string()),
        KyroError::Internal { message } => Status::internal(message),
        KyroError::Execution(e) => match e {
            ExecutionError::EntityNotFound { .. }
            | ExecutionError::BeliefNotFound { .. }
            | ExecutionError::SimulationNotFound { .. } => Status::not_found(e.to_string()),

            ExecutionError::Timeout { .. } => Status::deadline_exceeded(e.to_string()),
            ExecutionError::QueueFull { .. } | ExecutionError::SimulationLimitExceeded { .. } => {
                Status::resource_exhausted(e.to_string())
            }

            ExecutionError::InvalidOperation { .. } => Status::invalid_argument(e.to_string()),
            ExecutionError::NotImplemented { .. } => Status::unimplemented(e.to_string()),

            ExecutionError::ConflictsDetected { .. }
            | ExecutionError::PatternViolation { .. }
            | ExecutionError::ConflictResolutionFailed { .. }
            | ExecutionError::SimulationCommitNotAllowed { .. }
            | ExecutionError::SimulationPartialCommit { .. } => Status::failed_precondition(e.to_string()),

            ExecutionError::InvalidDerivation { .. } => Status::invalid_argument(e.to_string()),
            ExecutionError::Storage { .. } | ExecutionError::Index { .. } | ExecutionError::Disconnected { .. } => {
                Status::internal(e.to_string())
            }
        },
    }
}

fn to_transport_response(resp: EngineResponse) -> Result<TransportResponse, Status> {
    match resp {
        EngineResponse::Assert {
            belief_id,
            conflict_ids,
        } => Ok(TransportResponse::Assert {
            belief_id,
            conflict_ids,
        }),
        EngineResponse::Resolve { frame } => Ok(TransportResponse::Resolve { frame }),
        EngineResponse::Retract {
            retraction_belief_id,
        } => Ok(TransportResponse::Retract {
            retraction_belief_id,
        }),
        EngineResponse::DefinePattern { pattern_id } => Ok(TransportResponse::DefinePattern { pattern_id }),
        EngineResponse::Derive { derivation_id } => Ok(TransportResponse::Derive { derivation_id }),
        EngineResponse::Simulate { .. } => Err(Status::invalid_argument(
            "simulate responses are only returned via SimulateCreate",
        )),
        EngineResponse::Monitor { .. } => Err(Status::invalid_argument(
            "monitor responses are only returned via Monitor stream",
        )),
    }
}

fn parse_consistency_mode(mode: &str) -> Result<ConsistencyMode, Status> {
    if mode.len() > 64 {
        return Err(invalid_argument("consistency_mode too long"));
    }
    let quoted = format!("\"{mode}\"");
    serde_json::from_str::<ConsistencyMode>(&quoted)
        .map_err(|_| invalid_argument("invalid consistency_mode"))
}

fn build_hypothetical_belief(ir: &KyroIR) -> Result<Belief, Status> {
    let Operation::Assert(payload) = &ir.operation else {
        return Err(invalid_argument("expected assert operation"));
    };

    payload.validate().map_err(|e: ValidationError| status_from_kyro_error(KyroError::from(e)))?;

    Ok(Belief {
        id: BeliefId::new(),
        subject: payload.entity_id,
        predicate: payload.predicate.clone(),
        value: payload.value.clone(),
        confidence: payload.confidence.clone(),
        source: payload.source.clone(),
        valid_time: payload.valid_time.clone(),
        tx_time: ir.timestamp,
        reason: None,
        consistency_status: ConsistencyStatus::Provisional,
        supersedes: None,
        superseded_by: None,
        embedding: payload.embedding.clone(),
    })
}

#[tonic::async_trait]
impl KyroService for KyroServiceImpl {
    async fn execute(
        &self,
        request: Request<proto::ExecuteRequest>,
    ) -> Result<Response<proto::ExecuteResponse>, Status> {
        let req = request.into_inner();
        let ir = parse_ir(&req.ir_json)?;

        match ir.operation {
            Operation::Monitor(_) => {
                return Err(invalid_argument("monitor operation must use Monitor RPC"));
            }
            Operation::Simulate(_) => {
                return Err(invalid_argument("simulate operation must use SimulateCreate RPC"));
            }
            _ => {}
        }

        let resp = self.engine.execute(ir).map_err(status_from_kyro_error)?;
        let out = to_transport_response(resp)?;
        let response_json = encode_json(&out, MAX_RESPONSE_JSON_BYTES)?;
        Ok(Response::new(proto::ExecuteResponse { response_json }))
    }

    type MonitorStream = ReceiverStream<Result<proto::MonitorEvent, Status>>;

    async fn monitor(
        &self,
        request: Request<proto::MonitorRequest>,
    ) -> Result<Response<Self::MonitorStream>, Status> {
        let req = request.into_inner();
        let ir = parse_ir(&req.ir_json)?;

        if !matches!(ir.operation, Operation::Monitor(_)) {
            return Err(invalid_argument("MonitorRequest must contain op=monitor"));
        }

        let resp = self.engine.execute(ir).map_err(status_from_kyro_error)?;
        let EngineResponse::Monitor { registration } = resp else {
            return Err(Status::internal("engine returned non-monitor response"));
        };

        let stream: MonitorStream = registration.stream;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<proto::MonitorEvent, Status>>(128);
        tokio::task::spawn_blocking(move || loop {
            match stream.recv_timeout(Duration::from_secs(5)) {
                Ok(event) => {
                    let encoded = match serde_json::to_vec(&event) {
                        Ok(v) => v,
                        Err(e) => {
                            let _ = tx.blocking_send(Err(Status::internal(format!(
                                "failed to serialize monitor event: {e}"
                            ))));
                            stream.unsubscribe();
                            break;
                        }
                    };

                    if encoded.len() > MAX_EVENT_JSON_BYTES {
                        let _ = tx.blocking_send(Err(Status::resource_exhausted(
                            "monitor event exceeds maximum size",
                        )));
                        stream.unsubscribe();
                        break;
                    }

                    if tx
                        .blocking_send(Ok(proto::MonitorEvent { event_json: encoded }))
                        .is_err()
                    {
                        stream.unsubscribe();
                        break;
                    }
                }
                Err(err) => {
                    // Timeout: check for client disconnect, otherwise keep polling.
                    if matches!(err, KyroError::Execution(ExecutionError::Timeout { .. })) {
                        if tx.is_closed() {
                            stream.unsubscribe();
                            break;
                        }
                        continue;
                    }

                    let _ = tx.blocking_send(Err(status_from_kyro_error(err)));
                    stream.unsubscribe();
                    break;
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn simulate_create(
        &self,
        request: Request<proto::SimulateCreateRequest>,
    ) -> Result<Response<proto::SimulateCreateResponse>, Status> {
        let req = request.into_inner();
        let ir = parse_ir(&req.ir_json)?;

        if !matches!(ir.operation, Operation::Simulate(_)) {
            return Err(invalid_argument("SimulateCreateRequest must contain op=simulate"));
        }

        let resp = self.engine.execute(ir).map_err(status_from_kyro_error)?;
        let EngineResponse::Simulate { simulation } = resp else {
            return Err(Status::internal("engine returned non-simulate response"));
        };

        let simulation_id = simulation.id.to_string();
        let id = parse_uuid(&simulation_id)?;

        let mut sims = self.simulations.write().await;
        if sims.len() >= MAX_OPEN_SIMULATIONS {
            return Err(Status::resource_exhausted("server simulation registry is full"));
        }
        sims.insert(id, simulation);

        Ok(Response::new(proto::SimulateCreateResponse {
            simulation_id,
        }))
    }

    async fn simulate_execute(
        &self,
        request: Request<proto::SimulateExecuteRequest>,
    ) -> Result<Response<proto::SimulateExecuteResponse>, Status> {
        let req = request.into_inner();
        let sim_uuid = parse_uuid(&req.simulation_id)?;

        let sim = {
            let sims = self.simulations.read().await;
            sims.get(&sim_uuid).cloned()
        }
        .ok_or_else(|| Status::not_found("simulation not found"))?;

        let ir = parse_ir(&req.ir_json)?;

        match &ir.operation {
            Operation::Assert(_) | Operation::Resolve(_) | Operation::Derive(_) => {}
            Operation::Simulate(_) => {
                return Err(invalid_argument("nested simulate not supported via transport"));
            }
            Operation::Monitor(_) => {
                return Err(invalid_argument("monitor not supported inside simulation via transport"));
            }
            Operation::Retract(_) | Operation::DefinePattern(_) => {
                return Err(invalid_argument("operation not supported inside simulation"));
            }
        }

        let response = match ir.operation {
            Operation::Assert(_) => {
                let belief = build_hypothetical_belief(&ir)?;
                sim.assert_hypothetical(belief)
                    .map_err(status_from_kyro_error)
                    .map(|belief_id| TransportResponse::Assert {
                        belief_id,
                        conflict_ids: Vec::new(),
                    })?
            }
            Operation::Resolve(_) => {
                let frame = sim.resolve_ir(ir).map_err(status_from_kyro_error)?;
                TransportResponse::Resolve { frame }
            }
            Operation::Derive(_) => {
                let derivation_id = sim.derive_ir(ir).map_err(status_from_kyro_error)?;
                TransportResponse::Derive { derivation_id }
            }
            _ => {
                return Err(Status::invalid_argument("operation not supported"));
            }
        };

        let response_json = encode_json(&response, MAX_RESPONSE_JSON_BYTES)?;
        Ok(Response::new(proto::SimulateExecuteResponse { response_json }))
    }

    async fn simulate_impact(
        &self,
        request: Request<proto::SimulateImpactRequest>,
    ) -> Result<Response<proto::SimulateImpactResponse>, Status> {
        let req = request.into_inner();
        let sim_uuid = parse_uuid(&req.simulation_id)?;

        let sim = {
            let sims = self.simulations.read().await;
            sims.get(&sim_uuid).cloned()
        }
        .ok_or_else(|| Status::not_found("simulation not found"))?;

        let impact: SimulationImpact = sim.query_impact().map_err(status_from_kyro_error)?;
        let impact_json = encode_json(&impact, MAX_RESPONSE_JSON_BYTES)?;
        Ok(Response::new(proto::SimulateImpactResponse { impact_json }))
    }

    async fn simulate_commit(
        &self,
        request: Request<proto::SimulateCommitRequest>,
    ) -> Result<Response<proto::SimulateCommitResponse>, Status> {
        let req = request.into_inner();
        let sim_uuid = parse_uuid(&req.simulation_id)?;
        let mode = parse_consistency_mode(&req.consistency_mode)?;

        let sim = {
            let sims = self.simulations.read().await;
            sims.get(&sim_uuid).cloned()
        }
        .ok_or_else(|| Status::not_found("simulation not found"))?;

        let result: SimulationCommitResult = sim
            .commit_overlay(&self.engine, mode)
            .map_err(status_from_kyro_error)?;

        let commit_json = encode_json(&result, MAX_RESPONSE_JSON_BYTES)?;
        Ok(Response::new(proto::SimulateCommitResponse { commit_json }))
    }

    async fn simulate_close(
        &self,
        request: Request<proto::SimulateCloseRequest>,
    ) -> Result<Response<proto::SimulateCloseResponse>, Status> {
        let req = request.into_inner();
        let sim_uuid = parse_uuid(&req.simulation_id)?;

        let closed = self.simulations.write().await.remove(&sim_uuid).is_some();
        Ok(Response::new(proto::SimulateCloseResponse { closed }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;
    use tonic::Request;

    use crate::confidence::Confidence;
    use crate::entity::{Entity, EntityType};
    use crate::ir::{AssertPayload, SimulatePayload};
    use crate::source::Source;
    use crate::storage::InMemoryStores;
    use crate::time::TimeRange;
    use crate::value::Value;

    fn make_engine() -> Arc<KyroEngine> {
        let stores = InMemoryStores::default();
        Arc::new(KyroEngine::new(
            Arc::new(stores.entities),
            Arc::new(stores.beliefs),
            Arc::new(stores.patterns),
            Arc::new(stores.conflicts),
            Arc::new(stores.derivations),
        ))
    }

    fn make_entity(engine: &KyroEngine) -> crate::EntityId {
        let entity = Entity::new("e", EntityType::Concept);
        let id = entity.id;
        engine
            .entity_store()
            .insert(entity)
            .expect("insert entity");
        id
    }

    fn make_assert_ir(entity_id: crate::EntityId) -> KyroIR {
        KyroIR {
            version: KyroIR::CURRENT_VERSION.to_string(),
            request_id: uuid::Uuid::new_v4(),
            timestamp: Utc::now(),
            operation: Operation::Assert(AssertPayload {
                entity_id,
                predicate: "p".to_string(),
                value: Value::Bool(true),
                confidence: Confidence::from_agent(0.9, "agent").unwrap(),
                source: Source::agent("agent", None::<String>),
                valid_time: TimeRange::from_now(),
                consistency_mode: crate::ir::ConsistencyMode::default(),
                embedding: None,
            }),
        }
    }

    #[tokio::test]
    async fn execute_returns_json_response() {
        let engine = make_engine();
        let entity_id = make_entity(&engine);
        let ir = make_assert_ir(entity_id);

        let svc = KyroServiceImpl::new(engine);
        let req = proto::ExecuteRequest {
            ir_json: serde_json::to_vec(&ir).unwrap(),
        };

        let resp = svc.execute(Request::new(req)).await.unwrap().into_inner();
        let v: serde_json::Value = serde_json::from_slice(&resp.response_json).unwrap();
        assert_eq!(v["type"], "assert");
        assert!(v.get("belief_id").is_some());
    }

    #[tokio::test]
    async fn simulate_create_execute_and_impact_work() {
        let engine = make_engine();
        let entity_id = make_entity(&engine);

        let svc = KyroServiceImpl::new(engine);

        let sim_ir = KyroIR {
            version: KyroIR::CURRENT_VERSION.to_string(),
            request_id: uuid::Uuid::new_v4(),
            timestamp: Utc::now(),
            operation: Operation::Simulate(SimulatePayload::default()),
        };

        let sim_resp = svc
            .simulate_create(Request::new(proto::SimulateCreateRequest {
                ir_json: serde_json::to_vec(&sim_ir).unwrap(),
            }))
            .await
            .unwrap()
            .into_inner();

        let assert_ir = make_assert_ir(entity_id);
        let exec_resp = svc
            .simulate_execute(Request::new(proto::SimulateExecuteRequest {
                simulation_id: sim_resp.simulation_id.clone(),
                ir_json: serde_json::to_vec(&assert_ir).unwrap(),
            }))
            .await
            .unwrap()
            .into_inner();

        let v: serde_json::Value = serde_json::from_slice(&exec_resp.response_json).unwrap();
        assert_eq!(v["type"], "assert");

        let impact_resp = svc
            .simulate_impact(Request::new(proto::SimulateImpactRequest {
                simulation_id: sim_resp.simulation_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        let impact: serde_json::Value = serde_json::from_slice(&impact_resp.impact_json).unwrap();
        assert_eq!(impact["inserted_beliefs"], 1);
    }
}

pub use proto::kyro_service_client::KyroServiceClient;

