use std::io::{stdout, Write};

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
    let agent = Agent::load("pensieve").expect("Failed to load agent");

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
