use std::io::{self, Read, Write};
use std::process;
use std::sync::Arc;

use clap::Subcommand;

use vlindercli::config::Config;
use vlinder_core::domain::SecretStore;
use vlinder_proto::secret_service::GrpcSecretClient;

#[derive(Subcommand, Debug, PartialEq)]
pub enum SecretCommand {
    /// Store a secret (reads value from stdin)
    Put { name: String },
    /// Retrieve and print a secret
    Get { name: String },
    /// Delete a secret
    Delete { name: String },
    /// Check if a secret exists (exit code 0/1)
    Exists { name: String },
}

pub fn execute(cmd: SecretCommand) {
    match cmd {
        SecretCommand::Put { name } => put(&name),
        SecretCommand::Get { name } => get(&name),
        SecretCommand::Delete { name } => delete(&name),
        SecretCommand::Exists { name } => exists(&name),
    }
}

fn open_store() -> Arc<dyn SecretStore> {
    let config = Config::load();
    let addr = &config.distributed.secret_addr;
    match GrpcSecretClient::connect(addr) {
        Ok(client) => Arc::new(client),
        Err(e) => {
            eprintln!("Failed to connect to secret service at {}: {}", addr, e);
            process::exit(1);
        }
    }
}

fn put(name: &str) {
    let mut buf = Vec::new();
    if let Err(e) = io::stdin().read_to_end(&mut buf) {
        eprintln!("Failed to read stdin: {}", e);
        process::exit(1);
    }

    let store = open_store();

    if let Err(e) = store.put(name, &buf) {
        eprintln!("Failed to store secret: {}", e);
        process::exit(1);
    }

    eprintln!("Stored secret '{}'", name);
}

fn get(name: &str) {
    let store = open_store();

    match store.get(name) {
        Ok(value) => {
            if let Err(e) = io::stdout().write_all(&value) {
                eprintln!("Failed to write to stdout: {}", e);
                process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("{}", e);
            process::exit(1);
        }
    }
}

fn delete(name: &str) {
    let store = open_store();

    if let Err(e) = store.delete(name) {
        eprintln!("{}", e);
        process::exit(1);
    }

    eprintln!("Deleted secret '{}'", name);
}

fn exists(name: &str) {
    let store = open_store();

    match store.exists(name) {
        Ok(true) => {
            println!("exists");
        }
        Ok(false) => {
            println!("not found");
            process::exit(1);
        }
        Err(e) => {
            eprintln!("{}", e);
            process::exit(1);
        }
    }
}
