use std::io::{self, Read, Write};
use std::process;

use clap::Subcommand;

use vlindercli::secret_store;

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

fn put(name: &str) {
    let mut buf = Vec::new();
    if let Err(e) = io::stdin().read_to_end(&mut buf) {
        eprintln!("Failed to read stdin: {}", e);
        process::exit(1);
    }

    let store = match secret_store::from_config() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open secret store: {}", e);
            process::exit(1);
        }
    };

    if let Err(e) = store.put(name, &buf) {
        eprintln!("Failed to store secret: {}", e);
        process::exit(1);
    }

    eprintln!("Stored secret '{}'", name);
}

fn get(name: &str) {
    let store = match secret_store::from_config() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open secret store: {}", e);
            process::exit(1);
        }
    };

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
    let store = match secret_store::from_config() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open secret store: {}", e);
            process::exit(1);
        }
    };

    if let Err(e) = store.delete(name) {
        eprintln!("{}", e);
        process::exit(1);
    }

    eprintln!("Deleted secret '{}'", name);
}

fn exists(name: &str) {
    let store = match secret_store::from_config() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to open secret store: {}", e);
            process::exit(1);
        }
    };

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
