use std::io::{stdout, Write};
use std::path::PathBuf;

use reedline::Signal;
use tracing_subscriber::EnvFilter;

use vlindercli::cli::{self, VlinderPrompt};
use vlindercli::config::Config;
use vlindercli::domain::Agent;
use vlindercli::runtime::Runtime;

fn main() {
    let config = Config::load();
    init_tracing(&config);

    let runtime = Runtime::new();

    // Parse -p flag for agent path, default to current directory
    let agent_path = parse_agent_path();
    let agent = Agent::load(&agent_path).expect("Failed to load agent");

    let mut editor = cli::create_editor();
    let prompt = VlinderPrompt;
    let skin = cli::create_skin();

    println!();
    skin.print_text("Welcome to **Vlinder**! Type your message or `exit` to quit.\n");

    loop {
        stdout().flush().unwrap();

        match editor.read_line(&prompt) {
            Ok(Signal::Success(input)) => {
                let input = input.trim();
                if input.is_empty() {
                    continue;
                }

                if input == "exit" || input == "quit" {
                    println!("Goodbye!");
                    break;
                }

                let spinner = cli::spinner("processing...");
                let response = runtime.execute(&agent, input);
                spinner.finish_and_clear();

                println!();
                skin.print_text(&response);
                println!();
            }
            Ok(Signal::CtrlC) => {
                println!("^C");
            }
            Ok(Signal::CtrlD) => {
                println!("Goodbye!");
                break;
            }
            Err(err) => {
                eprintln!("Error: {:?}", err);
                break;
            }
        }
    }
}

fn parse_agent_path() -> PathBuf {
    let args: Vec<String> = std::env::args().collect();

    // Look for -p <path> or --path <path>
    let mut i = 1;
    while i < args.len() {
        if (args[i] == "-p" || args[i] == "--path") && i + 1 < args.len() {
            return PathBuf::from(&args[i + 1]);
        }
        i += 1;
    }

    // Default to current directory
    std::env::current_dir().expect("Failed to get current directory")
}

fn init_tracing(config: &Config) {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.tracing_filter())),
        )
        .with_target(false)
        .without_time()
        .init();
}
