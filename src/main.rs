mod app;
mod backend;
mod config;
mod render;
mod widget;

use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    app::App::new(config::Config::load()?).run()?;
    Ok(())
}
