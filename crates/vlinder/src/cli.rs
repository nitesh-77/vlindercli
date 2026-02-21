//! CLI styling and REPL components
//!
//! Separates terminal UI concerns from application logic.

use std::borrow::Cow;
use std::path::PathBuf;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use reedline::{
    default_emacs_keybindings, Emacs, FileBackedHistory, KeyCode, KeyModifiers, Prompt,
    PromptEditMode, PromptHistorySearch, PromptHistorySearchStatus, Reedline, ReedlineEvent,
};
use termimad::crossterm::style::Color;
use termimad::MadSkin;

/// Custom prompt for the REPL
pub struct VlinderPrompt;

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
pub fn create_skin() -> MadSkin {
    let mut skin = MadSkin::default();
    skin.set_headers_fg(Color::Cyan);
    skin.bold.set_fg(Color::Yellow);
    skin.italic.set_fg(Color::Magenta);
    skin.code_block.set_bg(Color::Rgb { r: 40, g: 40, b: 40 });
    skin
}

/// Show a spinner while processing
pub fn spinner(message: &str) -> ProgressBar {
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

/// Create a configured line editor with history and keybindings
pub fn create_editor() -> Reedline {
    let history_path = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("vlinder_history.txt");

    let history = Box::new(
        FileBackedHistory::with_file(1000, history_path).expect("Failed to create history file"),
    );

    let mut keybindings = default_emacs_keybindings();
    // Shift+Enter for newline (multiline input)
    keybindings.add_binding(
        KeyModifiers::SHIFT,
        KeyCode::Enter,
        ReedlineEvent::Edit(vec![reedline::EditCommand::InsertNewline]),
    );

    let edit_mode = Box::new(Emacs::new(keybindings));

    Reedline::create()
        .with_history(history)
        .with_edit_mode(edit_mode)
}
