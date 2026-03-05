//! SQLite-kv provider — declares the hostname and routes
//! for the sqlite-backed KV (object) storage backend.

#[cfg(feature = "worker")]
pub mod state_store;
#[cfg(feature = "worker")]
mod storage;
mod types;
#[cfg(feature = "worker")]
mod worker;

#[cfg(feature = "worker")]
pub use state_store::SqliteStateStore;
#[cfg(feature = "worker")]
pub use storage::SqliteObjectStorage;
pub use types::{KvDeleteRequest, KvGetRequest, KvListRequest, KvPutRequest};
#[cfg(feature = "worker")]
pub use worker::KvWorker;

use vlinder_core::domain::{
    HttpMethod, ObjectStorageType, Operation, ProviderHost, ProviderRoute, ServiceBackend,
};

/// The virtual hostname the sidecar will serve for sqlite-kv.
pub const HOSTNAME: &str = "sqlite-kv.vlinder.local";

/// Build the provider host declaration for sqlite-kv object storage.
pub fn provider_host() -> ProviderHost {
    let backend = ServiceBackend::Kv(ObjectStorageType::Sqlite);

    ProviderHost::new(
        HOSTNAME,
        vec![
            ProviderRoute::new::<KvGetRequest, serde_json::Value>(
                HttpMethod::Post,
                "/get",
                backend,
                Operation::Get,
            ),
            ProviderRoute::new::<KvPutRequest, serde_json::Value>(
                HttpMethod::Post,
                "/put",
                backend,
                Operation::Put,
            ),
            ProviderRoute::new::<KvDeleteRequest, serde_json::Value>(
                HttpMethod::Post,
                "/delete",
                backend,
                Operation::Delete,
            ),
            ProviderRoute::new::<KvListRequest, serde_json::Value>(
                HttpMethod::Post,
                "/list",
                backend,
                Operation::List,
            ),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostname_is_sqlite_kv_vlinder_local() {
        assert_eq!(HOSTNAME, "sqlite-kv.vlinder.local");
    }

    #[test]
    fn provider_host_has_correct_hostname() {
        let host = provider_host();
        assert_eq!(host.hostname, "sqlite-kv.vlinder.local");
    }

    #[test]
    fn provider_host_has_four_routes() {
        let host = provider_host();
        assert_eq!(host.routes.len(), 4);
    }

    #[test]
    fn routes_are_get_put_delete_list() {
        let host = provider_host();
        assert_eq!(host.routes[0].path, "/get");
        assert_eq!(host.routes[1].path, "/put");
        assert_eq!(host.routes[2].path, "/delete");
        assert_eq!(host.routes[3].path, "/list");
    }

    #[test]
    fn all_routes_are_post() {
        let host = provider_host();
        for route in &host.routes {
            assert_eq!(route.method, HttpMethod::Post);
        }
    }

    #[test]
    fn all_routes_have_kv_backend() {
        let host = provider_host();
        let expected = ServiceBackend::Kv(ObjectStorageType::Sqlite);
        for route in &host.routes {
            assert_eq!(route.service_backend, expected);
        }
    }

    #[test]
    fn routes_have_correct_operations() {
        let host = provider_host();
        assert_eq!(host.routes[0].operation, Operation::Get);
        assert_eq!(host.routes[1].operation, Operation::Put);
        assert_eq!(host.routes[2].operation, Operation::Delete);
        assert_eq!(host.routes[3].operation, Operation::List);
    }

    #[test]
    fn get_accepts_valid_request() {
        let host = provider_host();
        let body = serde_json::json!({"path": "/test.txt"});
        let bytes = serde_json::to_vec(&body).unwrap();
        assert!((host.routes[0].validate_request)(&bytes).is_ok());
    }

    #[test]
    fn put_accepts_valid_request() {
        let host = provider_host();
        let body = serde_json::json!({"path": "/test.txt", "content": "hello"});
        let bytes = serde_json::to_vec(&body).unwrap();
        assert!((host.routes[1].validate_request)(&bytes).is_ok());
    }

    #[test]
    fn rejects_invalid_request() {
        let host = provider_host();
        for route in &host.routes {
            assert!((route.validate_request)(b"not json").is_err());
        }
    }
}
