use std::error::Error;

use crate::{
    backend::{self, Backend},
    config::Config,
    render::BarRenderer,
    widget::{Widget, clock::Clock},
};

pub struct App<'a> {
    backend: Box<dyn Backend>,
    renderer: BarRenderer<'a>,
    widgets: Vec<Box<dyn Widget>>,
}

impl<'a> App<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self {
            backend: Box::new(backend::wayland::WaylandBackend::default()),
            renderer: BarRenderer::new(&config.style),
            widgets: vec![Box::new(Clock::new(&config.clock))],
        }
    }

    pub fn run(&mut self) -> Result<(), Box<dyn Error>> {
        self.backend.run(&mut self.renderer, &mut self.widgets)
    }
}
