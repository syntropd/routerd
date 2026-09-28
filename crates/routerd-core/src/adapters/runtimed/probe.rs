use super::RuntimedAdapter;
use crate::error::Result;
use serde_json::json;

impl RuntimedAdapter {
    pub(super) async fn check_health(&self) -> Result<bool> {
        if !self.socket_path.exists() {
            return Ok(false);
        }
        match self.call("io.syntrop.Runtime1.GetLoad", json!({})).await {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    pub(super) async fn available_models(&self) -> Result<Vec<String>> {
        let mut models = Vec::new();
        if self.socket_path.exists() {
            if let Ok(parameters) = self
                .call("io.syntrop.Runtime1.ListLoadedModels", json!({}))
                .await
            {
                if let Some(list) = parameters.get("models").and_then(|m| m.as_array()) {
                    for item in list {
                        if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                            models.push(name.to_string());
                        }
                    }
                }
            }
        }
        if models.is_empty() {
            Ok(self.configured_models.clone())
        } else {
            Ok(models)
        }
    }
}
