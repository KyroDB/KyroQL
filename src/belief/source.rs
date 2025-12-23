//! Source and provenance types.
//!
//! Every belief in KyroQL must have a source—knowing where information
//! comes from is critical for trust, debugging, and audit trails.

use serde::{Deserialize, Serialize};

use uuid::Uuid;

use crate::confidence::{BeliefId, SourceId};

const SOURCE_ID_NAMESPACE: Uuid = Uuid::from_bytes([
    0x5b, 0x1f, 0x67, 0x5e, 0x6d, 0x9f, 0x4b, 0x77, 0x8e, 0xf4, 0x4f, 0x8b, 0x9b, 0x2a, 0xa1, 0x1c,
]);

fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    push_u32(out, u32::try_from(bytes.len()).unwrap_or(u32::MAX));
    out.extend_from_slice(bytes);
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    push_bytes(out, s.as_bytes());
}

fn push_opt_str(out: &mut Vec<u8>, s: Option<&str>) {
    match s {
        Some(v) => {
            out.push(1);
            push_str(out, v);
        }
        None => out.push(0),
    }
}

fn push_vec_str(out: &mut Vec<u8>, values: &[String]) {
    push_u32(out, u32::try_from(values.len()).unwrap_or(u32::MAX));
    for v in values {
        push_str(out, v);
    }
}

fn push_opt_datetime(out: &mut Vec<u8>, dt: Option<&chrono::DateTime<chrono::Utc>>) {
    match dt {
        Some(v) => {
            out.push(1);
            out.extend_from_slice(&v.timestamp().to_le_bytes());
            out.extend_from_slice(&v.timestamp_subsec_nanos().to_le_bytes());
        }
        None => out.push(0),
    }
}

/// Source of a belief—where did this information come from?
///
/// Provenance is critical for AGI systems. It enables:
/// - Trust evaluation
/// - Conflict resolution
/// - Audit trails
/// - Belief retraction when sources are discredited
///
/// # Examples
///
/// ```
/// use kyroql::Source;
///
/// let paper_source = Source::paper("2307.12008", "LK-99 Initial Report");
/// let agent_source = Source::agent("gpt-4", Some("1.0"));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Source {
    /// Academic paper or publication.
    Paper {
        /// ArXiv identifier (e.g., "2307.12008").
        #[serde(skip_serializing_if = "Option::is_none")]
        arxiv_id: Option<String>,
        /// Digital Object Identifier.
        #[serde(skip_serializing_if = "Option::is_none")]
        doi: Option<String>,
        /// Paper title.
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// Author names.
        #[serde(default)]
        authors: Vec<String>,
    },

    /// Physical sensor or measurement device.
    Sensor {
        /// Unique sensor identifier.
        sensor_id: String,
        /// Type of sensor (e.g., "temperature", "pressure").
        #[serde(skip_serializing_if = "Option::is_none")]
        sensor_type: Option<String>,
        /// When the sensor was last calibrated.
        #[serde(skip_serializing_if = "Option::is_none")]
        calibration_date: Option<chrono::DateTime<chrono::Utc>>,
    },

    /// AI agent or model.
    Agent {
        /// Agent identifier.
        agent_id: String,
        /// Type (e.g., "llm", "classifier").
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_type: Option<String>,
        /// Model version.
        #[serde(skip_serializing_if = "Option::is_none")]
        model_version: Option<String>,
    },

    /// Human user.
    Human {
        /// User identifier.
        user_id: String,
        /// Role (e.g., "admin", "researcher").
        #[serde(skip_serializing_if = "Option::is_none")]
        role: Option<String>,
    },

    /// External API or service.
    Api {
        /// Service name.
        service_name: String,
        /// API endpoint.
        #[serde(skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
        /// API version.
        #[serde(skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },

    /// Derived from other beliefs via inference.
    Derived {
        /// Belief IDs used as premises.
        premise_ids: Vec<BeliefId>,
        /// Derivation rule applied.
        derivation_rule: String,
    },

    /// Unknown or unspecified source.
    Unknown {
        /// Optional description.
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

impl Source {
    fn stable_id_encoding(&self) -> Vec<u8> {
        let mut encoding = Vec::with_capacity(256);
        push_str(&mut encoding, "kyroql:source:v1");

        match self {
            Self::Paper {
                arxiv_id,
                doi,
                title,
                authors,
            } => {
                push_str(&mut encoding, "paper");
                push_opt_str(&mut encoding, arxiv_id.as_deref());
                push_opt_str(&mut encoding, doi.as_deref());
                push_opt_str(&mut encoding, title.as_deref());
                push_vec_str(&mut encoding, authors);
            }
            Self::Sensor {
                sensor_id,
                sensor_type,
                calibration_date,
            } => {
                push_str(&mut encoding, "sensor");
                push_str(&mut encoding, sensor_id);
                push_opt_str(&mut encoding, sensor_type.as_deref());
                push_opt_datetime(&mut encoding, calibration_date.as_ref());
            }
            Self::Agent {
                agent_id,
                agent_type,
                model_version,
            } => {
                push_str(&mut encoding, "agent");
                push_str(&mut encoding, agent_id);
                push_opt_str(&mut encoding, agent_type.as_deref());
                push_opt_str(&mut encoding, model_version.as_deref());
            }
            Self::Human { user_id, role } => {
                push_str(&mut encoding, "human");
                push_str(&mut encoding, user_id);
                push_opt_str(&mut encoding, role.as_deref());
            }
            Self::Api {
                service_name,
                endpoint,
                version,
            } => {
                push_str(&mut encoding, "api");
                push_str(&mut encoding, service_name);
                push_opt_str(&mut encoding, endpoint.as_deref());
                push_opt_str(&mut encoding, version.as_deref());
            }
            Self::Derived {
                premise_ids,
                derivation_rule,
            } => {
                push_str(&mut encoding, "derived");
                push_u32(&mut encoding, u32::try_from(premise_ids.len()).unwrap_or(u32::MAX));
                for id in premise_ids {
                    push_str(&mut encoding, &id.to_string());
                }
                push_str(&mut encoding, derivation_rule);
            }
            Self::Unknown { description } => {
                push_str(&mut encoding, "unknown");
                push_opt_str(&mut encoding, description.as_deref());
            }
        }

        encoding
    }

    /// Creates a paper source with ArXiv ID.
    #[must_use]
    pub fn paper(arxiv_id: impl Into<String>, title: impl Into<String>) -> Self {
        Self::Paper {
            arxiv_id: Some(arxiv_id.into()),
            doi: None,
            title: Some(title.into()),
            authors: Vec::new(),
        }
    }

    /// Creates a paper source with DOI.
    #[must_use]
    pub fn paper_doi(doi: impl Into<String>, title: impl Into<String>) -> Self {
        Self::Paper {
            arxiv_id: None,
            doi: Some(doi.into()),
            title: Some(title.into()),
            authors: Vec::new(),
        }
    }

    /// Creates a sensor source.
    #[must_use]
    pub fn sensor(sensor_id: impl Into<String>) -> Self {
        Self::Sensor {
            sensor_id: sensor_id.into(),
            sensor_type: None,
            calibration_date: None,
        }
    }

    /// Creates a sensor source with type.
    #[must_use]
    pub fn sensor_with_type(
        sensor_id: impl Into<String>,
        sensor_type: impl Into<String>,
    ) -> Self {
        Self::Sensor {
            sensor_id: sensor_id.into(),
            sensor_type: Some(sensor_type.into()),
            calibration_date: None,
        }
    }

    /// Creates an agent source.
    #[must_use]
    pub fn agent(agent_id: impl Into<String>, model_version: Option<impl Into<String>>) -> Self {
        Self::Agent {
            agent_id: agent_id.into(),
            agent_type: None,
            model_version: model_version.map(Into::into),
        }
    }

    /// Creates an agent source with type.
    #[must_use]
    pub fn agent_with_type(
        agent_id: impl Into<String>,
        agent_type: impl Into<String>,
        model_version: Option<impl Into<String>>,
    ) -> Self {
        Self::Agent {
            agent_id: agent_id.into(),
            agent_type: Some(agent_type.into()),
            model_version: model_version.map(Into::into),
        }
    }

    /// Creates a human source.
    #[must_use]
    pub fn human(user_id: impl Into<String>) -> Self {
        Self::Human {
            user_id: user_id.into(),
            role: None,
        }
    }

    /// Creates a human source with role.
    #[must_use]
    pub fn human_with_role(user_id: impl Into<String>, role: impl Into<String>) -> Self {
        Self::Human {
            user_id: user_id.into(),
            role: Some(role.into()),
        }
    }

    /// Creates an API source.
    #[must_use]
    pub fn api(service_name: impl Into<String>) -> Self {
        Self::Api {
            service_name: service_name.into(),
            endpoint: None,
            version: None,
        }
    }

    /// Creates a derived source.
    #[must_use]
    pub fn derived(premise_ids: Vec<BeliefId>, derivation_rule: impl Into<String>) -> Self {
        Self::Derived {
            premise_ids,
            derivation_rule: derivation_rule.into(),
        }
    }

    /// Creates an unknown source.
    #[must_use]
    pub fn unknown() -> Self {
        Self::Unknown { description: None }
    }

    /// Creates an unknown source with description.
    #[must_use]
    pub fn unknown_with_description(description: impl Into<String>) -> Self {
        Self::Unknown {
            description: Some(description.into()),
        }
    }

    /// Returns a human-readable source type.
    #[must_use]
    pub const fn source_type(&self) -> &'static str {
        match self {
            Self::Paper { .. } => "paper",
            Self::Sensor { .. } => "sensor",
            Self::Agent { .. } => "agent",
            Self::Human { .. } => "human",
            Self::Api { .. } => "api",
            Self::Derived { .. } => "derived",
            Self::Unknown { .. } => "unknown",
        }
    }

    /// Returns true if this is a human source.
    #[must_use]
    pub const fn is_human(&self) -> bool {
        matches!(self, Self::Human { .. })
    }

    /// Returns true if this is an automated source (agent, sensor, or API).
    #[must_use]
    pub const fn is_automated(&self) -> bool {
        matches!(self, Self::Agent { .. } | Self::Sensor { .. } | Self::Api { .. })
    }

    /// Returns true if this is a derived source.
    #[must_use]
    pub const fn is_derived(&self) -> bool {
        matches!(self, Self::Derived { .. })
    }

    /// Returns a unique identifier for this source.
    ///
    /// Computes a deterministic content-based identifier for this source.
    #[must_use]
    pub fn source_id(&self) -> SourceId {
        let encoding = self.stable_id_encoding();
        let digest = blake3::hash(&encoding);
        let uuid = Uuid::new_v5(&SOURCE_ID_NAMESPACE, digest.as_bytes());
        SourceId::from_uuid(uuid)
    }
}

impl Default for Source {
    fn default() -> Self {
        Self::unknown()
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Paper {
                arxiv_id, doi, title, ..
            } => {
                if let Some(id) = arxiv_id {
                    write!(f, "paper:arXiv:{id}")
                } else if let Some(d) = doi {
                    write!(f, "paper:doi:{d}")
                } else if let Some(t) = title {
                    write!(f, "paper:\"{t}\"")
                } else {
                    write!(f, "paper:unknown")
                }
            }
            Self::Sensor { sensor_id, .. } => write!(f, "sensor:{sensor_id}"),
            Self::Agent { agent_id, .. } => write!(f, "agent:{agent_id}"),
            Self::Human { user_id, .. } => write!(f, "human:{user_id}"),
            Self::Api { service_name, .. } => write!(f, "api:{service_name}"),
            Self::Derived {
                derivation_rule, ..
            } => write!(f, "derived:{derivation_rule}"),
            Self::Unknown { description } => {
                if let Some(desc) = description {
                    write!(f, "unknown:{desc}")
                } else {
                    write!(f, "unknown")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_paper() {
        let source = Source::paper("2307.12008", "LK-99 Report");
        assert_eq!(source.source_type(), "paper");
        assert!(!source.is_human());
        assert!(!source.is_automated());

        if let Source::Paper {
            arxiv_id, title, ..
        } = &source
        {
            assert_eq!(arxiv_id.as_deref(), Some("2307.12008"));
            assert_eq!(title.as_deref(), Some("LK-99 Report"));
        } else {
            panic!("Expected Paper source");
        }
    }

    #[test]
    fn test_source_paper_doi() {
        let source = Source::paper_doi("10.1234/test", "Test Paper");
        if let Source::Paper { doi, arxiv_id, .. } = &source {
            assert_eq!(doi.as_deref(), Some("10.1234/test"));
            assert!(arxiv_id.is_none());
        } else {
            panic!("Expected Paper source");
        }
    }

    #[test]
    fn test_source_sensor() {
        let source = Source::sensor("temp-01");
        assert_eq!(source.source_type(), "sensor");
        assert!(source.is_automated());

        if let Source::Sensor { sensor_id, .. } = &source {
            assert_eq!(sensor_id, "temp-01");
        } else {
            panic!("Expected Sensor source");
        }
    }

    #[test]
    fn test_source_sensor_with_type() {
        let source = Source::sensor_with_type("temp-01", "temperature");
        if let Source::Sensor {
            sensor_id,
            sensor_type,
            ..
        } = &source
        {
            assert_eq!(sensor_id, "temp-01");
            assert_eq!(sensor_type.as_deref(), Some("temperature"));
        } else {
            panic!("Expected Sensor source");
        }
    }

    #[test]
    fn test_source_agent() {
        let source = Source::agent("gpt-4", Some("2024-01"));
        assert_eq!(source.source_type(), "agent");
        assert!(source.is_automated());

        if let Source::Agent {
            agent_id,
            model_version,
            ..
        } = &source
        {
            assert_eq!(agent_id, "gpt-4");
            assert_eq!(model_version.as_deref(), Some("2024-01"));
        } else {
            panic!("Expected Agent source");
        }
    }

    #[test]
    fn test_source_human() {
        let source = Source::human("user-123");
        assert_eq!(source.source_type(), "human");
        assert!(source.is_human());
        assert!(!source.is_automated());

        if let Source::Human { user_id, .. } = &source {
            assert_eq!(user_id, "user-123");
        } else {
            panic!("Expected Human source");
        }
    }

    #[test]
    fn test_source_human_with_role() {
        let source = Source::human_with_role("user-123", "admin");
        if let Source::Human { user_id, role } = &source {
            assert_eq!(user_id, "user-123");
            assert_eq!(role.as_deref(), Some("admin"));
        } else {
            panic!("Expected Human source");
        }
    }

    #[test]
    fn test_source_api() {
        let source = Source::api("weather-service");
        assert_eq!(source.source_type(), "api");
        assert!(source.is_automated());
    }

    #[test]
    fn test_source_derived() {
        let premises = vec![BeliefId::new(), BeliefId::new()];
        let source = Source::derived(premises.clone(), "modus_ponens");
        assert_eq!(source.source_type(), "derived");
        assert!(source.is_derived());

        if let Source::Derived {
            premise_ids,
            derivation_rule,
        } = &source
        {
            assert_eq!(premise_ids.len(), 2);
            assert_eq!(derivation_rule, "modus_ponens");
        } else {
            panic!("Expected Derived source");
        }
    }

    #[test]
    fn test_source_unknown() {
        let source = Source::unknown();
        assert_eq!(source.source_type(), "unknown");

        let source_with_desc = Source::unknown_with_description("legacy data");
        if let Source::Unknown { description } = &source_with_desc {
            assert_eq!(description.as_deref(), Some("legacy data"));
        } else {
            panic!("Expected Unknown source");
        }
    }

    #[test]
    fn test_source_display() {
        assert!(format!("{}", Source::paper("123", "Test")).contains("arXiv:123"));
        assert!(format!("{}", Source::sensor("s1")).contains("sensor:s1"));
        assert!(format!("{}", Source::agent("a1", None::<String>)).contains("agent:a1"));
        assert!(format!("{}", Source::human("u1")).contains("human:u1"));
        assert!(format!("{}", Source::api("api1")).contains("api:api1"));
        assert!(format!("{}", Source::unknown()).contains("unknown"));
    }

    #[test]
    fn test_source_serialization() {
        let source = Source::agent("test-agent", Some("v1"));
        let json = serde_json::to_string(&source).unwrap();
        let deserialized: Source = serde_json::from_str(&json).unwrap();
        assert_eq!(source, deserialized);
    }

    #[test]
    fn test_source_default() {
        let source = Source::default();
        assert_eq!(source.source_type(), "unknown");
    }

    #[test]
    fn test_source_id_is_deterministic() {
        let source = Source::agent("gpt-4", Some("2024-01"));
        assert_eq!(source.source_id(), source.source_id());
    }

    #[test]
    fn test_source_id_paper_includes_arxiv_id() {
        let a = Source::paper("2307.12008", "LK-99");
        let b = Source::paper("2307.99999", "LK-99");
        assert_ne!(a.source_id(), b.source_id());
    }

    #[test]
    fn test_source_id_api_includes_service_name_even_without_endpoint() {
        let a = Source::api("service-a");
        let b = Source::api("service-b");
        assert_ne!(a.source_id(), b.source_id());
    }

    #[test]
    fn test_source_id_is_uuid_v5() {
        let source = Source::api("weather-service");
        let id = source.source_id();

        let raw: Uuid = serde_json::from_value(serde_json::to_value(id).unwrap()).unwrap();
        assert_eq!(raw.get_version_num(), 5);
    }
}
