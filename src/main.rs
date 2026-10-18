mod app;
mod color;
mod config;
mod module;
mod provider;
mod render;
mod widget;

use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = config::Config::load()?;
    app::App::new(&config)?.run()?;
    Ok(())
}
