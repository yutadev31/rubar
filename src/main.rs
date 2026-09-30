mod app;
mod backend;
mod render;
mod widget;

use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    app::App::new().run()?;
    Ok(())
}
