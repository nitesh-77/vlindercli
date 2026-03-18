//! Message queue implementations (ADR 044).
//!
//! Domain types (messages, traits, diagnostics) live in `crate::domain`.
//! This module contains concrete implementations:
//! - `InMemoryQueue`: Single-process, for local development
//! - `RecordingQueue`: Decorator for synchronous DAG recording
//!
//! NATS-backed `NatsQueue` lives in the `vlinder-nats` crate.
//!
//! Factory functions (`from_config`, `recording_from_config`) live in
//! `crate::queue_factory` to keep this module free of config/infra deps.

mod in_memory;
pub mod recording;

pub use in_memory::InMemoryQueue;
pub use recording::RecordingQueue;
