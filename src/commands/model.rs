use std::path::Path;
use std::sync::Arc;

use clap::Subcommand;

use vlindercli::catalog::OllamaCatalog;
use vlindercli::config::{registry_db_path, Config};
use vlindercli::domain::{Model, ModelCatalog, PersistentRegistry, Registry};
use vlindercli::registry_service::{GrpcRegistryClient, ping_registry};

#[derive(Subcommand, Debug, PartialEq)]
pub enum ModelCommand {
    /// Add a model from a catalog
    Add {
        /// Model name (e.g., "llama3", "nomic-embed-text")
        name: String,

        /// Catalog to use
        #[arg(short, long, default_value = "ollama")]
        catalog: String,

        /// Ollama endpoint (overrides config)
        #[arg(long)]
        endpoint: Option<String>,
    },

    /// List models available in a catalog
    Available {
        /// Catalog to query
        #[arg(short, long, default_value = "ollama")]
        catalog: String,

        /// Ollama endpoint (overrides config)
        #[arg(long)]
        endpoint: Option<String>,
    },

    /// List registered models (added via `model add`)
    List,

    /// Remove a registered model
    Remove {
        /// Model name to remove
        name: String,
    },
}

pub fn execute(cmd: ModelCommand) {
    let config = Config::load();

    match cmd {
        ModelCommand::Add { name, catalog, endpoint } => {
            let endpoint = endpoint.unwrap_or_else(|| config.ollama.endpoint.clone());
            let model = resolve_model(&name, &catalog, &endpoint);
            let Some(model) = model else { return };

            let registry = open_registry(&config);
            let Some(registry) = registry else { return };

            if let Err(e) = registry.register_model(model.clone()) {
                eprintln!("Failed to register model: {}", e);
                return;
            }

            println!("Added model '{}':", model.name);
            println!("  Type:   {:?}", model.model_type);
            println!("  Engine: {:?}", model.engine);
            println!("  Path:   {}", model.model_path);
        }
        ModelCommand::Available { catalog, endpoint } => {
            let endpoint = endpoint.unwrap_or_else(|| config.ollama.endpoint.clone());
            list_available(&catalog, &endpoint)
        }
        ModelCommand::List => {
            let registry = open_registry(&config);
            let Some(registry) = registry else { return };

            let models = registry.get_models();
            if models.is_empty() {
                println!("No models registered yet. Use 'vlinder model add <name>' to add models.");
                return;
            }
            println!("Registered models:");
            for model in models {
                println!("  {} ({:?}, {:?})", model.name, model.model_type, model.engine);
            }
        }
        ModelCommand::Remove { name } => {
            let registry = open_registry(&config);
            let Some(registry) = registry else { return };

            match registry.delete_model(&name) {
                Ok(true) => println!("Removed model '{}'", name),
                Ok(false) => println!("Model '{}' not found", name),
                Err(e) => eprintln!("Failed to remove model: {}", e),
            }
        }
    }
}

/// Connect to the registry — gRPC in distributed mode, local SQLite otherwise.
fn open_registry(config: &Config) -> Option<Arc<dyn Registry>> {
    if config.distributed.enabled {
        let registry_addr = if config.distributed.registry_addr.starts_with("http://")
            || config.distributed.registry_addr.starts_with("https://") {
            config.distributed.registry_addr.clone()
        } else {
            format!("http://{}", config.distributed.registry_addr)
        };

        if ping_registry(&registry_addr).is_none() {
            eprintln!("Cannot reach registry at {}. Is the daemon running?", registry_addr);
            return None;
        }

        match GrpcRegistryClient::connect(&registry_addr) {
            Ok(client) => Some(Arc::new(client)),
            Err(e) => {
                eprintln!("Failed to connect to registry: {}", e);
                None
            }
        }
    } else {
        let db_path = registry_db_path();
        match PersistentRegistry::open(&db_path, config) {
            Ok(r) => Some(Arc::new(r)),
            Err(e) => {
                eprintln!("Failed to open registry: {}", e);
                None
            }
        }
    }
}

/// Resolve a model from name — either a TOML manifest path or a catalog lookup.
fn resolve_model(name: &str, catalog: &str, endpoint: &str) -> Option<Model> {
    if Path::new(name).extension().is_some_and(|ext| ext == "toml") {
        match Model::load(Path::new(name)) {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("Failed to load model manifest '{}': {}", name, e);
                None
            }
        }
    } else {
        let catalog = match catalog {
            "ollama" => OllamaCatalog::new(endpoint),
            other => {
                eprintln!("Unknown catalog: {}. Supported: ollama", other);
                return None;
            }
        };

        match catalog.resolve(name) {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("Failed to resolve model '{}': {}", name, e);
                None
            }
        }
    }
}

fn list_available(catalog: &str, endpoint: &str) {
    let catalog = match catalog {
        "ollama" => OllamaCatalog::new(endpoint),
        other => {
            eprintln!("Unknown catalog: {}. Supported: ollama", other);
            return;
        }
    };

    match catalog.list() {
        Ok(models) => {
            if models.is_empty() {
                println!("No models found. Pull models with: ollama pull <model>");
                return;
            }
            println!("Available models:");
            for model in models {
                let size = model.size.unwrap_or_default();
                println!("  {} ({})", model.name, size);
            }
        }
        Err(e) => {
            eprintln!("Failed to list models: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_add_command() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(subcommand)]
            cmd: ModelCommand,
        }

        let cli = TestCli::try_parse_from(["test", "add", "llama3"]).unwrap();
        match cli.cmd {
            ModelCommand::Add { name, catalog, .. } => {
                assert_eq!(name, "llama3");
                assert_eq!(catalog, "ollama");
            }
            _ => panic!("Expected Add command"),
        }
    }

    #[test]
    fn parses_list_command() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(subcommand)]
            cmd: ModelCommand,
        }

        let cli = TestCli::try_parse_from(["test", "list"]).unwrap();
        assert!(matches!(cli.cmd, ModelCommand::List));
    }

    #[test]
    fn parses_available_command() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(subcommand)]
            cmd: ModelCommand,
        }

        let cli = TestCli::try_parse_from(["test", "available"]).unwrap();
        assert!(matches!(cli.cmd, ModelCommand::Available { .. }));
    }
}
