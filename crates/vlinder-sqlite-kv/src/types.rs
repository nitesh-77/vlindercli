//! Request types for the sqlite-kv provider routes.
//!
//! These are the JSON bodies agents POST to sqlite-kv.vlinder.local.
//! State is NOT part of the request body — it comes from the message
//! envelope (request.state), which the sidecar manages.

use serde::{Deserialize, Serialize};

/// Retrieve a file by path.
#[derive(Debug, Serialize, Deserialize)]
pub struct KvGetRequest {
    pub path: String,
}

/// Store a file at a path. Content is a plain UTF-8 string (no base64).
#[derive(Debug, Serialize, Deserialize)]
pub struct KvPutRequest {
    pub path: String,
    pub content: String,
}

/// List files under a directory path.
#[derive(Debug, Serialize, Deserialize)]
pub struct KvListRequest {
    pub path: String,
}

/// Delete a file by path.
#[derive(Debug, Serialize, Deserialize)]
pub struct KvDeleteRequest {
    pub path: String,
}
