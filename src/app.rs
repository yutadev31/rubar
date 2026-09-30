use std::error::Error;

use crate::{
    backend::{self, Backend},
    render::BarRenderer,
};

pub struct App {
    backend: Box<dyn Backend>,
    renderer: BarRenderer,
}

impl App {
    pub fn new() -> Self {
        Self {
            backend: Box::new(backend::wayland::WaylandBackend::default()),
            renderer: BarRenderer::new(),
        }
    }

    pub fn run(&mut self) -> Result<(), Box<dyn Error>> {
        self.backend.run(&mut self.renderer)
    }
}
