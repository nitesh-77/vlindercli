//! Dual-output trace logging for the sidecar.
//!
//! Logs are emitted immediately via `tracing` (visible in `podman logs`)
//! AND collected in a `Vec<String>` that becomes part of the CompleteMessage
//! diagnostics. This ensures that if the sidecar hangs mid-operation, the
//! last log line in `podman logs` shows exactly what it was doing.
//!
//! Usage:
//!   let mut trace = TraceLog::new();
//!   trace.log("Posting to agent container");
//!   // ... do work ...
//!   let lines: Vec<String> = trace.finish();

/// Collects trace lines and emits each one immediately via tracing.
pub struct TraceLog {
    lines: Vec<String>,
}

impl TraceLog {
    pub fn new() -> Self {
        Self { lines: Vec::new() }
    }

    /// Log a message: emits immediately via `tracing::info!` and stores it.
    pub fn log(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        tracing::info!(event = "trace", "{}", msg);
        self.lines.push(msg);
    }

    /// Consume the trace and return all collected lines.
    #[allow(dead_code)]
    pub fn finish(self) -> Vec<String> {
        self.lines
    }
}
