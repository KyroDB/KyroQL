//! Value types that beliefs can hold.
//!
//! Values in KyroQL support multiple types including primitives,
//! entity references, embeddings, and structured JSON data.

use serde::{Deserialize, Serialize};

use crate::entity::EntityId;

/// Possible values a belief can hold.
///
/// This enum covers all the value types that can be associated with
/// a belief's predicate.
///
/// # Examples
///
/// ```
/// use kyroql::Value;
///
/// let bool_val = Value::Bool(true);
/// let float_val = Value::Float(3.14);
/// let string_val = Value::String("hello".to_string());
///
/// assert!(bool_val.is_bool());
/// assert!(float_val.is_float());
/// assert!(string_val.is_string());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Entity(EntityId),
    Embedding(Vec<f32>),
    Structured(serde_json::Value),
    Null,
}

impl Value {
    pub const fn is_bool(&self) -> bool {
        matches!(self, Self::Bool(_))
    }

    pub const fn is_int(&self) -> bool {
        matches!(self, Self::Int(_))
    }

    pub const fn is_float(&self) -> bool {
        matches!(self, Self::Float(_))
    }

    pub const fn is_string(&self) -> bool {
        matches!(self, Self::String(_))
    }

    pub const fn is_entity(&self) -> bool {
        matches!(self, Self::Entity(_))
    }

    pub const fn is_embedding(&self) -> bool {
        matches!(self, Self::Embedding(_))
    }

    pub const fn is_structured(&self) -> bool {
        matches!(self, Self::Structured(_))
    }

    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub const fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(v) => Some(*v),
            _ => None,
        }
    }

    pub const fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(v) => Some(*v),
            Self::Int(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<&str> {
        match self {
            Self::String(v) => Some(v),
            _ => None,
        }
    }

    pub const fn as_entity(&self) -> Option<EntityId> {
        match self {
            Self::Entity(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_embedding(&self) -> Option<&[f32]> {
        match self {
            Self::Embedding(v) => Some(v),
            _ => None,
        }
    }

    pub const fn as_structured(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Structured(v) => Some(v),
            _ => None,
        }
    }

    /// Returns a human-readable type name.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Int(_) => "int",
            Self::Float(_) => "float",
            Self::String(_) => "string",
            Self::Entity(_) => "entity",
            Self::Embedding(_) => "embedding",
            Self::Structured(_) => "structured",
            Self::Null => "null",
        }
    }
}

impl Default for Value {
    fn default() -> Self {
        Self::Null
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bool(v) => write!(f, "{v}"),
            Self::Int(v) => write!(f, "{v}"),
            Self::Float(v) => write!(f, "{v}"),
            Self::String(v) => write!(f, "{v:?}"),
            Self::Entity(v) => write!(f, "entity:{v}"),
            Self::Embedding(v) => write!(f, "embedding[{}]", v.len()),
            Self::Structured(v) => write!(f, "{v}"),
            Self::Null => write!(f, "null"),
        }
    }
}

// Convenient From implementations
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Self::Int(i64::from(v))
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Self::Float(f64::from(v))
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::Float(v)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::String(v.to_string())
    }
}

impl From<EntityId> for Value {
    fn from(v: EntityId) -> Self {
        Self::Entity(v)
    }
}

impl From<Vec<f32>> for Value {
    fn from(v: Vec<f32>) -> Self {
        Self::Embedding(v)
    }
}

impl From<serde_json::Value> for Value {
    fn from(v: serde_json::Value) -> Self {
        Self::Structured(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_bool() {
        let val = Value::Bool(true);
        assert!(val.is_bool());
        assert_eq!(val.as_bool(), Some(true));
        assert_eq!(val.type_name(), "bool");
    }

    #[test]
    fn test_value_int() {
        let val = Value::Int(42);
        assert!(val.is_int());
        assert_eq!(val.as_int(), Some(42));
        assert_eq!(val.as_float(), Some(42.0)); // Int can be read as float
        assert_eq!(val.type_name(), "int");
    }

    #[test]
    fn test_value_float() {
        let val = Value::Float(3.14);
        assert!(val.is_float());
        assert!((val.as_float().unwrap() - 3.14).abs() < f64::EPSILON);
        assert_eq!(val.type_name(), "float");
    }

    #[test]
    fn test_value_string() {
        let val = Value::String("hello".to_string());
        assert!(val.is_string());
        assert_eq!(val.as_string(), Some("hello"));
        assert_eq!(val.type_name(), "string");
    }

    #[test]
    fn test_value_entity() {
        let id = EntityId::new();
        let val = Value::Entity(id);
        assert!(val.is_entity());
        assert_eq!(val.as_entity(), Some(id));
        assert_eq!(val.type_name(), "entity");
    }

    #[test]
    fn test_value_embedding() {
        let embedding = vec![0.1, 0.2, 0.3];
        let val = Value::Embedding(embedding.clone());
        assert!(val.is_embedding());
        assert_eq!(val.as_embedding(), Some(embedding.as_slice()));
        assert_eq!(val.type_name(), "embedding");
    }

    #[test]
    fn test_value_structured() {
        let json = serde_json::json!({"key": "value"});
        let val = Value::Structured(json.clone());
        assert!(val.is_structured());
        assert_eq!(val.as_structured(), Some(&json));
        assert_eq!(val.type_name(), "structured");
    }

    #[test]
    fn test_value_null() {
        let val = Value::Null;
        assert!(val.is_null());
        assert_eq!(val.type_name(), "null");
    }

    #[test]
    fn test_value_display() {
        assert_eq!(format!("{}", Value::Bool(true)), "true");
        assert_eq!(format!("{}", Value::Int(42)), "42");
        assert_eq!(format!("{}", Value::String("hi".into())), "\"hi\"");
        assert_eq!(format!("{}", Value::Null), "null");
        assert_eq!(
            format!("{}", Value::Embedding(vec![0.1, 0.2, 0.3])),
            "embedding[3]"
        );
    }

    #[test]
    fn test_value_from_conversions() {
        let _: Value = true.into();
        let _: Value = 42i32.into();
        let _: Value = 42i64.into();
        let _: Value = 3.14f32.into();
        let _: Value = 3.14f64.into();
        let _: Value = "hello".into();
        let _: Value = String::from("hello").into();
        let _: Value = EntityId::new().into();
        let _: Value = vec![0.1f32, 0.2, 0.3].into();
    }

    #[test]
    fn test_value_serialization() {
        let val = Value::String("test".into());
        let json = serde_json::to_string(&val).unwrap();
        let deserialized: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val, deserialized);
    }

    #[test]
    fn test_value_type_mismatch() {
        let val = Value::Bool(true);
        assert!(val.as_int().is_none());
        assert!(val.as_float().is_none());
        assert!(val.as_string().is_none());
    }
}
