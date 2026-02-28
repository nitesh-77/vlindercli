use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Subcommand, ValueEnum};

use crate::config::CliConfig;
use vlinder_core::domain::{Agent, AgentManifest, Harness, Registry};

use super::connect::{connect_harness, connect_registry, open_dag_store, read_latest_state};
use super::repl;

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum Language {
    Python,
    Golang,
    Js,
    Ts,
    Java,
    Dotnet,
}

impl Language {
    fn repo_suffix(&self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::Golang => "golang",
            Language::Js => "js",
            Language::Ts => "ts",
            Language::Java => "java",
            Language::Dotnet => "dotnet",
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.repo_suffix())
    }
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum AgentCommand {
    /// Deploy an agent manifest to the registry
    Deploy {
        /// Path to agent directory (default: current directory)
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
    /// Run a deployed agent interactively
    Run {
        /// Agent name
        name: String,
    },
    /// List deployed agents
    List,
    /// Show details for a deployed agent
    Get {
        /// Agent name
        name: String,
    },
    /// Delete a deployed agent
    Delete {
        /// Agent name
        name: String,
    },
    /// Create a new agent from a template
    New {
        /// Template language
        language: Language,
        /// Agent name (becomes the directory name)
        name: String,
    },
}

pub fn execute(cmd: AgentCommand) {
    match cmd {
        AgentCommand::Deploy { path } => deploy(path),
        AgentCommand::Run { name } => run(&name),
        AgentCommand::List => list(),
        AgentCommand::Get { name } => get(&name),
        AgentCommand::Delete { name } => delete(&name),
        AgentCommand::New { language, name } => scaffold(&language, &name),
    }
}

fn deploy(path: Option<PathBuf>) {
    let config = CliConfig::load();
    let agent_path = path.unwrap_or_else(|| {
        std::env::current_dir().expect("Failed to get current directory")
    });

    let absolute_path = agent_path
        .canonicalize()
        .expect("Failed to resolve agent path");

    let registry = connect_registry(&config);
    let agent = deploy_agent_from_path(&absolute_path, &*registry);

    println!("Deployed: {} ({})", agent.name, agent.id);
}

/// Load an agent manifest from a directory, auto-deploy its models, and register it.
///
/// Shared by `agent deploy` and `fleet deploy`.
pub(super) fn deploy_agent_from_path(agent_dir: &Path, registry: &dyn Registry) -> Agent {
    let manifest_path = agent_dir.join("agent.toml");
    let manifest = AgentManifest::load(&manifest_path)
        .unwrap_or_else(|e| {
            eprintln!("Failed to load agent manifest: {:?}", e);
            std::process::exit(1);
        });

    // Auto-deploy models from <agent_dir>/models/<name>.toml
    match auto_deploy_models(agent_dir, &manifest, registry) {
        Ok(deployed) => {
            for name in &deployed {
                println!("  Model: {} (auto-deployed)", name);
            }
        }
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }

    registry.register_manifest(manifest)
        .unwrap_or_else(|e| {
            eprintln!("Failed to deploy agent: {}", e);
            std::process::exit(1);
        })
}

fn run(name: &str) {
    let config = CliConfig::load();
    let registry = connect_registry(&config);

    // Look up already-deployed agent by name (ADR 103)
    let agent = match registry.get_agent_by_name(name) {
        Some(a) => a,
        None => {
            eprintln!("Agent '{}' not found — deploy it first with: vlinder agent deploy", name);
            std::process::exit(1);
        }
    };

    let agent_id = agent.id.clone();

    // Connect to harness via gRPC — the daemon's harness worker owns the
    // queue and registry connection. The CLI is now a pure gRPC client.
    let mut harness = connect_harness(&config);

    // Start conversation session (ADR 054, ADR 070)
    harness.start_session(name);

    // Read state from state service (ADR 079)
    apply_latest_state(&config, &mut *harness, name);

    // Run REPL with synchronous run_agent (ADR 092)
    repl::run(|input| {
        match harness.run_agent(&agent_id, input) {
            Ok(result) => result,
            Err(e) => format!("[error] {}", e),
        }
    });
}

/// Read the latest state for an agent from the DAG store (ADR 079)
/// and initialize the harness with it (state continuity across sessions).
fn apply_latest_state(config: &CliConfig, harness: &mut dyn Harness, agent_name: &str) {
    let store = open_dag_store(config);
    let Some(store) = store else { return };
    if let Some(state) = read_latest_state(store.as_ref(), agent_name) {
        println!("Resuming from state {}…", &state[..8.min(state.len())]);
        harness.set_initial_state(state);
    }
}

fn list() {
    let config = CliConfig::load();
    let registry = open_registry(&config);
    let Some(registry) = registry else { return };

    let agents = registry.get_agents();
    if agents.is_empty() {
        println!("No agents deployed.");
        return;
    }
    println!("Deployed agents:");
    for agent in agents {
        println!("  {} ({}, {})", agent.name, agent.runtime.as_str(), agent.executable);
    }
}

fn get(name: &str) {
    let config = CliConfig::load();
    let registry = open_registry(&config);
    let Some(registry) = registry else { return };

    let Some(agent) = registry.get_agent_by_name(name) else {
        eprintln!("Agent '{}' not found", name);
        return;
    };

    println!("Name:        {}", agent.name);
    println!("Runtime:     {}", agent.runtime.as_str());
    println!("Executable:  {}", agent.executable);
    println!("Description: {}", agent.description);
    if let Some(ref storage) = agent.object_storage {
        println!("Object storage: {}", storage);
    }
    if let Some(ref storage) = agent.vector_storage {
        println!("Vector storage: {}", storage);
    }
    if !agent.requirements.models.is_empty() {
        println!("Models:");
        for (alias, name) in &agent.requirements.models {
            println!("  {} -> {}", alias, name);
        }
    }
}

fn delete(name: &str) {
    let config = CliConfig::load();
    let registry = open_registry(&config);
    let Some(registry) = registry else { return };

    match registry.delete_agent(name) {
        Ok(true) => println!("Deleted agent '{}'", name),
        Ok(false) => eprintln!("Agent '{}' not found", name),
        Err(e) => eprintln!("Error: {}", e),
    }
}

fn open_registry(config: &CliConfig) -> Option<Arc<dyn Registry>> {
    super::connect::open_registry(config)
}

/// Scaffold a new agent project from a GitHub template.
fn scaffold(language: &Language, name: &str) {
    let target = PathBuf::from(name);
    if target.exists() {
        eprintln!("Error: directory '{}' already exists.", name);
        std::process::exit(1);
    }

    let suffix = language.repo_suffix();
    let url = format!(
        "https://github.com/vlindercli/vlinder-agent-{}/archive/refs/heads/main.tar.gz",
        suffix
    );

    println!("Downloading {} template…", language);

    // Download tarball to a temp file
    let tarball = match download_tarball(&url) {
        Ok(path) => path,
        Err(e) => {
            eprintln!("Error: failed to download template: {}", e);
            eprintln!("Check your internet connection and try again.");
            std::process::exit(1);
        }
    };

    // Create target directory
    if let Err(e) = std::fs::create_dir_all(&target) {
        eprintln!("Error: failed to create directory '{}': {}", name, e);
        std::process::exit(1);
    }

    // Extract tarball into target directory
    if let Err(e) = extract_tarball(&tarball, &target, suffix) {
        eprintln!("Error: failed to extract template: {}", e);
        // Clean up the empty directory we created
        let _ = std::fs::remove_dir_all(&target);
        std::process::exit(1);
    }

    // Clean up temp file
    let _ = std::fs::remove_file(&tarball);

    // Patch agent name in agent.toml and build.sh
    patch_file(&target.join("agent.toml"), "hello-agent", name);
    patch_file(&target.join("build.sh"), "hello-agent", name);

    println!("Created agent '{}' from {} template.", name, language);
    println!();
    println!("Next steps:");
    println!("  cd {}", name);
    println!("  ./build.sh");
    println!("  vlinder agent deploy");
    println!("  vlinder agent run {}", name);
}

/// Download a URL to a temporary file and return the path.
fn download_tarball(url: &str) -> Result<PathBuf, String> {
    let mut resp = ureq::get(url)
        .call()
        .map_err(|e| format!("{}", e))?;

    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join("vlinder-template.tar.gz");

    let mut file = std::fs::File::create(&tmp_path)
        .map_err(|e| format!("failed to create temp file: {}", e))?;

    std::io::copy(&mut resp.body_mut().as_reader(), &mut file)
        .map_err(|e| format!("failed to write temp file: {}", e))?;

    Ok(tmp_path)
}

/// Extract a tarball, moving files from the nested directory up into target.
fn extract_tarball(tarball: &PathBuf, target: &PathBuf, suffix: &str) -> Result<(), String> {
    // tar xzf into target, then move contents from nested dir up
    let status = std::process::Command::new("tar")
        .args(["xzf", &tarball.display().to_string(), "-C", &target.display().to_string()])
        .status()
        .map_err(|e| format!("failed to run tar: {}", e))?;

    if !status.success() {
        return Err("tar extraction failed".to_string());
    }

    // GitHub archives extract to vlinder-agent-{suffix}-main/
    let nested = target.join(format!("vlinder-agent-{}-main", suffix));
    if nested.exists() {
        // Move all files from nested dir up into target
        let entries = std::fs::read_dir(&nested)
            .map_err(|e| format!("failed to read extracted directory: {}", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("failed to read entry: {}", e))?;
            let dest = target.join(entry.file_name());
            std::fs::rename(entry.path(), dest)
                .map_err(|e| format!("failed to move file: {}", e))?;
        }

        std::fs::remove_dir_all(&nested)
            .map_err(|e| format!("failed to clean up nested directory: {}", e))?;
    }

    Ok(())
}

/// Replace all occurrences of `from` with `to` in a file.
fn patch_file(path: &PathBuf, from: &str, to: &str) {
    if let Ok(content) = std::fs::read_to_string(path) {
        let patched = content.replace(from, to);
        let _ = std::fs::write(path, patched);
    }
}

/// Auto-deploy models found in `<agent_dir>/models/` for each model in the manifest.
///
/// For each unique model name in `manifest.requirements.models`, checks if a
/// corresponding `<agent_dir>/models/<name>.toml` exists. If so, loads and registers it.
/// Models without a `.toml` file are skipped (they must already be registered).
///
/// Returns the names of models that were auto-deployed.
pub(super) fn auto_deploy_models(
    agent_dir: &Path,
    manifest: &AgentManifest,
    registry: &dyn Registry,
) -> Result<Vec<String>, String> {
    let models_dir = agent_dir.join("models");
    let model_names: std::collections::HashSet<&str> = manifest
        .requirements
        .models
        .values()
        .map(|s| s.as_str())
        .collect();

    let mut deployed = Vec::new();
    for model_name in &model_names {
        let model_toml = models_dir.join(format!("{}.toml", model_name));
        if model_toml.exists() {
            let model = super::model::load_and_register_model(&model_toml, registry)?;
            deployed.push(model.name);
        }
    }
    Ok(deployed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vlinder_core::domain::{InMemoryRegistry, InMemorySecretStore, Provider};
    use vlinder_core::domain::RequirementsConfig;

    fn test_registry() -> Arc<InMemoryRegistry> {
        let store = Arc::new(InMemorySecretStore::new());
        Arc::new(InMemoryRegistry::new(store))
    }

    fn write_model_toml(dir: &Path, name: &str, provider: &str, model_path: &str) {
        let models_dir = dir.join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let content = format!(
            "name = \"{name}\"\ntype = \"inference\"\nprovider = \"{provider}\"\nmodel_path = \"{model_path}\"\n"
        );
        std::fs::write(models_dir.join(format!("{}.toml", name)), content).unwrap();
    }

    fn manifest_with_models(models: Vec<(&str, &str)>) -> AgentManifest {
        let mut model_map = std::collections::HashMap::new();
        for (alias, name) in models {
            model_map.insert(alias.to_string(), name.to_string());
        }
        AgentManifest {
            name: "test-agent".to_string(),
            description: "test".to_string(),
            source: None,
            runtime: "container".to_string(),
            executable: "localhost/test:latest".to_string(),
            requirements: RequirementsConfig {
                models: model_map,
                services: std::collections::HashMap::new(),
                mounts: std::collections::HashMap::new(),
            },
            prompts: None,
            object_storage: None,
            vector_storage: None,
        }
    }

    #[test]
    fn auto_deploy_registers_discovered_models() {
        let dir = tempfile::tempdir().unwrap();
        write_model_toml(dir.path(), "claude-sonnet", "openrouter", "openrouter://anthropic/claude-sonnet-4");

        let registry = test_registry();
        registry.register_inference_engine(Provider::OpenRouter);

        let manifest = manifest_with_models(vec![("inference_model", "claude-sonnet")]);
        let deployed = auto_deploy_models(dir.path(), &manifest, &*registry).unwrap();

        assert_eq!(deployed, vec!["claude-sonnet"]);
        assert!(registry.get_model("claude-sonnet").is_some());
    }

    #[test]
    fn auto_deploy_skips_models_without_toml() {
        let dir = tempfile::tempdir().unwrap();
        // No models/ directory at all

        let registry = test_registry();
        let manifest = manifest_with_models(vec![("inference_model", "already-registered")]);
        let deployed = auto_deploy_models(dir.path(), &manifest, &*registry).unwrap();

        assert!(deployed.is_empty());
    }

    #[test]
    fn auto_deploy_deduplicates_model_names() {
        let dir = tempfile::tempdir().unwrap();
        write_model_toml(dir.path(), "claude-sonnet", "openrouter", "openrouter://anthropic/claude-sonnet-4");

        let registry = test_registry();
        registry.register_inference_engine(Provider::OpenRouter);

        // Two aliases pointing to the same model
        let manifest = manifest_with_models(vec![
            ("inference_model", "claude-sonnet"),
            ("summary_model", "claude-sonnet"),
        ]);
        let deployed = auto_deploy_models(dir.path(), &manifest, &*registry).unwrap();

        // Registered only once
        assert_eq!(deployed.len(), 1);
        assert_eq!(registry.get_models().len(), 1);
    }

    #[test]
    fn auto_deploy_handles_multiple_models() {
        let dir = tempfile::tempdir().unwrap();
        write_model_toml(dir.path(), "claude-sonnet", "openrouter", "openrouter://anthropic/claude-sonnet-4");

        // Write an embedding model TOML
        let models_dir = dir.path().join("models");
        std::fs::write(
            models_dir.join("nomic-embed.toml"),
            "name = \"nomic-embed\"\ntype = \"embedding\"\nprovider = \"ollama\"\nmodel_path = \"ollama://localhost:11434/nomic-embed-text:latest\"\n",
        ).unwrap();

        let registry = test_registry();
        registry.register_inference_engine(Provider::OpenRouter);
        registry.register_embedding_engine(Provider::Ollama);

        let manifest = manifest_with_models(vec![
            ("inference_model", "claude-sonnet"),
            ("embedding_model", "nomic-embed"),
        ]);
        let mut deployed = auto_deploy_models(dir.path(), &manifest, &*registry).unwrap();
        deployed.sort();

        assert_eq!(deployed, vec!["claude-sonnet", "nomic-embed"]);
        assert_eq!(registry.get_models().len(), 2);
    }

    #[test]
    fn auto_deploy_with_no_required_models() {
        let dir = tempfile::tempdir().unwrap();
        let registry = test_registry();

        let manifest = manifest_with_models(vec![]);
        let deployed = auto_deploy_models(dir.path(), &manifest, &*registry).unwrap();

        assert!(deployed.is_empty());
    }

    #[test]
    fn auto_deploy_propagates_registration_error() {
        let dir = tempfile::tempdir().unwrap();
        write_model_toml(dir.path(), "claude-sonnet", "openrouter", "openrouter://anthropic/claude-sonnet-4");

        let registry = test_registry();
        // Don't register OpenRouter engine — registration should fail

        let manifest = manifest_with_models(vec![("inference_model", "claude-sonnet")]);
        let result = auto_deploy_models(dir.path(), &manifest, &*registry);

        assert!(result.is_err());
    }
}
