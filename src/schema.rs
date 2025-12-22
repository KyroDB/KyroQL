use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// The fundamental atom of KyroDB.
/// It is not just data; it is a claim about reality with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Belief {
    /// Unique identifier for this specific fact/belief version
    pub id: Uuid,
    
    /// The entity this belief is about (e.g., "Superconductivity")
    pub entity: String,
    
    /// The attribute being described (e.g., "resistance_at_20c")
    pub attribute: String,
    
    /// The value of the attribute
    pub value: Value,
    
    /// EPISTEMIC METADATA
    /// How sure are we? 0.0 to 1.0
    pub confidence: f32,
    
    /// Who said this? Provenance is critical for AGI.
    pub source: Source,
    
    /// TEMPORAL DIMENSIONS (Bitemporal)
    /// When is this true in reality?
    pub valid_time: TimeRange,
    
    /// When did the system learn this?
    pub tx_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Value {
    Float(f64),
    Integer(i64),
    String(String),
    Boolean(bool),
    Vector(Vec<f32>),
    Json(String), // For complex nested structures
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub agent_id: String, // Who asserted this?
    pub origin: String,   // "arXiv:2307.12008", "Sensor:Temp_01"
    pub method: String,   // "Inference", "Observation", "User_Input"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>, // None means "forever" or "until superseded"
}

/// The result of a RESOLVE operation.
/// It returns a synthesized answer, not just raw rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveFrame {
    /// The synthesized answer, if one could be resolved
    pub answer: Option<Value>,
    
    /// The aggregate confidence of this answer
    pub confidence: f32,
    
    /// Any conflicts that were found during resolution
    pub conflicts: Vec<Conflict>,
    
    /// Missing information that prevented a full answer
    pub missing_data: Vec<String>,
    
    /// The trace of facts used to derive this answer
    pub trace: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub entity: String,
    pub attribute: String,
    pub conflicting_values: Vec<Value>,
    pub belief_ids: Vec<Uuid>,
    pub reason: String, // "Mutually exclusive values", "Pattern violation"
}
