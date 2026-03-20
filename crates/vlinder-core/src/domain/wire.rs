//! Wire-format types for service call payloads (ADR 118).

use serde::{Deserialize, Serialize};

/// A captured HTTP response, serializable via `http-serde-ext`.
#[derive(Serialize, Deserialize)]
pub struct WireResponse {
    #[serde(with = "http_serde_ext::response")]
    pub inner: http::Response<Vec<u8>>,
}

/// A captured HTTP request, serializable via `http-serde-ext`.
#[derive(Serialize, Deserialize)]
pub struct WireRequest {
    #[serde(with = "http_serde_ext::request")]
    pub inner: http::Request<Vec<u8>>,
}
