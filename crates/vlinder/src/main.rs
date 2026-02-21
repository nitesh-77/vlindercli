mod cli;
mod commands;
mod config;
mod tracing_setup;

fn main() {
    let config = config::CliConfig::load();
    let filter = format!("warn,vlinder={}", config.logging.level);
    tracing_setup::init_tracing(&filter);

    commands::run();
}
