use std::io::{self, Write};
use vlindercli::domain::{Model, Behavior};
use vlindercli::runtime::Runtime;

fn main() {
    let runtime = Runtime::new();

    // Hardcoded for now - will be configurable later
    let agent = runtime.spawn_agent(
        "reader-agent",
        "agents/reader-agent/target/wasm32-unknown-unknown/release/reader_agent.wasm",
        Model { path: "models/default.gguf".to_string() },
        Behavior { system_prompt: "You are helpful.".to_string() },
    ).expect("Failed to load agent");

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
