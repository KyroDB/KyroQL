//! IR serialization helpers.
//!
//! Serde already provides JSON (and other) serialization. This module
//! centralizes convenience helpers used by clients/servers and keeps
//! formatting stable.

use crate::error::KyroError;
use crate::ir::operations::KyroIR;

/// Serialize an IR to pretty JSON.
pub fn to_json_pretty(ir: &KyroIR) -> Result<String, KyroError> {
    serde_json::to_string_pretty(ir).map_err(|e| KyroError::internal(format!("serialize IR: {e}")))
}

/// Deserialize an IR from JSON.
///
/// Callers should then invoke `ir.validate()` before executing.
pub fn from_json(s: &str) -> Result<KyroIR, KyroError> {
    serde_json::from_str::<KyroIR>(s).map_err(|e| KyroError::internal(format!("deserialize IR: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confidence::Confidence;
    use crate::entity::EntityId;
    use crate::ir::operations::{AssertPayload, Operation};
    use crate::source::Source;
    use crate::time::TimeRange;
    use crate::value::Value;

    #[test]
    fn json_roundtrip_works() {
        let ir = KyroIR::new(Operation::Assert(AssertPayload {
            entity_id: EntityId::new(),
            predicate: "p".to_string(),
            value: Value::Bool(true),
            confidence: Confidence::from_agent(0.9, "a").unwrap(),
            source: Source::agent("a", None::<String>),
            valid_time: TimeRange::from_now(),
            consistency_mode: crate::ir::ConsistencyMode::Strict,
            embedding: Some(vec![0.1, 0.2]),
        }));

        let json = to_json_pretty(&ir).unwrap();
        let decoded = from_json(&json).unwrap();
        assert_eq!(ir, decoded);
    }
}
