use crate::error::{Result, RouterError};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Systemd credentials directory environment variable.
pub const CREDENTIALS_DIRECTORY_ENV: &str = "CREDENTIALS_DIRECTORY";

/// Resolves an API key or credential string using systemd credentials or environment variables.
///
/// Supported formats:
/// - `cred:<name>`: Reads from `$CREDENTIALS_DIRECTORY/<name>`
/// - `env:<NAME>`: Reads from environment variable `<NAME>`
/// - `${NAME}` or `$NAME`: Reads from environment variable `<NAME>`
/// - `/path/to/file`: Reads credential directly from filesystem path
/// - literal string: If no prefix matches and not a file, used as raw key
/// - fallback for provider: Checks `$CREDENTIALS_DIRECTORY/<provider_id>_api_key` or `<PROVIDER_ID>_API_KEY`
pub fn resolve_credential(raw: Option<&str>, provider_id: &str) -> Result<Option<String>> {
    if let Some(spec) = raw {
        let trimmed = spec.trim();
        if trimmed.is_empty() {
            return resolve_provider_fallback(provider_id);
        }

        // 1. cred:<name>
        if let Some(cred_name) = trimmed.strip_prefix("cred:") {
            return read_systemd_credential(cred_name);
        }

        // 2. env:<NAME>
        if let Some(var_name) = trimmed.strip_prefix("env:") {
            return read_env_var(var_name);
        }

        // 3. ${NAME}
        if trimmed.starts_with("${") && trimmed.ends_with('}') && trimmed.len() > 3 {
            let var_name = &trimmed[2..trimmed.len() - 1];
            return read_env_var(var_name);
        }

        // 4. $NAME
        if let Some(var_name) = trimmed.strip_prefix('$') {
            return read_env_var(var_name);
        }

        // 5. Direct file path if exists
        let path = Path::new(trimmed);
        if path.is_absolute() && path.exists() {
            return read_file_credential(path);
        }

        // 6. Otherwise literal key
        return Ok(Some(trimmed.to_string()));
    }

    resolve_provider_fallback(provider_id)
}

fn read_systemd_credential(name: &str) -> Result<Option<String>> {
    if let Ok(dir) = env::var(CREDENTIALS_DIRECTORY_ENV) {
        let p = PathBuf::from(dir).join(name);
        if p.exists() {
            return read_file_credential(&p);
        }
    }

    // Default fallback path for systemd service credentials
    let fallback = PathBuf::from("/run/credentials/routerd.service").join(name);
    if fallback.exists() {
        return read_file_credential(&fallback);
    }

    // Check environment variable matching uppercase credential name
    let env_name = name.replace('-', "_").to_uppercase();
    if let Ok(val) = env::var(&env_name) {
        let trimmed = val.trim().to_string();
        if !trimmed.is_empty() {
            debug!("Resolved credential '{}' from environment variable {}", name, env_name);
            return Ok(Some(trimmed));
        }
    }

    Ok(None)
}

fn read_env_var(name: &str) -> Result<Option<String>> {
    match env::var(name) {
        Ok(val) => {
            let trimmed = val.trim().to_string();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed))
            }
        }
        Err(_) => Ok(None),
    }
}

fn read_file_credential(path: &Path) -> Result<Option<String>> {
    let content = fs::read_to_string(path).map_err(|e| {
        RouterError::Credential(format!("Failed to read credential file '{:?}': {}", path, e))
    })?;
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed))
    }
}

fn resolve_provider_fallback(provider_id: &str) -> Result<Option<String>> {
    let clean_id = provider_id.replace('-', "_").to_lowercase();
    let cred_name = format!("{}_api_key", clean_id);

    // Check systemd credentials first
    if let Ok(Some(k)) = read_systemd_credential(&cred_name) {
        return Ok(Some(k));
    }

    // Check ROUTERD_<PROVIDER>_API_KEY
    let env_pref = format!("ROUTERD_{}_API_KEY", clean_id.to_uppercase());
    if let Ok(Some(k)) = read_env_var(&env_pref) {
        return Ok(Some(k));
    }

    // Check <PROVIDER>_API_KEY
    let env_bare = format!("{}_API_KEY", clean_id.to_uppercase());
    if let Ok(Some(k)) = read_env_var(&env_bare) {
        return Ok(Some(k));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::io::Write;

    #[test]
    fn test_literal_and_env_resolution() {
        env::set_var("TEST_ROUTER_API_KEY", "secret_key_1234");
        assert_eq!(
            resolve_credential(Some("env:TEST_ROUTER_API_KEY"), "test").unwrap(),
            Some("secret_key_1234".to_string())
        );
        assert_eq!(
            resolve_credential(Some("${TEST_ROUTER_API_KEY}"), "test").unwrap(),
            Some("secret_key_1234".to_string())
        );
        assert_eq!(
            resolve_credential(Some("$TEST_ROUTER_API_KEY"), "test").unwrap(),
            Some("secret_key_1234".to_string())
        );
        assert_eq!(
            resolve_credential(Some("raw-secret-value"), "test").unwrap(),
            Some("raw-secret-value".to_string())
        );
    }

    #[test]
    fn test_file_and_systemd_credential() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, " file_secret_token \n").unwrap();
        let path = file.path().to_str().unwrap();

        assert_eq!(
            resolve_credential(Some(path), "test").unwrap(),
            Some("file_secret_token".to_string())
        );
    }
}
