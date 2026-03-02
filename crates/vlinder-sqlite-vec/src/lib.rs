//! SQLite-vec provider — declares the hostname and routes
//! for the sqlite-vec vector storage backend.

mod storage;
mod types;
mod worker;

pub use storage::SqliteVectorStorage;
pub use types::{SqliteVecDeleteRequest, SqliteVecSearchRequest, SqliteVecStoreRequest};
pub use worker::SqliteVecWorker;

use vlinder_core::domain::{
    HttpMethod, Operation, ProviderHost, ProviderRoute, ServiceBackend, VectorStorageType,
};

/// The virtual hostname the sidecar will serve for sqlite-vec.
pub const HOSTNAME: &str = "sqlite-vec.vlinder.local";

/// Build the provider host declaration for sqlite-vec vector storage.
pub fn provider_host() -> ProviderHost {
    let backend = ServiceBackend::Vec(VectorStorageType::SqliteVec);

    ProviderHost::new(
        HOSTNAME,
        vec![
            ProviderRoute::new::<SqliteVecStoreRequest, serde_json::Value>(
                HttpMethod::Post,
                "/store",
                backend,
                Operation::Store,
            ),
            ProviderRoute::new::<SqliteVecSearchRequest, serde_json::Value>(
                HttpMethod::Post,
                "/search",
                backend,
                Operation::Search,
            ),
            ProviderRoute::new::<SqliteVecDeleteRequest, serde_json::Value>(
                HttpMethod::Post,
                "/delete",
                backend,
                Operation::Delete,
            ),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostname_is_sqlite_vec_vlinder_local() {
        assert_eq!(HOSTNAME, "sqlite-vec.vlinder.local");
    }

    #[test]
    fn provider_host_has_correct_hostname() {
        let host = provider_host();
        assert_eq!(host.hostname, "sqlite-vec.vlinder.local");
    }

    #[test]
    fn provider_host_has_three_routes() {
        let host = provider_host();
        assert_eq!(host.routes.len(), 3);
    }

    #[test]
    fn routes_are_store_search_delete() {
        let host = provider_host();
        assert_eq!(host.routes[0].path, "/store");
        assert_eq!(host.routes[1].path, "/search");
        assert_eq!(host.routes[2].path, "/delete");
    }

    #[test]
    fn all_routes_are_post() {
        let host = provider_host();
        for route in &host.routes {
            assert_eq!(route.method, HttpMethod::Post);
        }
    }

    #[test]
    fn all_routes_have_vec_backend() {
        let host = provider_host();
        let expected = ServiceBackend::Vec(VectorStorageType::SqliteVec);
        for route in &host.routes {
            assert_eq!(route.service_backend, expected);
        }
    }

    #[test]
    fn routes_have_correct_operations() {
        let host = provider_host();
        assert_eq!(host.routes[0].operation, Operation::Store);
        assert_eq!(host.routes[1].operation, Operation::Search);
        assert_eq!(host.routes[2].operation, Operation::Delete);
    }

    #[test]
    fn store_accepts_valid_request() {
        let host = provider_host();
        let body = serde_json::json!({
            "key": "test-key",
            "vector": [0.1, 0.2, 0.3],
            "metadata": "test metadata"
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        assert!((host.routes[0].validate_request)(&bytes).is_ok());
    }

    #[test]
    fn search_accepts_valid_request() {
        let host = provider_host();
        let body = serde_json::json!({
            "vector": [0.1, 0.2, 0.3],
            "limit": 5
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        assert!((host.routes[1].validate_request)(&bytes).is_ok());
    }

    #[test]
    fn delete_accepts_valid_request() {
        let host = provider_host();
        let body = serde_json::json!({
            "key": "test-key"
        });
        let bytes = serde_json::to_vec(&body).unwrap();
        assert!((host.routes[2].validate_request)(&bytes).is_ok());
    }

    #[test]
    fn rejects_invalid_request() {
        let host = provider_host();
        for route in &host.routes {
            assert!((route.validate_request)(b"not json").is_err());
        }
    }
}
