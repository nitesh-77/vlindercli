use std::path::PathBuf;

use clap::Subcommand;

use vlindercli::domain::Daemon;

use super::repl;

#[derive(Subcommand, Debug, PartialEq)]
pub enum AgentCommand {
    /// Run an agent interactively
    Run {
        /// Path to agent directory (default: current directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
}

pub fn execute(cmd: AgentCommand) {
    match cmd {
        AgentCommand::Run { path } => run(path),
    }
}

fn run(path: Option<PathBuf>) {
    let agent_path = path.unwrap_or_else(|| {
        std::env::current_dir().expect("Failed to get current directory")
    });

    // Canonicalize to absolute path
    let absolute_path = agent_path
        .canonicalize()
        .expect("Failed to resolve agent path");

    // Resolve relative paths in manifest to absolute
    let resolved_toml = resolve_manifest_paths(&absolute_path);

    // Create daemon
    let mut daemon = Daemon::new();

    // Run REPL
    repl::run(|input| {
        match daemon.invoke(&resolved_toml, input) {
            Ok(job_id) => {
                // Tick until complete
                loop {
                    daemon.tick();
                    if let Some(result) = daemon.poll(&job_id) {
                        return result;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            Err(e) => format!("[error] {}", e),
        }
    });
}

/// Resolve relative paths in manifest to absolute paths.
///
/// This is the CLI's job - daemon receives resolved TOML.
fn resolve_manifest_paths(base_path: &std::path::Path) -> String {
    // Simple approach: use AgentManifest to parse and resolve, then rebuild
    use vlindercli::domain::AgentManifest;

    let manifest_path = base_path.join("agent.toml");
    let manifest = AgentManifest::load(&manifest_path)
        .expect("Failed to parse manifest");

    // Rebuild TOML with resolved paths
    let mut result = format!(
        "name = \"{}\"\ndescription = ",
        manifest.name
    );

    // Handle multi-line description
    if manifest.description.contains('\n') {
        result.push_str(&format!("\"\"\"\n{}\"\"\"\n", manifest.description));
    } else {
        result.push_str(&format!("\"{}\"\n", manifest.description));
    }

    result.push_str(&format!("id = \"{}\"\n", manifest.id));

    if let Some(ref source) = manifest.source {
        result.push_str(&format!("source = \"{}\"\n", source));
    }

    // Requirements
    result.push_str("\n[requirements]\n");
    result.push_str(&format!("services = {:?}\n", manifest.requirements.services));

    if !manifest.requirements.models.is_empty() {
        result.push_str("\n[requirements.models]\n");
        for (name, uri) in &manifest.requirements.models {
            result.push_str(&format!("{} = \"{}\"\n", name, uri));
        }
    }

    // Mounts
    for mount in &manifest.mounts {
        result.push_str(&format!(
            "\n[[mounts]]\nhost_path = \"{}\"\nguest_path = \"{}\"\nmode = \"{}\"\n",
            mount.host_path, mount.guest_path, mount.mode
        ));
    }

    result
}
