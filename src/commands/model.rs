use clap::Subcommand;

use vlindercli::catalog::OllamaCatalog;
use vlindercli::domain::ModelCatalog;

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
}

pub fn execute(cmd: ModelCommand) {
    match cmd {
        ModelCommand::Add { name, catalog, endpoint } => add(&name, &catalog, &endpoint),
        ModelCommand::List { catalog, endpoint } => list(&catalog, &endpoint),
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

    match catalog.resolve(name) {
        Ok(model) => {
            println!("Resolved model:");
            println!("  Name:   {}", model.name);
            println!("  Type:   {:?}", model.model_type);
            println!("  Engine: {:?}", model.engine);
            println!("  Path:   {}", model.model_path);
            println!();
            println!("Model ready. Registry integration pending.");
        }
        Err(e) => {
            eprintln!("Failed to resolve model '{}': {}", name, e);
        }
    }
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
