#[cfg(feature = "server")]
mod connect;
#[cfg(feature = "server")]
mod queue;
pub mod secret_service;
#[cfg(feature = "server")]
mod secret_store;

/// Expand `~/...` to the user's home directory.
#[cfg(feature = "server")]
pub(crate) fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

#[cfg(feature = "server")]
pub use connect::NatsConfig;
#[cfg(feature = "server")]
pub use queue::NatsQueue;
#[cfg(feature = "server")]
pub use queue::{
    complete_to_nats_headers, delegate_to_nats_headers, from_nats_headers, invoke_to_nats_headers,
    request_to_nats_headers, response_to_nats_headers, subject_to_routing_key,
};
#[cfg(feature = "server")]
pub use secret_store::NatsSecretStore;
