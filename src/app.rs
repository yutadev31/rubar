use std::env;
use std::error::Error;

use crate::{
    backend::{self, Backend},
    config::Config,
    render::BarRenderer,
    widget::{WidgetGroups, battery::Battery, clock::Clock, volume::Volume, workspace::Workspace},
};

pub struct App<'a> {
    backend: Box<dyn Backend>,
    renderer: BarRenderer<'a>,
    widgets: WidgetGroups,
}

impl<'a> App<'a> {
    pub fn new(config: &'a Config) -> Result<Self, Box<dyn Error>> {
        let right = config.modules.right.clone();
        let widgets = WidgetGroups {
            left: create_widgets(&config.modules.left, config)?,
            center: create_widgets(&config.modules.center, config)?,
            right: create_widgets(&right, config)?,
        };
        let backend: Box<dyn Backend> = if env::var_os("WAYLAND_DISPLAY").is_some() {
            Box::new(backend::wayland::WaylandBackend::default())
        } else {
            Box::new(backend::x11::X11Backend::default())
        };
        Ok(Self {
            backend,
            renderer: BarRenderer::new(&config.style),
            widgets,
        })
    }

    pub fn run(&mut self) -> Result<(), Box<dyn Error>> {
        self.backend.run(&mut self.renderer, &mut self.widgets)
    }
}

fn create_widgets(
    names: &[String],
    config: &Config,
) -> Result<Vec<Box<dyn crate::widget::Widget>>, Box<dyn Error>> {
    names
        .iter()
        .map(|name| match name.as_str() {
            "clock" => Ok(Box::new(Clock::new(&config.clock)) as Box<dyn crate::widget::Widget>),
            "battery" => {
                Ok(Box::new(Battery::new(&config.battery)) as Box<dyn crate::widget::Widget>)
            }
            "volume" => Ok(Box::new(Volume::new(&config.volume)) as Box<dyn crate::widget::Widget>),
            "workspace" => {
                Ok(Box::new(Workspace::new(&config.workspace)) as Box<dyn crate::widget::Widget>)
            }
            _ => Err(format!("unknown module `{name}`").into()),
        })
        .collect()
}
