use thiserror::Error;

#[derive(Error, Debug)]
pub enum RouterError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Credential error: {0}")]
    Credential(String),

    #[error("Routing error: {0}")]
    Routing(String),

    #[error("Context limit exceeded for model '{model}': requested {requested_tokens} tokens, max allowed is {context_limit}")]
    ContextLimitExceeded {
        model: String,
        requested_tokens: usize,
        context_limit: usize,
    },

    #[error("No healthy provider available for request: {0}")]
    NoHealthyProvider(String),

    #[error("Provider '{provider}' unavailable: {reason}")]
    ProviderUnavailable { provider: String, reason: String },

    #[error("Varlink error: {0}")]
    Varlink(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("HTTP client error: {0}")]
    Http(String),

    #[error("Operation timed out: {0}")]
    Timeout(String),

    #[error("Internal router error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, RouterError>;
