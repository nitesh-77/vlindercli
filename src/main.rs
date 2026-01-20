use std::io::{self, Write};
use tracing_subscriber::EnvFilter;
use vlindercli::config::Config;
use vlindercli::domain::Agent;
use vlindercli::runtime::Runtime;

fn main() {
    // Load config from .vlinder/config.toml (or defaults)
    let config = Config::load();

    // Initialize tracing
    // - Levels from config (default: app=info, llama=error)
    // - Override with RUST_LOG env var for debugging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.tracing_filter()))
        )
        .with_target(false)  // Hide module paths for cleaner output
        .without_time()      // CLI doesn't need timestamps
        .init();

    // Route llama.cpp logs through tracing (must be called after subscriber init)
    llama_cpp_2::send_logs_to_tracing(llama_cpp_2::LogOptions::default());

    let runtime = Runtime::new();
    let agent = Agent::load("pensieve").expect("Failed to load agent");

    loop {
        print!("> ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        if io::stdin().read_line(&mut input).unwrap() == 0 {
            break; // EOF
        }

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        let response = runtime.execute(&agent, input);
        println!("{}", response);
    }
}
