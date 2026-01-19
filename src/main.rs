use std::io::{self, Write};
use vlindercli::domain::{Agent, Model};
use vlindercli::runtime::Runtime;

fn main() {
    let runtime = Runtime::new();
    let agent = Agent::load("pensieve", vec![
        Model { name: "phi3".to_string() },
    ]).expect("Failed to load agent");

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
