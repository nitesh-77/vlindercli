use clap::Subcommand;

use crate::config::CliConfig;

use super::connect::open_dag_store;

#[derive(Subcommand, Debug, PartialEq)]
pub enum TurnCommand {
    /// Show all messages in a turn (submission)
    Get {
        /// Submission ID
        submission_id: String,
    },
}

pub fn execute(cmd: TurnCommand) {
    match cmd {
        TurnCommand::Get { submission_id } => get(&submission_id),
    }
}

fn get(submission_id: &str) {
    let config = CliConfig::load();
    let store = open_dag_store(&config).unwrap_or_else(|| {
        eprintln!("Cannot connect to state service. Is the daemon running?");
        std::process::exit(1);
    });

    let nodes = store
        .get_nodes_by_submission(submission_id)
        .unwrap_or_else(|e| {
            eprintln!("Failed to query turn: {e}");
            std::process::exit(1);
        });

    if nodes.is_empty() {
        println!("No messages found for submission {submission_id}");
        return;
    }

    for node in &nodes {
        let msg = node.message.as_ref().expect("dag node missing message");
        let (from, to) = msg.sender_receiver();
        println!("Hash:       {}", node.id);
        println!("Parent:     {}", node.parent_id);
        println!("Type:       {}", node.message_type().as_str());
        println!("From:       {from}");
        println!("To:         {to}");
        println!("Session:    {}", node.session_id());
        if let Some(op) = msg.operation() {
            println!("Operation:  {op}");
        }
        if let Some(ckpt) = msg.checkpoint() {
            println!("Checkpoint: {ckpt}");
        }
        if let Some(state) = msg.state() {
            println!("State:      {state}");
        }
        println!("Created:    {}", node.created_at);
        let payload = node.payload();
        match std::str::from_utf8(payload) {
            Ok(text) if !text.is_empty() => println!("Payload:    {text}"),
            _ if !payload.is_empty() => {
                println!("Payload:    <{} bytes binary>", payload.len());
            }
            _ => {}
        }
        println!();
    }
}
