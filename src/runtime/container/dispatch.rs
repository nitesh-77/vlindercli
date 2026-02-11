//! Dispatch — send work to a container and track the result.
//!
//! Contains the types for tracking in-flight invocations and the HTTP POST
//! function that delivers payloads to a container's /invoke endpoint.

use std::io::Read;
use std::thread::JoinHandle;
use std::time::Instant;

use crate::queue::InvokeMessage;

/// Tracks an in-flight invocation dispatched to a container.
pub(crate) struct RunningTask {
    pub(crate) handle: JoinHandle<Result<Vec<u8>, DispatchError>>,
    pub(crate) invoke: InvokeMessage,
    /// For delegated work: the subject to send the result to.
    /// None for harness-invoked work (uses normal send_complete).
    pub(crate) reply_subject: Option<String>,
    /// Wall-clock start time for duration measurement.
    pub(crate) started_at: Instant,
    /// True if this is a retry after dispatch-failure eviction (ADR 073).
    /// Prevents infinite retry loops — at most one retry per invocation.
    pub(crate) is_retry: bool,
}

/// Dispatch failure classification (ADR 073).
///
/// Transport errors (connection refused, timeout) indicate the container is dead.
/// HTTP status errors mean the container is alive — treated as successful dispatch.
pub(crate) enum DispatchError {
    /// The container is unreachable (connection refused, DNS failure, timeout).
    ContainerDead(String),
}

/// Dispatch payload to a container's /invoke endpoint via HTTP POST.
///
/// Passes the session ID as X-Vlinder-Session header (ADR 054).
///
/// Returns `Ok(body)` if the container responded (any HTTP status — the agent
/// is alive). Returns `Err(ContainerDead)` on transport errors (connection
/// refused, timeout) — the container is gone (ADR 073).
pub(crate) fn dispatch_to_container(host_port: u16, payload: &[u8], session_id: &str) -> Result<Vec<u8>, DispatchError> {
    let url = format!("http://127.0.0.1:{}/invoke", host_port);

    match ureq::post(&url)
        .set("X-Vlinder-Session", session_id)
        .send_bytes(payload)
    {
        Ok(response) => {
            let mut body = Vec::new();
            response.into_reader().read_to_end(&mut body).unwrap_or_default();
            Ok(body)
        }
        Err(ureq::Error::Status(code, response)) => {
            // Container is alive but returned an error status — pass the body through
            let mut body = Vec::new();
            response.into_reader().read_to_end(&mut body).unwrap_or_default();
            if body.is_empty() {
                body = format!("[error] agent returned HTTP {}", code).into_bytes();
            }
            Ok(body)
        }
        Err(ureq::Error::Transport(t)) => {
            // Container is dead — connection refused, DNS failure, timeout
            tracing::warn!(
                event = "dispatch.transport_error",
                port = host_port,
                error = %t,
                "Transport error — container likely dead"
            );
            Err(DispatchError::ContainerDead(t.to_string()))
        }
    }
}
