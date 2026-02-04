use clap::Subcommand;

use vlindercli::catalog::OllamaCatalog;
use vlindercli::config::registry_db_path;
use vlindercli::domain::{ModelCatalog, RegistryRepository};
use vlindercli::storage::SqliteRegistryRepository;

#[derive(Subcommand, Debug, PartialEq)]
pub enum ModelCommand {
    /// Add a model from a catalog
    Add {
        /// Model name (e.g., "llama3", "nomic-embed-text")
        name: String,

        /// Catalog to use
        #[arg(short, long, default_value = "ollama")]
        catalog: String,

        /// Ollama endpoint (for ollama catalog)
        #[arg(long, default_value = "http://localhost:11434")]
        endpoint: String,
    },

    /// List models from a catalog
    List {
        /// Catalog to query
        #[arg(short, long, default_value = "ollama")]
        catalog: String,

        /// Ollama endpoint (for ollama catalog)
        #[arg(long, default_value = "http://localhost:11434")]
        endpoint: String,
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
    match cmd {
        ModelCommand::Add { name, catalog, endpoint } => add(&name, &catalog, &endpoint),
        ModelCommand::List { catalog, endpoint } => list(&catalog, &endpoint),
        ModelCommand::Registered => registered(),
        ModelCommand::Remove { name } => remove(&name),
    }
}

fn add(name: &str, catalog: &str, endpoint: &str) {
    let catalog = match catalog {
        "ollama" => OllamaCatalog::new(endpoint),
        other => {
            eprintln!("Unknown catalog: {}. Supported: ollama", other);
            return;
        }
    };

    let model = match catalog.resolve(name) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Failed to resolve model '{}': {}", name, e);
            return;
        }
    };

    // Persist to registry
    let db_path = registry_db_path();
    let repo = match SqliteRegistryRepository::open(&db_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to open registry: {}", e);
            return;
        }
    };

    if let Err(e) = repo.save_model(&model) {
        eprintln!("Failed to save model: {}", e);
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

    let repo = match SqliteRegistryRepository::open(&db_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to open registry: {}", e);
            return;
        }
    };

    match repo.load_models() {
        Ok(models) => {
            if models.is_empty() {
                println!("No models registered yet. Use 'vlinder model add <name>' to add models.");
                return;
            }
            println!("Registered models:");
            for model in models {
                println!("  {} ({:?}, {:?})", model.name, model.model_type, model.engine);
            }
        }
        Err(e) => {
            eprintln!("Failed to load models: {}", e);
        }
    }
}

fn remove(name: &str) {
    let db_path = registry_db_path();

    let repo = match SqliteRegistryRepository::open(&db_path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to open registry: {}", e);
            return;
        }
    };

    match repo.delete_model(name) {
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
