//! OpenAI-compatible tool and function calling definitions.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Definition of a callable function schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
}

/// Tool definition exposed to the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub r#type: String,
    pub function: FunctionDefinition,
}

impl ToolDefinition {
    pub fn function(name: impl Into<String>, desc: Option<String>, params: Option<Value>) -> Self {
        Self {
            r#type: "function".into(),
            function: FunctionDefinition {
                name: name.into(),
                description: desc,
                parameters: params,
            },
        }
    }
}

/// Invocation parameters of a function call requested by the model.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FunctionCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

/// Individual tool call request returned in assistant responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    pub function: FunctionCall,
}

impl ToolCall {
    pub fn function(id: impl Into<String>, name: impl Into<String>, args: impl Into<String>) -> Self {
        Self {
            index: None,
            id: Some(id.into()),
            r#type: Some("function".into()),
            function: FunctionCall {
                name: Some(name.into()),
                arguments: Some(args.into()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_definition_serialization() {
        let tool = ToolDefinition::function("unit_status", Some("probe".into()), Some(serde_json::json!({"type": "object"})));
        let json_tool = serde_json::to_string(&tool).unwrap();
        assert!(json_tool.contains("unit_status"));
        assert!(json_tool.contains("\"type\":\"function\""));
    }

    #[test]
    fn test_tool_call_serialization() {
        let call = ToolCall::function("call_1", "unit_status", "{\"unit\":\"dbus\"}");
        let json_call = serde_json::to_string(&call).unwrap();
        assert!(json_call.contains("call_1"));
        assert!(json_call.contains("unit_status"));
    }
}
