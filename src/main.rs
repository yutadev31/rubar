mod app;
mod backend;
mod render;

use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    app::App::new().run()?;
    Ok(())
}
