use std::borrow::Cow;
use std::io::{stdout, Write};
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use reedline::{
    default_emacs_keybindings, Emacs, FileBackedHistory, KeyCode, KeyModifiers, Prompt,
    PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus, Reedline, ReedlineEvent,
    Signal,
};
use termimad::crossterm::style::Color;
use termimad::MadSkin;
use tracing_subscriber::EnvFilter;
use vlindercli::config::Config;
use vlindercli::domain::Agent;
use vlindercli::runtime::Runtime;

/// Custom prompt for the REPL
struct VlinderPrompt;

impl VlinderPrompt {
    fn new() -> Self {
        Self
    }
}

impl Prompt for VlinderPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _edit_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed("\x1b[34m❯\x1b[0m ") // Blue
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("  ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        Cow::Owned(format!(
            "({}reverse-search: {}) ",
            prefix, history_search.term
        ))
    }
}

/// Create a markdown skin for pretty output
fn create_skin() -> MadSkin {
    let mut skin = MadSkin::default();
    skin.set_headers_fg(Color::Cyan);
    skin.bold.set_fg(Color::Yellow);
    skin.italic.set_fg(Color::Magenta);
    skin.code_block.set_bg(Color::Rgb { r: 40, g: 40, b: 40 });
    skin
}

/// Show a spinner while processing
fn show_spinner(message: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    spinner.set_message(message.to_string());
    spinner.enable_steady_tick(Duration::from_millis(80));
    spinner
}

fn main() {
    // Load config
    let config = Config::load();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.tracing_filter())),
        )
        .with_target(false)
        .without_time()
        .init();

    // Route llama.cpp logs through tracing
    llama_cpp_2::send_logs_to_tracing(llama_cpp_2::LogOptions::default());

    let runtime = Runtime::new();
    let agent = Agent::load("pensieve").expect("Failed to load agent");

    // Set up reedline with history
    let history_path = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("vlinder_history.txt");

    let history = Box::new(
        FileBackedHistory::with_file(1000, history_path).expect("Failed to create history file"),
    );

    // Set up keybindings (emacs-style)
    let mut keybindings = default_emacs_keybindings();
    // Shift+Enter for newline (multiline input)
    keybindings.add_binding(
        KeyModifiers::SHIFT,
        KeyCode::Enter,
        ReedlineEvent::Edit(vec![reedline::EditCommand::InsertNewline]),
    );

    let edit_mode = Box::new(Emacs::new(keybindings));

    let mut line_editor = Reedline::create()
        .with_history(history)
        .with_edit_mode(edit_mode);

    let prompt = VlinderPrompt::new();
    let skin = create_skin();

    println!();
    skin.print_text("Welcome to **Vlinder**! Type your message or `exit` to quit.\n");

    loop {
        stdout().flush().unwrap();

        match line_editor.read_line(&prompt) {
            Ok(Signal::Success(input)) => {
                let input = input.trim();
                if input.is_empty() {
                    continue;
                }

                if input == "exit" || input == "quit" {
                    println!("Goodbye!");
                    break;
                }

                // Show spinner while processing
                let spinner = show_spinner("processing...");

                // Execute the agent
                let response = runtime.execute(&agent, input);

                // Stop spinner
                spinner.finish_and_clear();

                // Print response with markdown formatting
                println!();
                skin.print_text(&response);
                println!();
            }
            Ok(Signal::CtrlC) => {
                println!("^C");
                continue;
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
