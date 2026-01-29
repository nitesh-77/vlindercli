use std::io::{stdout, Write};

use reedline::Signal;

use vlindercli::cli::{self, VlinderPrompt};

/// Run an interactive REPL loop.
///
/// The `process` function is called for each user input and returns the response to display.
pub fn run<F>(mut process: F)
where
    F: FnMut(&str) -> String,
{
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
                let response = process(input);
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
