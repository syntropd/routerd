use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct VarlinkRequest {
    pub method: String,
    #[serde(default)]
    pub parameters: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VarlinkReply {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continues: Option<bool>,
}

impl VarlinkReply {
    fn to_bytes(&self) -> Vec<u8> {
        let mut b = serde_json::to_vec(self).unwrap();
        b.push(0);
        b
    }
}

#[test]
fn test_varlink_framing_and_nul_terminator() {
    let rep = VarlinkReply {
        parameters: Some(json!({ "status": "active" })),
        error: None,
        continues: Some(false),
    };
    let bytes = rep.to_bytes();
    assert_eq!(*bytes.last().unwrap(), 0u8);

    let clean = &bytes[..bytes.len() - 1];
    let parsed: VarlinkReply = serde_json::from_slice(clean).unwrap();
    assert_eq!(
        parsed.parameters.unwrap().get("status").unwrap(),
        "active"
    );
}

#[test]
fn test_varlink_error_frame() {
    let rep = VarlinkReply {
        parameters: Some(json!({ "parameter": "interface" })),
        error: Some("org.varlink.service.InvalidParameter".to_string()),
        continues: None,
    };
    let bytes = rep.to_bytes();
    assert_eq!(*bytes.last().unwrap(), 0u8);

    let parsed: VarlinkReply = serde_json::from_slice(&bytes[..bytes.len() - 1]).unwrap();
    assert_eq!(
        parsed.error.as_deref(),
        Some("org.varlink.service.InvalidParameter")
    );
}
