//! Agent bridge — routes agent SDK calls to platform services via the queue.
//!
//! The bridge is the interface between agent code and the platform.
//! Agents call SDK methods (kv_get, infer, delegate, etc.) which the
//! bridge translates into queue messages and polls for responses.
//!
//! - `http_bridge`: queue-backed AgentBridge implementation
//! - `http_server`: HTTP transport that exposes the bridge to containers

pub(crate) mod http_bridge;
pub(crate) mod http_server;

pub(crate) use http_bridge::HttpBridge;
pub(crate) use http_server::HttpBridgeServer;
