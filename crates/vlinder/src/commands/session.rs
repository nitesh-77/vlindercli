use std::str::FromStr;

use clap::Subcommand;

use crate::config::CliConfig;
use vlinder_core::domain::{
    AgentId, DagStore, ForkParams, MessageType, Operation, RepairParams, Sequence, ServiceBackend,
    ServiceType,
};

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
    /// Replay a failed service call on a repair branch
    Repair {
        /// Branch name (must match a Timeline created by fork)
        branch: String,
    },
    /// Seal old timeline and promote the repaired branch
    Promote {
        /// Branch name to promote
        branch: String,
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
        SessionCommand::Repair { branch } => repair(&branch),
        SessionCommand::Promote { branch } => promote(&branch),
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
        "{:<40} {:<24} {:>8} STATUS",
        "SESSION_ID", "STARTED", "MESSAGES"
    );
    for s in &filtered {
        let status = if s.is_open { "open" } else { "closed" };
        println!(
            "{:<40} {:<24} {:>8} {}",
            s.session_id,
            s.started_at.format("%Y-%m-%d %H:%M:%S"),
            s.message_count,
            status,
        );
    }
}

fn get(session_id: &str) {
    let config = CliConfig::load();
    let store = require_dag_store(&config);

    let nodes = store.get_session_nodes(session_id).unwrap_or_else(|e| {
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
        if !turn_map.contains_key(node.submission_id.as_str()) {
            turn_order.push(node.submission_id.clone());
        }
        turn_map.entry(&node.submission_id).or_default().push(node);
    }

    for sub_id in &turn_order {
        let messages = &turn_map[sub_id.as_str()];
        println!("Turn {}", sub_id);
        for node in messages {
            let ts = node.created_at.format("%H:%M:%S%.3f");
            let mut parts = vec![
                format!("{}", ts),
                node.hash[..8].to_string(),
                node.message_type.as_str().to_string(),
                node.from.clone(),
                format!("-> {}", node.to),
            ];
            if let Some(op) = &node.operation {
                parts.push(format!("op:{}", op));
            }
            if let Some(ckpt) = &node.checkpoint {
                parts.push(format!("ckpt:{}", ckpt));
            }
            println!("  {}", parts.join(" "));
        }
        println!();
    }
}

fn fork(session_id: &str, from_hash: &str, branch_name: &str) {
    let config = CliConfig::load();
    let store = require_dag_store(&config);

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

    if node.session_id != session_id {
        eprintln!(
            "Node {} belongs to session {}, not {}",
            from_hash, node.session_id, session_id
        );
        std::process::exit(1);
    }

    // Derive agent name from the session's Invoke message
    let agent_name = find_agent_name(&*store, session_id).unwrap_or_else(|| {
        eprintln!("Cannot determine agent name for session {}", session_id);
        std::process::exit(1);
    });

    // Send ForkMessage through the harness/queue (CQRS: both SQL and git react)
    let mut harness = connect_harness(&config);
    harness.start_session(&agent_name);

    let params = ForkParams {
        agent_name,
        branch_name: branch_name.to_string(),
        fork_point: node.hash.clone(),
        parent_timeline_id: 1, // main timeline
    };

    harness.fork_timeline(params).unwrap_or_else(|e| {
        eprintln!("Failed to fork timeline: {}", e);
        std::process::exit(1);
    });

    println!(
        "Created timeline '{}' forked from {}",
        branch_name,
        &node.hash[..8]
    );
}

fn repair(branch: &str) {
    let config = CliConfig::load();
    let store = require_dag_store(&config);

    // Look up the timeline
    let timeline = store
        .get_timeline_by_branch(branch)
        .unwrap_or_else(|e| {
            eprintln!("Failed to look up timeline: {}", e);
            std::process::exit(1);
        })
        .unwrap_or_else(|| {
            eprintln!("Timeline '{}' not found", branch);
            std::process::exit(1);
        });

    let fork_point = timeline.fork_point.unwrap_or_else(|| {
        eprintln!("Timeline '{}' has no fork point", branch);
        std::process::exit(1);
    });

    // Get the DagNode at the fork point — should be a Request
    let node = store
        .get_node(&fork_point)
        .unwrap_or_else(|e| {
            eprintln!("Failed to look up fork point: {}", e);
            std::process::exit(1);
        })
        .unwrap_or_else(|| {
            eprintln!("Fork point {} not found", fork_point);
            std::process::exit(1);
        });

    if node.message_type != MessageType::Request {
        eprintln!(
            "Fork point {} is a {} message, expected request",
            fork_point,
            node.message_type.as_str()
        );
        std::process::exit(1);
    }

    let checkpoint = node.checkpoint.clone().unwrap_or_else(|| {
        eprintln!(
            "Fork point {} has no checkpoint — repair requires a checkpoint handler",
            fork_point
        );
        std::process::exit(1);
    });

    // Parse service from node.to ("infer.ollama" -> ServiceBackend)
    let service = parse_service_backend(&node.to).unwrap_or_else(|| {
        eprintln!("Cannot parse service backend from '{}'", node.to);
        std::process::exit(1);
    });

    // Parse operation
    let operation_str = node.operation.clone().unwrap_or_else(|| {
        eprintln!("Fork point {} has no operation", fork_point);
        std::process::exit(1);
    });
    let operation = Operation::from_str(&operation_str).unwrap_or_else(|e| {
        eprintln!("Invalid operation '{}': {}", operation_str, e);
        std::process::exit(1);
    });

    // Extract sequence from diagnostics
    let sequence = extract_sequence(&node.diagnostics);

    // Derive agent name from the Invoke message in this session
    let agent_name = find_agent_name(&*store, &node.session_id).unwrap_or_else(|| {
        eprintln!(
            "Cannot determine agent name for session {}",
            node.session_id
        );
        std::process::exit(1);
    });

    let params = RepairParams {
        agent_id: AgentId::new(node.from.clone()),
        dag_parent: node.hash.clone(),
        checkpoint,
        service,
        operation,
        sequence,
        payload: node.payload.clone(),
        state: node.state.clone(),
    };

    // Set up harness
    let mut harness = connect_harness(&config);
    harness.start_session(&agent_name);
    if let Some(state) = &node.state {
        harness.set_initial_state(state.clone());
    }
    harness.set_dag_parent(node.hash.clone());
    harness.set_timeline(vlinder_core::domain::TimelineId::from(timeline.id), false);

    println!("Repairing: replaying {} on {}...", operation_str, node.to);
    match harness.repair_agent(params) {
        Ok(result) => {
            println!("Repair completed successfully.");
            println!("{}", result);
        }
        Err(e) => {
            eprintln!("Repair failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn promote(branch: &str) {
    let config = CliConfig::load();
    let store = require_dag_store(&config);

    let timeline = store
        .get_timeline_by_branch(branch)
        .unwrap_or_else(|e| {
            eprintln!("Failed to look up timeline: {}", e);
            std::process::exit(1);
        })
        .unwrap_or_else(|| {
            eprintln!("Timeline '{}' not found", branch);
            std::process::exit(1);
        });

    let parent_id = timeline.parent_timeline_id.unwrap_or_else(|| {
        eprintln!("Timeline '{}' has no parent to promote over", branch);
        std::process::exit(1);
    });

    store.seal_timeline(parent_id).unwrap_or_else(|e| {
        eprintln!("Failed to seal parent timeline: {}", e);
        std::process::exit(1);
    });

    println!(
        "Promoted '{}': parent timeline {} sealed",
        branch, parent_id
    );
}

fn branches(session_id: &str) {
    let config = CliConfig::load();
    let store = require_dag_store(&config);

    let timelines = store
        .get_timelines_for_session(session_id)
        .unwrap_or_else(|e| {
            eprintln!("Failed to query timelines: {}", e);
            std::process::exit(1);
        });

    if timelines.is_empty() {
        println!("No branches for session '{}'", session_id);
        return;
    }

    println!(
        "{:<4} {:<24} {:<10} {:<10} FORK_POINT",
        "ID", "BRANCH", "PARENT", "STATUS"
    );
    for t in &timelines {
        let status = if t.broken_at.is_some() {
            "sealed"
        } else {
            "active"
        };
        let parent = t
            .parent_timeline_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "-".to_string());
        let fork = t
            .fork_point
            .as_deref()
            .map(|h| &h[..8.min(h.len())])
            .unwrap_or("-");
        println!(
            "{:<4} {:<24} {:<10} {:<10} {}",
            t.id, t.branch_name, parent, status, fork
        );
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Sort nodes in causal order by walking the parent_hash Merkle chain.
///
/// Finds the root (empty parent_hash) and follows children to produce
/// a linear ordering that respects causality regardless of wall-clock time.
fn causal_sort(nodes: &[vlinder_core::domain::DagNode]) -> Vec<vlinder_core::domain::DagNode> {
    let by_parent: std::collections::HashMap<&str, &vlinder_core::domain::DagNode> =
        nodes.iter().map(|n| (n.parent_hash.as_str(), n)).collect();

    // Find the root node (empty parent_hash)
    let mut sorted = Vec::with_capacity(nodes.len());
    let Some(root) = nodes.iter().find(|n| n.parent_hash.is_empty()) else {
        // No root found — fall back to input order
        return nodes.to_vec();
    };

    sorted.push(root.clone());
    let mut current_hash = root.hash.as_str();
    while let Some(next) = by_parent.get(current_hash) {
        sorted.push((*next).clone());
        current_hash = &next.hash;
    }

    // Append any nodes not in the chain (orphans from forks, etc.)
    let in_chain: std::collections::HashSet<String> =
        sorted.iter().map(|n| n.hash.clone()).collect();
    for node in nodes {
        if !in_chain.contains(&node.hash) {
            sorted.push(node.clone());
        }
    }

    sorted
}

/// Parse "infer.ollama" into ServiceBackend::Infer(Ollama).
fn parse_service_backend(s: &str) -> Option<ServiceBackend> {
    let (service_str, backend_str) = s.split_once('.')?;
    let service_type = ServiceType::from_str(service_str).ok()?;
    ServiceBackend::from_parts(service_type, backend_str)
}

/// Extract sequence number from RequestDiagnostics JSON.
fn extract_sequence(diagnostics: &[u8]) -> Sequence {
    if diagnostics.is_empty() {
        return Sequence::from(1u32);
    }
    #[derive(serde::Deserialize)]
    struct Diag {
        sequence: u32,
    }
    match serde_json::from_slice::<Diag>(diagnostics) {
        Ok(d) => Sequence::from(d.sequence),
        Err(_) => Sequence::from(1u32),
    }
}

/// Find the agent name from the Invoke message in a session.
fn find_agent_name(store: &dyn DagStore, session_id: &str) -> Option<String> {
    let nodes = store.get_session_nodes(session_id).ok()?;
    nodes
        .iter()
        .find(|n| n.message_type == MessageType::Invoke)
        .map(|n| n.to.clone())
}
