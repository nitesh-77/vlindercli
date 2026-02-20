//! gRPC client implementing the Harness trait.

use std::sync::Mutex;
use tonic::transport::Channel;

use crate::domain::{Harness, HarnessType, ResourceId, TimelineId};
use super::proto::{self, harness_client::HarnessClient};

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
