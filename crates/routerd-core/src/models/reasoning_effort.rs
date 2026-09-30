//! Reasoning effort tier for guiding model thinking token budgets.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Reasoning effort tier requested by the client or adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    #[serde(alias = "off", alias = "0", alias = "disabled", alias = "false")]
    None,
    #[serde(alias = "1")]
    Low,
    #[serde(alias = "med", alias = "2")]
    Medium,
    #[serde(alias = "3")]
    High,
    #[serde(alias = "unlimited", alias = "full")]
    Max,
}

impl ReasoningEffort {
    /// Return the canonical string name for the effort level.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }

    /// Convert effort level to token budget given model context limit.
    pub fn to_budget(self, context_limit: usize) -> Option<usize> {
        match self {
            Self::None => Some(0),
            Self::Low => Some(1024.min(context_limit / 4)),
            Self::Medium => Some(4096.min(context_limit / 3)),
            Self::High => Some(16384.min(context_limit / 2)),
            Self::Max => None,
        }
    }
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for ReasoningEffort {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" | "off" | "0" | "disabled" | "false" => Ok(Self::None),
            "low" | "1" => Ok(Self::Low),
            "medium" | "med" | "2" => Ok(Self::Medium),
            "high" | "3" => Ok(Self::High),
            "max" | "unlimited" | "full" => Ok(Self::Max),
            other => Err(format!("unknown reasoning effort: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reasoning_effort_from_str() {
        assert_eq!("none".parse::<ReasoningEffort>().unwrap(), ReasoningEffort::None);
        assert_eq!("low".parse::<ReasoningEffort>().unwrap(), ReasoningEffort::Low);
        assert_eq!("med".parse::<ReasoningEffort>().unwrap(), ReasoningEffort::Medium);
        assert_eq!("high".parse::<ReasoningEffort>().unwrap(), ReasoningEffort::High);
        assert_eq!("max".parse::<ReasoningEffort>().unwrap(), ReasoningEffort::Max);
        assert!("invalid".parse::<ReasoningEffort>().is_err());
    }

    #[test]
    fn test_reasoning_effort_serde() {
        let de: ReasoningEffort = serde_json::from_str("\"low\"").unwrap();
        assert_eq!(de, ReasoningEffort::Low);
        let ser = serde_json::to_string(&de).unwrap();
        assert_eq!(ser, "\"low\"");
    }

    #[test]
    fn test_reasoning_effort_budget() {
        assert_eq!(ReasoningEffort::None.to_budget(8192), Some(0));
        assert_eq!(ReasoningEffort::Low.to_budget(8192), Some(1024));
        assert_eq!(ReasoningEffort::Medium.to_budget(8192), Some(2730));
        assert_eq!(ReasoningEffort::Medium.to_budget(32768), Some(4096));
        assert_eq!(ReasoningEffort::High.to_budget(8192), Some(4096));
        assert_eq!(ReasoningEffort::High.to_budget(65536), Some(16384));
        assert_eq!(ReasoningEffort::Max.to_budget(8192), None);
    }
}
