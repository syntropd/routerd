use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VarlinkRequest {
    pub method: String,
    #[serde(default)]
    pub parameters: Option<Value>,
    #[serde(default)]
    pub more: Option<bool>,
    #[serde(default)]
    pub oneway: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VarlinkReply {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continues: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl VarlinkReply {
    pub fn ok(parameters: Value) -> Self {
        Self {
            parameters: Some(parameters),
            continues: None,
            error: None,
        }
    }

    #[allow(dead_code)]
    pub fn streaming(parameters: Value, continues: bool) -> Self {
        Self {
            parameters: Some(parameters),
            continues: Some(continues),
            error: None,
        }
    }

    pub fn error(error: impl Into<String>, parameters: Value) -> Self {
        Self {
            parameters: Some(parameters),
            continues: None,
            error: Some(error.into()),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(self).unwrap_or_default();
        bytes.push(0); // NUL-terminated Varlink frame
        bytes
    }
}

pub fn parse_request(bytes: &[u8]) -> Result<VarlinkRequest, String> {
    let clean = if let Some(&0) = bytes.last() {
        &bytes[..bytes.len() - 1]
    } else {
        bytes
    };
    serde_json::from_slice(clean).map_err(|e| format!("Invalid Varlink JSON: {}", e))
}

pub fn make_method_not_found(method: &str) -> Vec<u8> {
    VarlinkReply::error(
        "org.varlink.service.MethodNotFound",
        serde_json::json!({ "method": method }),
    )
    .to_bytes()
}

#[allow(dead_code)]
pub fn make_invalid_parameter(param: &str) -> Vec<u8> {
    VarlinkReply::error(
        "org.varlink.service.InvalidParameter",
        serde_json::json!({ "parameter": param }),
    )
    .to_bytes()
}
