//! Operation — typed enum for the operation dimension of the routing triple.
//!
//! Part of the (service, backend, operation) routing triple (ADR 043).
//! Each service defines which operations it supports:
//! - Kv: Get, Put, List, Delete
//! - Vec: Store, Search, Delete
//! - Infer: Run
//! - Embed: Run

use serde::{Deserialize, Serialize};

/// The operation being performed on a service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Get,
    Put,
    List,
    Delete,
    Store,
    Search,
    Run,
}

impl Operation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Operation::Get => "get",
            Operation::Put => "put",
            Operation::List => "list",
            Operation::Delete => "delete",
            Operation::Store => "store",
            Operation::Search => "search",
            Operation::Run => "run",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "get" => Some(Operation::Get),
            "put" => Some(Operation::Put),
            "list" => Some(Operation::List),
            "delete" => Some(Operation::Delete),
            "store" => Some(Operation::Store),
            "search" => Some(Operation::Search),
            "run" => Some(Operation::Run),
            _ => None,
        }
    }
}

impl std::fmt::Display for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_round_trips() {
        let ops = [
            Operation::Get, Operation::Put, Operation::List, Operation::Delete,
            Operation::Store, Operation::Search, Operation::Run,
        ];
        for op in ops {
            assert_eq!(Operation::from_str(op.as_str()), Some(op));
        }
    }

    #[test]
    fn unknown_returns_none() {
        assert_eq!(Operation::from_str("unknown"), None);
        assert_eq!(Operation::from_str(""), None);
    }

    #[test]
    fn serde_round_trip() {
        let op = Operation::Store;
        let json = serde_json::to_string(&op).unwrap();
        assert_eq!(json, "\"store\"");
        let parsed: Operation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, op);
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(format!("{}", Operation::Run), "run");
        assert_eq!(format!("{}", Operation::Delete), "delete");
    }
}
