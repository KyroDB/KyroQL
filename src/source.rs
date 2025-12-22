//! Source and provenance types.
//!
//! Every belief in KyroQL must have a source—knowing where information
//! comes from is critical for trust, debugging, and audit trails.

use serde::{Deserialize, Serialize};

use crate::confidence::BeliefId;

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
    Paper {
        #[serde(skip_serializing_if = "Option::is_none")]
        arxiv_id: Option<String>,
        
        #[serde(skip_serializing_if = "Option::is_none")]
        doi: Option<String>,
        
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        
        #[serde(default)]
        authors: Vec<String>,
    },

    Sensor {
        sensor_id: String,
        
        #[serde(skip_serializing_if = "Option::is_none")]
        sensor_type: Option<String>,
        
        #[serde(skip_serializing_if = "Option::is_none")]
        calibration_date: Option<chrono::DateTime<chrono::Utc>>,
    },

    Agent {
        agent_id: String,
        
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_type: Option<String>,
        
        #[serde(skip_serializing_if = "Option::is_none")]
        model_version: Option<String>,
    },

    Human {
        user_id: String,
        
        #[serde(skip_serializing_if = "Option::is_none")]
        role: Option<String>,
    },

    Api {
        service_name: String,
        
        #[serde(skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
        
        #[serde(skip_serializing_if = "Option::is_none")]
        version: Option<String>,
    },

    Derived {
        premise_ids: Vec<BeliefId>,
        derivation_rule: String,
    },

    Unknown {
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

impl Source {
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
}
