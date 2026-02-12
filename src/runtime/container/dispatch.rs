//! Dispatch — state machine loop for container agents (ADR 075).
//!
//! The platform drives the agent by calling POST /handle in a loop.
//! The agent returns an `AgentAction` (what service it needs), the platform
//! executes it via `AgentBridge`, and sends the result back as an `AgentEvent`.
//! The loop continues until the agent returns `Complete`.

use std::io::Read;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use serde_json::json;

use crate::domain::{AgentAction, AgentBridge, AgentEvent};
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

/// POST a JSON event to the container's /handle endpoint and parse the response.
///
/// Passes the session ID as X-Vlinder-Session header (ADR 054).
fn post_handle(host_port: u16, event: &AgentEvent, session_id: &str) -> Result<AgentAction, DispatchError> {
    let url = format!("http://127.0.0.1:{}/handle", host_port);
    let body = serde_json::to_vec(event).expect("AgentEvent serialization cannot fail");

    match ureq::post(&url)
        .set("Content-Type", "application/json")
        .set("X-Vlinder-Session", session_id)
        .send_bytes(&body)
    {
        Ok(response) => {
            let mut buf = Vec::new();
            response.into_reader().read_to_end(&mut buf).unwrap_or_default();
            serde_json::from_slice(&buf).map_err(|e| {
                DispatchError::ContainerDead(format!("invalid AgentAction JSON: {}", e))
            })
        }
        Err(ureq::Error::Status(code, response)) => {
            // Container is alive but returned an error status.
            // Try to parse the body as AgentAction — agent might
            // return valid JSON with a non-200 status.
            let mut buf = Vec::new();
            response.into_reader().read_to_end(&mut buf).unwrap_or_default();
            serde_json::from_slice(&buf).map_err(|e| {
                let body_str = String::from_utf8_lossy(&buf);
                DispatchError::ContainerDead(
                    format!("POST /handle HTTP {} — invalid JSON: {} (body: {})", code, e, body_str)
                )
            })
        }
        Err(ureq::Error::Transport(t)) => {
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

/// Run the state machine dispatch loop for a container agent.
///
/// 1. Send `AgentEvent::Invoke` with the input payload and empty state
/// 2. Receive `AgentAction` from the agent
/// 3. Execute the requested service call via `AgentBridge`
/// 4. Send the result back as an `AgentEvent`
/// 5. Repeat until `AgentAction::Complete`
pub(crate) fn dispatch_state_machine(
    host_port: u16,
    payload: &[u8],
    session_id: &str,
    bridge: Arc<dyn AgentBridge>,
) -> Result<Vec<u8>, DispatchError> {
    let input = String::from_utf8_lossy(payload).to_string();
    let mut event = AgentEvent::Invoke {
        input,
        state: json!({}),
    };

    loop {
        let action = post_handle(host_port, &event, session_id)?;

        event = match action {
            AgentAction::Complete { payload, .. } => {
                return Ok(payload.into_bytes());
            }

            AgentAction::KvGet { path, state } => {
                match bridge.kv_get(&path) {
                    Ok(data) => AgentEvent::KvGet { data, state },
                    Err(msg) => AgentEvent::Error { message: msg, state },
                }
            }

            AgentAction::KvPut { path, content, state } => {
                match bridge.kv_put(&path, &content) {
                    Ok(()) => AgentEvent::KvPut { state },
                    Err(msg) => AgentEvent::Error { message: msg, state },
                }
            }

            AgentAction::KvList { prefix, state } => {
                match bridge.kv_list(&prefix) {
                    Ok(paths) => AgentEvent::KvList { paths, state },
                    Err(msg) => AgentEvent::Error { message: msg, state },
                }
            }

            AgentAction::KvDelete { path, state } => {
                match bridge.kv_delete(&path) {
                    Ok(existed) => AgentEvent::KvDelete { existed, state },
                    Err(msg) => AgentEvent::Error { message: msg, state },
                }
            }

            AgentAction::VectorStore { key, vector, metadata, state } => {
                match bridge.vector_store(&key, &vector, &metadata) {
                    Ok(()) => AgentEvent::VectorStore { state },
                    Err(msg) => AgentEvent::Error { message: msg, state },
                }
            }

            AgentAction::VectorSearch { vector, limit, state } => {
                match bridge.vector_search(&vector, limit) {
                    Ok(matches) => AgentEvent::VectorSearch { matches, state },
                    Err(msg) => AgentEvent::Error { message: msg, state },
                }
            }

            AgentAction::VectorDelete { key, state } => {
                match bridge.vector_delete(&key) {
                    Ok(existed) => AgentEvent::VectorDelete { existed, state },
                    Err(msg) => AgentEvent::Error { message: msg, state },
                }
            }

            AgentAction::Infer { model, prompt, max_tokens, state } => {
                match bridge.infer(&model, &prompt, max_tokens) {
                    Ok(text) => AgentEvent::Infer { text, state },
                    Err(msg) => AgentEvent::Error { message: msg, state },
                }
            }

            AgentAction::Embed { model, text, state } => {
                match bridge.embed(&model, &text) {
                    Ok(vector) => AgentEvent::Embed { vector, state },
                    Err(msg) => AgentEvent::Error { message: msg, state },
                }
            }

            AgentAction::Delegate { agent, input, state } => {
                match bridge.delegate(&agent, &input) {
                    Ok(handle) => {
                        // Delegate is a two-phase operation: delegate + wait
                        match bridge.wait(&handle) {
                            Ok(output_bytes) => {
                                let output = String::from_utf8_lossy(&output_bytes).to_string();
                                AgentEvent::Delegate { output, state }
                            }
                            Err(msg) => AgentEvent::Error { message: msg, state },
                        }
                    }
                    Err(msg) => AgentEvent::Error { message: msg, state },
                }
            }
        };
    }
}
