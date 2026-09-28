use super::RouterctlClient;
use anyhow::Result;
use serde_json::{json, Value};

impl RouterctlClient {
    pub async fn get_status(&self) -> Result<Value> {
        if self.socket_path.exists() {
            if let Ok(val) = self.varlink_call("io.syntrop.Router1.GetStatus", json!({})).await {
                return Ok(val);
            }
        }

        // Fallback to HTTP health
        let url = format!("{}/health", self.http_url);
        let resp = self.http_client.get(&url).send().await?;
        let json_val = resp.json().await?;
        Ok(json_val)
    }

    pub async fn list_providers(&self) -> Result<Value> {
        self.varlink_call("io.syntrop.Router1.ListProviders", json!({})).await
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        if self.socket_path.exists() {
            if let Ok(val) = self.varlink_call("io.syntrop.Router1.ListModels", json!({})).await {
                if let Some(arr) = val.get("models").and_then(|m| m.as_array()) {
                    let mut list = Vec::new();
                    for item in arr {
                        if let Some(s) = item.as_str() {
                            list.push(s.to_string());
                        }
                    }
                    return Ok(list);
                }
            }
        }

        // Fallback to HTTP /v1/models
        let url = format!("{}/v1/models", self.http_url);
        let resp = self.http_client.get(&url).send().await?;
        let json_val: Value = resp.json().await?;
        let mut list = Vec::new();
        if let Some(arr) = json_val.get("data").and_then(|d| d.as_array()) {
            for item in arr {
                if let Some(id) = item.get("id").and_then(|i| i.as_str()) {
                    list.push(id.to_string());
                }
            }
        }
        Ok(list)
    }

    pub async fn route_request(
        &self,
        model: Option<&str>,
        tier: Option<&str>,
        tokens: Option<usize>,
        require_stream: bool,
    ) -> Result<Value> {
        let params = json!({
            "model": model,
            "tier": tier,
            "estimated_tokens": tokens,
            "require_stream": require_stream
        });
        self.varlink_call("io.syntrop.Router1.RouteRequest", params).await
    }

    pub async fn test_provider(&self, provider_id: &str) -> Result<Value> {
        let params = json!({
            "provider_id": provider_id
        });
        self.varlink_call("io.syntrop.Router1.TestProvider", params).await
    }

    pub async fn get_info(&self) -> Result<Value> {
        self.varlink_call("org.varlink.service.GetInfo", json!({})).await
    }

    pub async fn get_interface_description(&self, iface: &str) -> Result<String> {
        let val = self
            .varlink_call(
                "org.varlink.service.GetInterfaceDescription",
                json!({ "interface": iface }),
            )
            .await?;
        let desc = val
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_string();
        Ok(desc)
    }
}
