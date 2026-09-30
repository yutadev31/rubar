use std::error::Error;

use crate::{
    backend::{self, Backend},
    render::BarRenderer,
    widget::{Widget, clock::Clock},
};

pub struct App {
    backend: Box<dyn Backend>,
    renderer: BarRenderer,
    widgets: Vec<Box<dyn Widget>>,
}

impl App {
    pub fn new() -> Self {
        Self {
            backend: Box::new(backend::wayland::WaylandBackend::default()),
            renderer: BarRenderer::new(),
            widgets: vec![Box::new(Clock::default())],
        }
    }

    pub fn run(&mut self) -> Result<(), Box<dyn Error>> {
        self.backend.run(&mut self.renderer, &mut self.widgets)
    }
}
