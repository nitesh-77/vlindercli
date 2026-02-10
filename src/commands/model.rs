use std::path::Path;
use std::sync::Arc;

use clap::Subcommand;

use vlindercli::catalog::{OllamaCatalog, OpenRouterCatalog};
use vlindercli::config::Config;
use vlindercli::domain::{Model, ModelCatalog, Registry};
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
        /// Filter models by name (substring match)
        filter: Option<String>,

        /// Catalog to query (default: all)
        #[arg(short, long, default_value = "all")]
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
    let mut config = Config::load();

    match cmd {
        ModelCommand::Add { name, catalog, endpoint } => {
            if let Some(ep) = endpoint {
                config.ollama.endpoint = ep;
            }
            let model = resolve_model(&name, &catalog, &config);
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
        ModelCommand::Available { filter, ref catalog, endpoint } => {
            if let Some(ep) = endpoint {
                config.ollama.endpoint = ep;
            }
            list_available(catalog, filter.as_deref(), &config)
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

/// Connect to the registry via gRPC.
fn open_registry(config: &Config) -> Option<Arc<dyn Registry>> {
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
}

/// Resolve a model from name — either a TOML manifest path or a catalog lookup.
fn resolve_model(name: &str, catalog: &str, config: &Config) -> Option<Model> {
    if Path::new(name).extension().is_some_and(|ext| ext == "toml") {
        match Model::load(Path::new(name)) {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("Failed to load model manifest '{}': {}", name, e);
                None
            }
        }
    } else {
        let catalog: Box<dyn ModelCatalog> = match catalog {
            "ollama" => Box::new(OllamaCatalog::new(&config.ollama.endpoint)),
            "openrouter" => Box::new(OpenRouterCatalog::new(
                &config.openrouter.endpoint,
                &config.openrouter.api_key,
            )),
            other => {
                eprintln!("Unknown catalog: {}. Supported: ollama, openrouter", other);
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

/// Known catalogs in display order.
const CATALOGS: &[&str] = &["ollama", "openrouter"];

fn list_available(catalog_name: &str, filter: Option<&str>, config: &Config) {
    let catalogs: Vec<&str> = if catalog_name == "all" {
        CATALOGS.to_vec()
    } else if CATALOGS.contains(&catalog_name) {
        vec![catalog_name]
    } else {
        eprintln!("Unknown catalog: {}. Supported: all, ollama, openrouter", catalog_name);
        return;
    };

    let show_all = catalogs.len() > 1;
    println!("Tip: use --catalog <name> to pick a catalog, or pass a filter to narrow results.");
    println!();

    for name in &catalogs {
        // When showing all catalogs, skip OpenRouter if no API key is configured
        if show_all && *name == "openrouter" && config.openrouter.api_key.is_empty() {
            println!("openrouter: set VLINDER_OPENROUTER_API_KEY to browse OpenRouter models.");
            println!();
            continue;
        }

        let catalog: Box<dyn ModelCatalog> = match *name {
            "ollama" => Box::new(OllamaCatalog::new(&config.ollama.endpoint)),
            "openrouter" => Box::new(OpenRouterCatalog::new(
                &config.openrouter.endpoint,
                &config.openrouter.api_key,
            )),
            _ => continue,
        };

        match catalog.list() {
            Ok(models) => {
                let filtered: Vec<_> = if let Some(q) = filter {
                    let q = q.to_lowercase();
                    models.into_iter().filter(|m| m.name.to_lowercase().contains(&q)).collect()
                } else {
                    models
                };

                if filtered.is_empty() {
                    continue;
                }

                println!("{} ({} models):", name, filtered.len());
                for model in &filtered {
                    let size = model.size.as_deref().unwrap_or_default();
                    let detail = if size.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", size)
                    };
                    println!("  {}{}", model.name, detail);
                }
                println!();
            }
            Err(e) => {
                eprintln!("  {} — failed to list: {}", name, e);
            }
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
        match cli.cmd {
            ModelCommand::Available { catalog, filter, .. } => {
                assert_eq!(catalog, "all");
                assert_eq!(filter, None);
            }
            _ => panic!("Expected Available command"),
        }
    }

    #[test]
    fn parses_available_with_filter() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(subcommand)]
            cmd: ModelCommand,
        }

        let cli = TestCli::try_parse_from(["test", "available", "claude"]).unwrap();
        match cli.cmd {
            ModelCommand::Available { filter, catalog, .. } => {
                assert_eq!(filter, Some("claude".to_string()));
                assert_eq!(catalog, "all");
            }
            _ => panic!("Expected Available command"),
        }
    }

    #[test]
    fn parses_available_with_catalog_and_filter() {
        use clap::Parser;

        #[derive(Parser)]
        struct TestCli {
            #[command(subcommand)]
            cmd: ModelCommand,
        }

        let cli = TestCli::try_parse_from(["test", "available", "--catalog", "openrouter", "claude"]).unwrap();
        match cli.cmd {
            ModelCommand::Available { filter, catalog, .. } => {
                assert_eq!(filter, Some("claude".to_string()));
                assert_eq!(catalog, "openrouter");
            }
            _ => panic!("Expected Available command"),
        }
    }
}
