use vlindercli::config::Config;

mod cli;
mod commands;
mod tracing_setup;

fn main() {
    let config = Config::load();
    let filter = format!("warn,vlinder={}", config.logging.level);
    tracing_setup::init_tracing(&filter);

    commands::run();
}
