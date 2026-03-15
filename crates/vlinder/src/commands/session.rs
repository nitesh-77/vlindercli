use clap::Subcommand;

use crate::config::CliConfig;
use vlinder_core::domain::{DagStore, ForkParams, MessageType, SessionId, TimelineId};

use super::connect::{connect_harness, open_dag_store};

#[derive(Subcommand, Debug, PartialEq)]
pub enum SessionCommand {
    /// List sessions for an agent
    List {
        /// Agent name
        agent_name: String,
    },
    /// Show turns and messages in a session
    Get {
        /// Session ID
        session_id: String,
    },
    /// Create a named branch from a point in the DAG
    Fork {
        /// Session ID containing the fork point
        session_id: String,
        /// Canonical hash of the DagNode to fork from
        #[arg(long)]
        from: String,
        /// Branch name for the new timeline
        #[arg(long)]
        name: String,
    },
    /// List branches (timelines) forked from a session
    Branches {
        /// Session ID
        session_id: String,
    },
}

pub fn execute(cmd: SessionCommand) {
    match cmd {
        SessionCommand::List { agent_name } => list(&agent_name),
        SessionCommand::Get { session_id } => get(&session_id),
        SessionCommand::Fork {
            session_id,
            from,
            name,
        } => fork(&session_id, &from, &name),

        SessionCommand::Branches { session_id } => branches(&session_id),
    }
}

fn require_dag_store(config: &CliConfig) -> Box<dyn DagStore> {
    open_dag_store(config).unwrap_or_else(|| {
        eprintln!("Cannot connect to state service. Is the daemon running?");
        std::process::exit(1);
    })
}

fn list(agent_name: &str) {
    let config = CliConfig::load();
    let store = require_dag_store(&config);

    let sessions = store.list_sessions().unwrap_or_else(|e| {
        eprintln!("Failed to list sessions: {}", e);
        std::process::exit(1);
    });

    let filtered: Vec<_> = sessions
        .iter()
        .filter(|s| s.agent_name == agent_name)
        .collect();

    if filtered.is_empty() {
        println!("No sessions found for agent '{}'", agent_name);
        return;
    }

    println!(
        "{:<28} {:<40} {:<24} {:>8} STATUS",
        "NAME", "SESSION_ID", "STARTED", "MESSAGES"
    );
    for s in &filtered {
        let status = if s.is_open { "open" } else { "closed" };
        let name = store
            .get_session(&s.session_id)
            .ok()
            .flatten()
            .map(|sess| sess.name)
            .unwrap_or_default();
        println!(
            "{:<28} {:<40} {:<24} {:>8} {}",
            name,
            s.session_id,
            s.started_at.format("%Y-%m-%d %H:%M:%S"),
            s.message_count,
            status,
        );
    }
}

fn get(session_id_or_name: &str) {
    let config = CliConfig::load();
    let store = require_dag_store(&config);

    let session_id = resolve_session_id(&*store, session_id_or_name);
    let nodes = store.get_session_nodes(&session_id).unwrap_or_else(|e| {
        eprintln!("Failed to query session: {}", e);
        std::process::exit(1);
    });

    if nodes.is_empty() {
        println!("No messages found for session {}", session_id);
        return;
    }

    // Sort nodes in causal order by walking the parent_hash chain
    let nodes = causal_sort(&nodes);

    // Group by submission_id (turns), preserving causal order
    let mut turn_order: Vec<String> = Vec::new();
    let mut turn_map: std::collections::HashMap<&str, Vec<&vlinder_core::domain::DagNode>> =
        std::collections::HashMap::new();
    for node in &nodes {
        let sub_str = node.submission_id().as_str();
        if !turn_map.contains_key(sub_str) {
            turn_order.push(sub_str.to_string());
        }
        turn_map.entry(sub_str).or_default().push(node);
    }

    for sub_id in &turn_order {
        let messages = &turn_map[sub_id.as_str()];
        println!("Turn {}", sub_id);
        for node in messages {
            let ts = node.created_at.format("%H:%M:%S%.3f");
            let (from, to) = node.message.from_to();
            let mut parts = vec![
                format!("{}", ts),
                node.id.as_str()[..8].to_string(),
                node.message_type().as_str().to_string(),
                from,
                format!("-> {}", to),
            ];
            if let Some(op) = node.message.operation() {
                parts.push(format!("op:{}", op));
            }
            if let Some(ckpt) = node.message.checkpoint() {
                parts.push(format!("ckpt:{}", ckpt));
            }
            println!("  {}", parts.join(" "));
        }
        println!();
    }
}

fn fork(session_id_or_name: &str, from_hash: &str, branch_name: &str) {
    let config = CliConfig::load();
    let store = require_dag_store(&config);
    let session_id = resolve_session_id(&*store, session_id_or_name);

    // Verify the node exists and belongs to this session
    let node = store
        .get_node_by_prefix(from_hash)
        .unwrap_or_else(|e| {
            eprintln!("Failed to look up node: {}", e);
            std::process::exit(1);
        })
        .unwrap_or_else(|| {
            eprintln!("Node {} not found", from_hash);
            std::process::exit(1);
        });

    if *node.session_id() != session_id {
        eprintln!(
            "Node {} belongs to session {}, not {}",
            from_hash,
            node.session_id(),
            session_id
        );
        std::process::exit(1);
    }

    // Derive agent name from the session's Invoke message
    let agent_name = find_agent_name(&*store, &session_id).unwrap_or_else(|| {
        eprintln!("Cannot determine agent name for session {}", session_id);
        std::process::exit(1);
    });

    // Send ForkMessage through the harness/queue (CQRS: both SQL and git react)
    let harness = connect_harness(&config);
    let timeline = TimelineId::main();
    let session_id = harness.start_session(&agent_name, timeline.clone());

    let params = ForkParams {
        agent_name,
        branch_name: branch_name.to_string(),
        fork_point: node.id.clone(),
    };

    harness
        .fork_timeline(params, session_id, timeline)
        .unwrap_or_else(|e| {
            eprintln!("Failed to fork timeline: {}", e);
            std::process::exit(1);
        });

    println!(
        "Created timeline '{}' forked from {}",
        branch_name,
        &node.id.as_str()[..8]
    );
}

fn branches(session_id_or_name: &str) {
    let config = CliConfig::load();
    let store = require_dag_store(&config);
    let session_id = resolve_session_id(&*store, session_id_or_name);

    let branches = store
        .get_branches_for_session(&session_id)
        .unwrap_or_else(|e| {
            eprintln!("Failed to query branches: {}", e);
            std::process::exit(1);
        });

    if branches.is_empty() {
        println!("No branches for session '{}'", session_id);
        return;
    }

    println!("{:<4} {:<24} {:<10} FORK_POINT", "ID", "BRANCH", "STATUS");
    for b in &branches {
        let status = if b.broken_at.is_some() {
            "sealed"
        } else {
            "active"
        };
        let fork = match &b.fork_point {
            Some(h) => {
                let s = h.as_str();
                &s[..8.min(s.len())]
            }
            None => "-",
        };
        println!("{:<4} {:<24} {:<10} {}", b.id, b.name, status, fork);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Sort nodes in causal order by walking the parent_hash Merkle chain.
///
/// Finds the root (empty parent_id) and follows children to produce
/// a linear ordering that respects causality regardless of wall-clock time.
fn causal_sort(nodes: &[vlinder_core::domain::DagNode]) -> Vec<vlinder_core::domain::DagNode> {
    let by_parent: std::collections::HashMap<&str, &vlinder_core::domain::DagNode> =
        nodes.iter().map(|n| (n.parent_id.as_str(), n)).collect();

    // Find the root node (empty parent_id)
    let mut sorted = Vec::with_capacity(nodes.len());
    let Some(root) = nodes.iter().find(|n| n.parent_id.is_empty()) else {
        // No root found — fall back to input order
        return nodes.to_vec();
    };

    sorted.push(root.clone());
    let mut current_hash = root.id.as_str();
    while let Some(next) = by_parent.get(current_hash) {
        sorted.push((*next).clone());
        current_hash = next.id.as_str();
    }

    // Append any nodes not in the chain (orphans from forks, etc.)
    let in_chain: std::collections::HashSet<String> =
        sorted.iter().map(|n| n.id.to_string()).collect();
    for node in nodes {
        if !in_chain.contains(&node.id.to_string()) {
            sorted.push(node.clone());
        }
    }

    sorted
}

/// Resolve a user-provided string (UUID or petname) to a SessionId.
fn resolve_session_id(store: &dyn DagStore, id_or_name: &str) -> SessionId {
    // If it's a valid UUID, use it directly
    if let Ok(session_id) = SessionId::try_from(id_or_name.to_string()) {
        return session_id;
    }
    // Try by petname
    if let Some(session) = store.get_session_by_name(id_or_name).ok().flatten() {
        return session.id;
    }
    eprintln!("Session '{}' not found", id_or_name);
    std::process::exit(1);
}

/// Find the agent name from the Invoke message in a session.
fn find_agent_name(store: &dyn DagStore, session_id: &SessionId) -> Option<String> {
    let nodes = store.get_session_nodes(session_id).ok()?;
    nodes
        .iter()
        .find(|n| n.message_type() == MessageType::Invoke)
        .map(|n| n.message.from_to().1)
}
