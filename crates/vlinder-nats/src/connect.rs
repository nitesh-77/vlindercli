//! Shared NATS connection logic.
//!
//! Centralises URL connection and optional credential file authentication
//! so that NatsQueue, NatsSecretStore, and any future NATS users don't
//! duplicate the handshake.

use async_nats::ConnectOptions;

use crate::expand_tilde;

/// NATS connection parameters.
///
/// Constructed by the daemon config layer, consumed by NatsQueue and
/// NatsSecretStore. Keeps NATS-specific details inside vlinder-nats.
#[derive(Debug, Clone)]
pub struct NatsConfig {
    pub url: String,
    pub creds_file: Option<String>,
    /// Inline credential content (e.g. from a secret store).
    /// Takes priority over `creds_file` when both are set.
    pub creds_content: Option<String>,
}

/// Connect to a NATS server using the given config.
///
/// Returns a connected `async_nats::Client`. This is an async function —
/// callers that need a sync facade should wrap it in `Runtime::block_on`.
pub(crate) async fn nats_connect(config: &NatsConfig) -> Result<async_nats::Client, String> {
    if let Some(ref content) = config.creds_content {
        let options = ConnectOptions::with_credentials(content)
            .map_err(|e| format!("failed to parse inline credentials: {e}"))?;
        options
            .connect(&config.url)
            .await
            .map_err(|e| format!("failed to connect to {}: {e}", config.url))
    } else if let Some(ref creds_path) = config.creds_file {
        let expanded = expand_tilde(creds_path);
        let options = ConnectOptions::with_credentials_file(&expanded)
            .await
            .map_err(|e| format!("failed to load credentials from {expanded}: {e}"))?;
        options
            .connect(&config.url)
            .await
            .map_err(|e| format!("failed to connect to {}: {e}", config.url))
    } else {
        async_nats::connect(&config.url)
            .await
            .map_err(|e| format!("failed to connect to {}: {}", config.url, e))
    }
}
