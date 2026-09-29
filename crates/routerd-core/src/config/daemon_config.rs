use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_listen_tcp")]
    pub listen_tcp: String,
    #[serde(default = "default_listen_unix", alias = "listen_socket")]
    pub listen_unix: String,
    #[serde(default = "default_varlink_socket")]
    pub varlink_socket: String,
    #[serde(default = "default_inferenced_socket")]
    pub inferenced_socket: String,
    #[serde(default = "default_runtimed_socket")]
    pub runtimed_socket: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn default_listen_tcp() -> String {
    "127.0.0.1:32768".to_string()
}
fn default_listen_unix() -> String {
    "/run/syntrop/router.sock".to_string()
}
fn default_varlink_socket() -> String {
    "/run/syntrop/io.syntrop.Router1".to_string()
}
fn default_inferenced_socket() -> String {
    "/run/syntrop/io.syntrop.Inference1".to_string()
}
fn default_runtimed_socket() -> String {
    std::env::var("SYNTROP_RUNTIME_SOCKET")
        .unwrap_or_else(|_| "/run/syntrop/io.syntrop.Runtime1".to_string())
}
fn default_log_level() -> String {
    "info".to_string()
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            listen_tcp: default_listen_tcp(),
            listen_unix: default_listen_unix(),
            varlink_socket: default_varlink_socket(),
            inferenced_socket: default_inferenced_socket(),
            runtimed_socket: default_runtimed_socket(),
            log_level: default_log_level(),
        }
    }
}
