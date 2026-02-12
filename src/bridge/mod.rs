//! Agent bridge — routes agent SDK calls to platform services via the queue.
//!
//! The bridge is the interface between agent code and the platform.
//! Agents are state machines (ADR 075): the platform calls POST /handle
//! on the container, and the dispatch loop uses the bridge to execute
//! the requested service calls.
//!
//! - `http_bridge`: queue-backed AgentBridge implementation

pub(crate) mod http_bridge;

pub(crate) use http_bridge::HttpBridge;
