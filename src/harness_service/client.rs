//! gRPC client implementing the Harness trait.

use std::sync::Mutex;
use tonic::transport::Channel;

use crate::domain::{Harness, HarnessType, ResourceId, TimelineId};
use super::proto::{self, harness_client::HarnessClient};

/// Ping a harness service at the given address, returning its protocol version.
///
/// Creates a temporary connection and sends a Ping. Returns the server's
/// version on success, None on any connection or transport error.
pub fn ping_harness(addr: &str) -> Option<(u32, u32, u32)> {
    let Ok(runtime) = tokio::runtime::Runtime::new() else {
        return None;
    };

    runtime.block_on(async {
        let Ok(mut client) = HarnessClient::connect(addr.to_string()).await else {
            return None;
        };
        client.ping(proto::PingRequest {}).await.ok().map(|r| {
            let v = r.into_inner();
            (v.major, v.minor, v.patch)
        })
    })
}

/// Harness implementation that makes gRPC calls to a remote server.
pub struct GrpcHarnessClient {
    client: Mutex<HarnessClient<Channel>>,
    runtime: tokio::runtime::Runtime,
}

impl GrpcHarnessClient {
    /// Connect to a harness server.
    pub fn connect(addr: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let runtime = tokio::runtime::Runtime::new()?;
        let client = runtime.block_on(async {
            HarnessClient::connect(addr.to_string()).await
        })?;

        Ok(Self {
            client: Mutex::new(client),
            runtime,
        })
    }
}

impl Harness for GrpcHarnessClient {
    fn harness_type(&self) -> HarnessType {
        HarnessType::Grpc
    }

    fn set_timeline(&mut self, timeline: TimelineId, sealed: bool) {
        let request = proto::SetTimelineRequest {
            timeline_id: timeline.as_str().to_string(),
            sealed,
        };

        let _ = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .set_timeline(request)
                .await
        });
    }

    fn start_session(&mut self, agent_name: &str) {
        let request = proto::StartSessionRequest {
            agent_name: agent_name.to_string(),
        };

        let _ = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .start_session(request)
                .await
        });
    }

    fn set_initial_state(&mut self, state: String) {
        let request = proto::SetInitialStateRequest { state };

        let _ = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .set_initial_state(request)
                .await
        });
    }

    fn run_agent(&mut self, agent_id: &ResourceId, input: &str) -> Result<String, String> {
        let request = proto::RunAgentRequest {
            agent_id: agent_id.as_str().to_string(),
            input: input.to_string(),
        };

        let response = self.runtime.block_on(async {
            self.client.lock().unwrap()
                .run_agent(request)
                .await
        }).map_err(|e| format!("gRPC error: {}", e))?;

        let resp = response.into_inner();
        if let Some(error) = resp.error {
            Err(error)
        } else {
            Ok(resp.output)
        }
    }
}
