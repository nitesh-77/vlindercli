//! `ObservableMessageV2` — clean message types with routing separated from payload.

use super::super::routing_key::DataRoutingKey;
use super::complete::CompleteMessageV2;
use super::invoke::InvokeMessage;

/// Observable message with routing and payload cleanly separated.
///
/// New message types are added here as they migrate from `ObservableMessage`.
/// Old variants are removed from `ObservableMessage` once fully migrated.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ObservableMessageV2 {
    InvokeV2 {
        key: DataRoutingKey,
        msg: InvokeMessage,
    },
    CompleteV2 {
        key: DataRoutingKey,
        msg: CompleteMessageV2,
    },
}
