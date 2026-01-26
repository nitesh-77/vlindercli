use tracing_subscriber::EnvFilter;
use vlindercli::config::Config;

mod commands;

fn main() {
    let config = Config::load();
    init_tracing(&config);

    commands::run();
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
