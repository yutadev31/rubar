mod app;
mod backend;
mod config;
mod render;
mod widget;

use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let config = config::Config::load()?;
    app::App::new(&config).run()?;
    Ok(())
}
