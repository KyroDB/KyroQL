//! KyroQL gRPC Server
//!
//! A standalone server binary for running KyroQL over gRPC.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::signal;
use tonic::transport::Server;

use kyroql::engine::KyroEngine;
use kyroql::storage::open_database;
use kyroql::storage::PersistentStores;
use kyroql::transport::KyroServiceImpl;
use kyroql::{
    Belief, BeliefId, BeliefStore, Conflict, ConflictId, ConflictStore, DerivationId,
    DerivationRecord, DerivationStore, Entity, EntityId, EntityStore, Pattern, PatternId,
    PatternStore, StorageError, TimeRange,
};
use chrono::{DateTime, Utc};

/// Server configuration
struct Config {
    /// Address to bind to
    addr: SocketAddr,
    /// Data directory for persistent storage
    data_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1:50051".parse().unwrap(),
            data_dir: PathBuf::from("./brain.kyro"),
        }
    }
}

fn parse_args() -> Config {
    let args: Vec<String> = std::env::args().collect();
    let mut config = Config::default();
    
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                if i + 1 < args.len() {
                    let port: u16 = args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!("error: invalid port number: {}", args[i + 1]);
                        std::process::exit(1);
                    });
                    config.addr.set_port(port);
                    i += 2;
                } else {
                    eprintln!("error: --port requires a value");
                    std::process::exit(1);
                }
            }
            "--data-dir" | "-d" => {
                if i + 1 < args.len() {
                    config.data_dir = PathBuf::from(&args[i + 1]);
                    i += 2;
                } else {
                    eprintln!("error: --data-dir requires a value");
                    std::process::exit(1);
                }
            }
            "--help" | "-h" => {
                println!("kyroql-server - KyroQL gRPC Server");
                println!();
                println!("USAGE:");
                println!("    kyroql-server [OPTIONS]");
                println!();
                println!("OPTIONS:");
                println!("    -p, --port <PORT>         Port to listen on [default: 50051]");
                println!("    -d, --data-dir <DIR>      Data directory [default: ./brain.kyro]");
                println!("    -h, --help                Print help information");
                std::process::exit(0);
            }
            arg => {
                eprintln!("error: unknown argument: {}", arg);
                std::process::exit(1);
            }
        }
    }
    
    config
}

struct EntityStoreProxy {
    stores: Arc<PersistentStores>,
}

impl EntityStore for EntityStoreProxy {
    fn insert(&self, entity: Entity) -> Result<(), StorageError> {
        self.stores.entities.insert(entity)
    }

    fn get(&self, id: EntityId) -> Result<Option<Entity>, StorageError> {
        self.stores.entities.get(id)
    }

    fn update(&self, entity: Entity) -> Result<(), StorageError> {
        self.stores.entities.update(entity)
    }

    fn delete(&self, id: EntityId) -> Result<(), StorageError> {
        self.stores.entities.delete(id)
    }

    fn find_by_name(&self, name: &str) -> Result<Vec<Entity>, StorageError> {
        self.stores.entities.find_by_name(name)
    }

    fn find_by_name_fuzzy(&self, query: &str, limit: usize) -> Result<Vec<Entity>, StorageError> {
        self.stores.entities.find_by_name_fuzzy(query, limit)
    }

    fn find_by_embedding(
        &self,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<(Entity, f32)>, StorageError> {
        self.stores.entities.find_by_embedding(embedding, limit)
    }

    fn merge(&self, primary: EntityId, secondary: EntityId) -> Result<Entity, StorageError> {
        self.stores.entities.merge(primary, secondary)
    }

    fn get_at_version(&self, id: EntityId, version: u64) -> Result<Option<Entity>, StorageError> {
        self.stores.entities.get_at_version(id, version)
    }

    fn list_versions(&self, id: EntityId) -> Result<Vec<Entity>, StorageError> {
        self.stores.entities.list_versions(id)
    }
}

struct BeliefStoreProxy {
    stores: Arc<PersistentStores>,
}

impl BeliefStore for BeliefStoreProxy {
    fn insert(&self, belief: Belief) -> Result<(), StorageError> {
        self.stores.beliefs.insert(belief)
    }

    fn get(&self, id: BeliefId) -> Result<Option<Belief>, StorageError> {
        self.stores.beliefs.get(id)
    }

    fn supersede(&self, old_id: BeliefId, new_id: BeliefId) -> Result<(), StorageError> {
        self.stores.beliefs.supersede(old_id, new_id)
    }

    fn find_by_entity_predicate(
        &self,
        entity_id: EntityId,
        predicate: &str,
    ) -> Result<Vec<Belief>, StorageError> {
        self.stores
            .beliefs
            .find_by_entity_predicate(entity_id, predicate)
    }

    fn find_as_of(
        &self,
        entity_id: EntityId,
        predicate: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Vec<Belief>, StorageError> {
        self.stores.beliefs.find_as_of(entity_id, predicate, as_of)
    }

    fn find_by_time_range(&self, range: &TimeRange) -> Result<Vec<Belief>, StorageError> {
        self.stores.beliefs.find_by_time_range(range)
    }

    fn find_by_embedding(
        &self,
        embedding: &[f32],
        limit: usize,
        min_confidence: Option<f32>,
    ) -> Result<Vec<(Belief, f32)>, StorageError> {
        self.stores
            .beliefs
            .find_by_embedding(embedding, limit, min_confidence)
    }

    fn count_by_entity(&self, entity_id: EntityId) -> Result<usize, StorageError> {
        self.stores.beliefs.count_by_entity(entity_id)
    }
}

struct PatternStoreProxy {
    stores: Arc<PersistentStores>,
}

impl PatternStore for PatternStoreProxy {
    fn insert(&self, pattern: Pattern) -> Result<(), StorageError> {
        self.stores.patterns.insert(pattern)
    }

    fn get(&self, id: PatternId) -> Result<Option<Pattern>, StorageError> {
        self.stores.patterns.get(id)
    }

    fn update(&self, pattern: Pattern) -> Result<(), StorageError> {
        self.stores.patterns.update(pattern)
    }

    fn delete(&self, id: PatternId) -> Result<(), StorageError> {
        self.stores.patterns.delete(id)
    }

    fn find_by_predicate(&self, predicate: &str) -> Result<Vec<Pattern>, StorageError> {
        self.stores.patterns.find_by_predicate(predicate)
    }

    fn find_active(&self) -> Result<Vec<Pattern>, StorageError> {
        self.stores.patterns.find_active()
    }
}

struct ConflictStoreProxy {
    stores: Arc<PersistentStores>,
}

impl ConflictStore for ConflictStoreProxy {
    fn insert(&self, conflict: Conflict) -> Result<(), StorageError> {
        self.stores.conflicts.insert(conflict)
    }

    fn get(&self, id: ConflictId) -> Result<Option<Conflict>, StorageError> {
        self.stores.conflicts.get(id)
    }

    fn update(&self, conflict: Conflict) -> Result<(), StorageError> {
        self.stores.conflicts.update(conflict)
    }

    fn find_by_belief(&self, belief_id: BeliefId) -> Result<Vec<Conflict>, StorageError> {
        self.stores.conflicts.find_by_belief(belief_id)
    }

    fn find_open(&self) -> Result<Vec<Conflict>, StorageError> {
        self.stores.conflicts.find_open()
    }
}

struct DerivationStoreProxy {
    stores: Arc<PersistentStores>,
}

impl DerivationStore for DerivationStoreProxy {
    fn insert(&self, record: DerivationRecord) -> Result<(), StorageError> {
        self.stores.derivations.insert(record)
    }

    fn get(&self, id: DerivationId) -> Result<Option<DerivationRecord>, StorageError> {
        self.stores.derivations.get(id)
    }

    fn find_by_premise(&self, premise_id: BeliefId) -> Result<Vec<DerivationRecord>, StorageError> {
        self.stores.derivations.find_by_premise(premise_id)
    }

    fn find_by_derived_belief(
        &self,
        derived_belief_id: BeliefId,
    ) -> Result<Vec<DerivationRecord>, StorageError> {
        self.stores.derivations.find_by_derived_belief(derived_belief_id)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = parse_args();
    
    println!("KyroQL Server v{}", env!("CARGO_PKG_VERSION"));
    println!("Opening database at: {}", config.data_dir.display());
    
    // Open persistent storage (holds an exclusive lock for the process lifetime).
    let stores = Arc::new(open_database(&config.data_dir, None)?);

    let entities: Arc<dyn EntityStore> = Arc::new(EntityStoreProxy {
        stores: Arc::clone(&stores),
    });
    let beliefs: Arc<dyn BeliefStore> = Arc::new(BeliefStoreProxy {
        stores: Arc::clone(&stores),
    });
    let patterns: Arc<dyn PatternStore> = Arc::new(PatternStoreProxy {
        stores: Arc::clone(&stores),
    });
    let conflicts: Arc<dyn ConflictStore> = Arc::new(ConflictStoreProxy {
        stores: Arc::clone(&stores),
    });
    let derivations: Arc<dyn DerivationStore> = Arc::new(DerivationStoreProxy {
        stores: Arc::clone(&stores),
    });

    let engine = Arc::new(KyroEngine::new(
        entities,
        beliefs,
        patterns,
        conflicts,
        derivations,
    ));

    let svc = KyroServiceImpl::new(engine).into_server();

    println!("Database opened successfully");
    println!("Starting gRPC server on {}", config.addr);
    println!("Press Ctrl+C to stop");

    Server::builder()
        .add_service(svc)
        .serve_with_shutdown(config.addr, async {
            let _ = signal::ctrl_c().await;
        })
        .await?;

    println!("Shut down");
    Ok(())
}
