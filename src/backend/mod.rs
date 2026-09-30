use std::error::Error;

use crate::render::BarRenderer;

pub mod wayland;

/// Window-system integration boundary. An X11 backend can implement this
/// trait without changing rendering or application startup code.
pub trait Backend {
    fn run(&mut self, renderer: &mut BarRenderer) -> Result<(), Box<dyn Error>>;
}
