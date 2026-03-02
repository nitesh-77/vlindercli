mod connect;
mod queue;
pub mod secret_service;
mod secret_store;

/// Expand `~/...` to the user's home directory.
pub(crate) fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

pub use connect::NatsConfig;
pub use queue::NatsQueue;
pub use queue::{
    complete_to_nats_headers, delegate_to_nats_headers, from_nats_headers, invoke_to_nats_headers,
    request_to_nats_headers, response_to_nats_headers, subject_to_routing_key,
};
pub use secret_store::NatsSecretStore;
