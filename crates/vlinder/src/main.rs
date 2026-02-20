use vlindercli::config::Config;

mod commands;

fn main() {
    let config = Config::load();
    vlindercli::tracing_setup::init_tracing(&config);

    commands::run();
}
