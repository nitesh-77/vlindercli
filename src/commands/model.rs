use std::path::Path;

use clap::Subcommand;

use vlindercli::catalog::OllamaCatalog;
use vlindercli::config::{registry_db_path, Config};
use vlindercli::domain::{Model, ModelCatalog, PersistentRegistry, Registry};

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

    /// List models from a catalog
    List {
        /// Catalog to query
        #[arg(short, long, default_value = "ollama")]
        catalog: String,

        /// Ollama endpoint (overrides config)
        #[arg(long)]
        endpoint: Option<String>,
    },

    /// List registered models (added via `model add`)
    Registered,

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
            let endpoint = endpoint.unwrap_or(config.ollama.endpoint);
            add(&name, &catalog, &endpoint)
        }
        ModelCommand::List { catalog, endpoint } => {
            let endpoint = endpoint.unwrap_or(config.ollama.endpoint);
            list(&catalog, &endpoint)
        }
        ModelCommand::Registered => registered(),
        ModelCommand::Remove { name } => remove(&name),
    }
}

fn add(name: &str, catalog: &str, endpoint: &str) {
    // If the name looks like a file path, load from manifest directly
    let model = if Path::new(name).extension().is_some_and(|ext| ext == "toml") {
        match Model::load(Path::new(name)) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Failed to load model manifest '{}': {}", name, e);
                return;
            }
        }
    } else {
        let catalog = match catalog {
            "ollama" => OllamaCatalog::new(endpoint),
            other => {
                eprintln!("Unknown catalog: {}. Supported: ollama", other);
                return;
            }
        };

        match catalog.resolve(name) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Failed to resolve model '{}': {}", name, e);
                return;
            }
        }
    };

    // Register through PersistentRegistry (writes to both disk and cache)
    let db_path = registry_db_path();
    let registry = match PersistentRegistry::open(&db_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to open registry: {}", e);
            return;
        }
    };

    if let Err(e) = registry.register_model(model.clone()) {
        eprintln!("Failed to register model: {}", e);
        return;
    }

    println!("Added model '{}':", model.name);
    println!("  Type:   {:?}", model.model_type);
    println!("  Engine: {:?}", model.engine);
    println!("  Path:   {}", model.model_path);
}

fn list(catalog: &str, endpoint: &str) {
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

fn registered() {
    let db_path = registry_db_path();

    if !db_path.exists() {
        println!("No models registered yet. Use 'vlinder model add <name>' to add models.");
        return;
    }

    let registry = match PersistentRegistry::open(&db_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to open registry: {}", e);
            return;
        }
    };

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

fn remove(name: &str) {
    let db_path = registry_db_path();

    let registry = match PersistentRegistry::open(&db_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to open registry: {}", e);
            return;
        }
    };

    match registry.delete_model(name) {
        Ok(true) => println!("Removed model '{}'", name),
        Ok(false) => println!("Model '{}' not found", name),
        Err(e) => eprintln!("Failed to remove model: {}", e),
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
        assert!(matches!(cli.cmd, ModelCommand::List { .. }));
    }
}
