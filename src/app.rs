use std::error::Error;
use std::sync::Arc;

use crate::{
    config::Config,
    provider::wm::{self, WmClient},
    render::BarRenderer,
    widget::{
        WidgetGroups, battery::Battery, clock::Clock, tray::Tray, volume::Volume, window::Window,
        workspace::Workspace,
    },
};
use shell_surface::{
    Anchors, Backend, InputEvent, MouseButton as SurfaceMouseButton, OutputSelection, Shell, Size,
    SurfaceConfig, SurfaceId,
};

pub struct App<'a> {
    renderer: BarRenderer<'a>,
    widgets: WidgetGroups,
    _ipc_client: Arc<dyn WmClient>,
    surfaces: Vec<SurfaceConfig>,
}

impl<'a> App<'a> {
    pub fn new(config: &'a Config) -> Result<Self, Box<dyn Error>> {
        let ipc_client = wm::connect(&config.workspace.persistent_workspaces);
        let widgets = WidgetGroups {
            left: create_widgets(&config.modules.left, config, &ipc_client)?,
            center: create_widgets(&config.modules.center, config, &ipc_client)?,
            right: create_widgets(&config.modules.right, config, &ipc_client)?,
        };
        Ok(Self {
            renderer: BarRenderer::new(&config.style),
            widgets,
            _ipc_client: ipc_client,
            surfaces: vec![panel_surface_config(&config.style)],
        })
    }

    pub fn run(&mut self) -> Result<(), Box<dyn Error>> {
        let mut backend = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            Box::new(shell_surface::backend::wayland::WaylandBackend) as Box<dyn Backend>
        } else {
            Box::new(shell_surface::backend::x11::X11Backend) as Box<dyn Backend>
        };
        backend.run(self)
    }
}

impl Shell for App<'_> {
    fn surface_configs(&self) -> &[SurfaceConfig] {
        &self.surfaces
    }

    fn render(
        &mut self,
        _surface: SurfaceId,
        size: Size,
        output: Option<&str>,
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        self.widgets.set_monitor_name(output);
        Ok(self.renderer.render(
            size.width.max(1),
            size.height.max(1),
            &mut self.widgets.left,
            &mut self.widgets.center,
            &mut self.widgets.right,
        ))
    }

    fn handle_event(&mut self, _surface: SurfaceId, event: InputEvent) {
        match event {
            InputEvent::PointerButton {
                position,
                button,
                pressed: true,
                output,
            } => {
                self.widgets.set_monitor_name(output.as_deref());
                self.renderer.handle_click(
                    position.x,
                    map_mouse_button(button),
                    output.as_deref(),
                    &mut self.widgets.left,
                    &mut self.widgets.center,
                    &mut self.widgets.right,
                );
            }
            InputEvent::PointerScroll {
                position,
                delta_y,
                output,
                ..
            } if delta_y != 0.0 => {
                self.widgets.set_monitor_name(output.as_deref());
                self.renderer.handle_scroll(
                    position.x,
                    if delta_y < 0.0 {
                        crate::widget::ScrollDirection::Up
                    } else {
                        crate::widget::ScrollDirection::Down
                    },
                    &mut self.widgets.left,
                    &mut self.widgets.center,
                    &mut self.widgets.right,
                );
            }
            _ => {}
        }
    }

    fn take_redraw_request(&mut self) -> bool {
        // Widgets such as the clock and polling providers are refreshed while
        // rendering, so keep the shell backend's frame loop alive.
        true
    }
}

fn panel_surface_config(style: &crate::config::StyleConfig) -> SurfaceConfig {
    let mut surface = SurfaceConfig::new("rubar", Size::new(0, style.height.max(1)));
    surface.anchors = Anchors::TOP | Anchors::LEFT | Anchors::RIGHT;
    surface.exclusive_zone = style.height.max(1) as i32;
    surface.output = OutputSelection::All;
    surface
}

fn map_mouse_button(button: SurfaceMouseButton) -> crate::widget::MouseButton {
    match button {
        SurfaceMouseButton::Left => crate::widget::MouseButton::Left,
        SurfaceMouseButton::Middle => crate::widget::MouseButton::Middle,
        SurfaceMouseButton::Right => crate::widget::MouseButton::Right,
        SurfaceMouseButton::Other(button) => crate::widget::MouseButton::Other(button),
    }
}

fn create_widgets(
    names: &[String],
    config: &Config,
    ipc_client: &Arc<dyn WmClient>,
) -> Result<Vec<Box<dyn crate::widget::Widget>>, Box<dyn Error>> {
    names
        .iter()
        .map(|name| match name.as_str() {
            "clock" => Ok(Box::new(Clock::new(&config.clock)) as Box<dyn crate::widget::Widget>),
            "battery" => {
                Ok(Box::new(Battery::new(&config.battery)) as Box<dyn crate::widget::Widget>)
            }
            "volume" => Ok(Box::new(Volume::new(&config.volume)) as Box<dyn crate::widget::Widget>),
            "workspace" => Ok(
                Box::new(Workspace::new(&config.workspace, Arc::clone(ipc_client)))
                    as Box<dyn crate::widget::Widget>,
            ),
            "window" => Ok(
                Box::new(Window::new(&config.window, Arc::clone(ipc_client)))
                    as Box<dyn crate::widget::Widget>,
            ),
            "tray" => {
                Ok(Box::new(Tray::new(config.style.clone())) as Box<dyn crate::widget::Widget>)
            }
            _ => Err(format!("unknown module `{name}`").into()),
        })
        .collect()
}
