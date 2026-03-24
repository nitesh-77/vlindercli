//! `InvokeMessage`: Data-plane invoke payload (ADR 121).

use serde::{Deserialize, Serialize};

use super::super::diagnostics::InvokeDiagnostics;
use super::identity::{DagNodeId, MessageId};

/// Serde helper: encode Vec<u8> as a base64 string for JSON-friendly transport.
mod base64_serde {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(d)?;
        STANDARD.decode(&encoded).map_err(serde::de::Error::custom)
    }
}

/// Data-plane invoke payload — everything NOT in the subject.
///
/// The subject carries routing (session, branch, submission, harness, runtime, agent)
/// and protocol version. This struct carries the domain data that goes in the
/// NATS payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InvokeMessage {
    pub id: MessageId,
    /// Content-addressed DAG node ID. Computed by the recording queue
    /// before publish; empty on initial construction.
    #[serde(default = "DagNodeId::root")]
    pub dag_id: DagNodeId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    pub diagnostics: InvokeDiagnostics,
    pub dag_parent: DagNodeId,
    #[serde(with = "base64_serde")]
    pub payload: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invoke_message_json_round_trip() {
        let msg = InvokeMessage {
            id: MessageId::from("msg-1".to_string()),
            dag_id: DagNodeId::root(),
            state: Some("abc123".to_string()),
            diagnostics: InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
            },
            dag_parent: DagNodeId::root(),
            payload: b"hello world".to_vec(),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let back: InvokeMessage = serde_json::from_str(&json).unwrap();

        assert_eq!(back, msg);
    }

    #[test]
    fn invoke_message_payload_is_base64() {
        let msg = InvokeMessage {
            id: MessageId::from("msg-1".to_string()),
            dag_id: DagNodeId::root(),
            state: None,
            diagnostics: InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
            },
            dag_parent: DagNodeId::root(),
            payload: b"hello world".to_vec(),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(raw["payload"].is_string());
        assert_eq!(raw["payload"].as_str().unwrap(), "aGVsbG8gd29ybGQ=");
    }

    #[test]
    fn invoke_message_omits_none_state() {
        let msg = InvokeMessage {
            id: MessageId::from("msg-1".to_string()),
            dag_id: DagNodeId::root(),
            state: None,
            diagnostics: InvokeDiagnostics {
                harness_version: "0.1.0".to_string(),
            },
            dag_parent: DagNodeId::root(),
            payload: b"test".to_vec(),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(raw.get("state").is_none());
    }
}
