use std::sync::Arc;
use std::{
    collections::HashMap,
    error::Error,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::{
    config::Config,
    module::ModuleKind,
    provider::wm::{self, WmClient},
    render::BarRenderer,
    widget::{
        WidgetGroups, battery::Battery, clock::Clock, cpu::Cpu, memory::Memory, tray::Tray,
        volume::Volume, window::Window, workspace::Workspace,
    },
};
use shell_surface::{
    Anchors, Backend, InputEvent, MouseButton as SurfaceMouseButton, OutputSelection, Shell, Size,
    SurfaceConfig, SurfaceId,
};
use x11rb::connection::Connection;

pub struct App {
    renderer: BarRenderer,
    widgets: WidgetGroups,
    _ipc_client: Arc<dyn WmClient>,
    surfaces: Vec<SurfaceConfig>,
    revision: u64,
    announced_revision: u64,
    frames: HashMap<Option<String>, CachedFrame>,
    config_path: Option<PathBuf>,
    last_contents: Option<String>,
    last_config_check: Instant,
    restart_backend: bool,
    active_config: Config,
    dormant_trays: Vec<(String, Box<dyn crate::widget::Widget>)>,
    x11_width: Option<u32>,
}

struct CachedFrame {
    size: Size,
    revision: u64,
    pixels: Vec<u8>,
}

impl App {
    pub fn new(config: &Config) -> Result<Self, Box<dyn Error>> {
        tracing::debug!("initializing window manager client");
        let ipc_client = wm::connect(&config.workspace.persistent_workspaces);
        let widgets = WidgetGroups {
            left: create_widgets(&config.modules.left, config, &ipc_client)?,
            center: create_widgets(&config.modules.center, config, &ipc_client)?,
            right: create_widgets(&config.modules.right, config, &ipc_client)?,
        };
        let config_path = crate::config::config_path();
        let last_contents = config_path
            .as_ref()
            .and_then(|path| fs::read_to_string(path).ok());
        Ok(Self {
            renderer: BarRenderer::new(&config.style)?,
            widgets,
            _ipc_client: ipc_client,
            surfaces: vec![panel_surface_config(&config.style, None)],
            revision: 0,
            announced_revision: 0,
            frames: HashMap::new(),
            config_path,
            last_contents,
            last_config_check: Instant::now(),
            restart_backend: false,
            active_config: config.clone(),
            dormant_trays: Vec::new(),
            x11_width: None,
        })
    }

    pub fn run(&mut self) -> Result<(), Box<dyn Error>> {
        let mut backend = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            tracing::debug!("starting Wayland backend");
            Box::new(shell_surface::backend::wayland::WaylandBackend) as Box<dyn Backend>
        } else {
            tracing::debug!("starting X11 backend");
            // Unlike layer-shell, the X11 backend creates a 1px window for a zero width.
            let (connection, screen_number) = x11rb::connect(None)?;
            let width = u32::from(connection.setup().roots[screen_number].width_in_pixels);
            self.x11_width = Some(width);
            self.surfaces = vec![panel_surface_config(
                &self.active_config.style,
                self.x11_width,
            )];
            Box::new(shell_surface::backend::x11::X11Backend) as Box<dyn Backend>
        };
        loop {
            backend.run(self)?;
            if !self.restart_backend {
                return Ok(());
            }
            self.restart_backend = false;
        }
    }

    fn poll_changes(&mut self) {
        self.poll_config();
        for (_, tray) in &mut self.dormant_trays {
            tray.poll_changed();
        }
        if self.widgets.poll_changed() {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    fn poll_config(&mut self) {
        if self.last_config_check.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.last_config_check = Instant::now();
        let Some(path) = self.config_path.clone() else {
            return;
        };
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "could not read configuration");
                return;
            }
        };
        if contents == self.last_contents {
            return;
        }
        self.last_contents = contents.clone();
        let result = match contents {
            Some(contents) => Config::parse(&contents),
            None => Ok(Config::default()),
        };
        match result.and_then(|config| self.apply_config(&config)) {
            Ok(()) => tracing::info!(path = %path.display(), "configuration reloaded"),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "configuration reload failed; keeping current settings")
            }
        }
    }

    fn apply_config(&mut self, config: &Config) -> Result<(), Box<dyn Error>> {
        if &self.active_config == config {
            return Ok(());
        }
        let renderer = BarRenderer::new(&config.style)?;
        let ipc_client = if self.active_config.workspace.persistent_workspaces
            == config.workspace.persistent_workspaces
        {
            Arc::clone(&self._ipc_client)
        } else {
            wm::connect(&config.workspace.persistent_workspaces)
        };
        let available_names: Vec<String> = self
            .active_config
            .modules
            .left
            .iter()
            .chain(&self.active_config.modules.center)
            .chain(&self.active_config.modules.right)
            .cloned()
            .chain(self.dormant_trays.iter().map(|(name, _)| name.clone()))
            .collect();
        let mut reserved = vec![false; available_names.len()];
        let left = plan_widgets(
            &config.modules.left,
            config,
            &self.active_config,
            &ipc_client,
            &available_names,
            &mut reserved,
        )?;
        let center = plan_widgets(
            &config.modules.center,
            config,
            &self.active_config,
            &ipc_client,
            &available_names,
            &mut reserved,
        )?;
        let right = plan_widgets(
            &config.modules.right,
            config,
            &self.active_config,
            &ipc_client,
            &available_names,
            &mut reserved,
        )?;

        let old_widgets = std::mem::take(&mut self.widgets);
        let mut available: Vec<_> = old_widgets
            .left
            .into_iter()
            .chain(old_widgets.center)
            .chain(old_widgets.right)
            .chain(self.dormant_trays.drain(..).map(|(_, widget)| widget))
            .map(Some)
            .collect();
        let widgets = WidgetGroups {
            left: materialize_widgets(left, &mut available, config),
            center: materialize_widgets(center, &mut available, config),
            right: materialize_widgets(right, &mut available, config),
        };
        let surfaces = vec![panel_surface_config(&config.style, self.x11_width)];
        self.restart_backend = self.surfaces[0].size.height != surfaces[0].size.height;
        self.renderer = renderer;
        self._ipc_client = ipc_client;
        self.widgets = widgets;
        self.dormant_trays = available_names
            .into_iter()
            .zip(available)
            .filter_map(|(name, widget)| {
                (ModuleKind::parse(&name) == Some(ModuleKind::Tray))
                    .then_some(widget)
                    .flatten()
                    .map(|widget| (name, widget))
            })
            .collect();
        self.surfaces = surfaces;
        self.active_config = config.clone();
        self.frames.clear();
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }
}

impl Shell for App {
    fn surface_configs(&self) -> &[SurfaceConfig] {
        &self.surfaces
    }

    fn render(
        &mut self,
        _surface: SurfaceId,
        size: Size,
        output: Option<&str>,
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        self.poll_changes();
        let key = output.map(str::to_owned);
        if let Some(frame) = self.frames.get(&key)
            && frame.size == size
            && frame.revision == self.revision
        {
            return Ok(frame.pixels.clone());
        }
        tracing::trace!(width = size.width, height = size.height, output = ?output, "rendering bar");
        self.widgets.set_monitor_name(output);
        let pixels = self.renderer.render(
            size.width.max(1),
            size.height.max(1),
            output,
            &mut self.widgets.left,
            &mut self.widgets.center,
            &mut self.widgets.right,
        );
        self.frames.insert(
            key,
            CachedFrame {
                size,
                revision: self.revision,
                pixels: pixels.clone(),
            },
        );
        Ok(pixels)
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
                self.revision = self.revision.wrapping_add(1);
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
                    output.as_deref(),
                    &mut self.widgets.left,
                    &mut self.widgets.center,
                    &mut self.widgets.right,
                );
                self.revision = self.revision.wrapping_add(1);
            }
            _ => {}
        }
    }

    fn take_redraw_request(&mut self) -> bool {
        self.poll_changes();
        let changed = self.announced_revision != self.revision;
        self.announced_revision = self.revision;
        changed
    }

    fn should_close(&self) -> bool {
        self.restart_backend
    }
}

fn panel_surface_config(
    style: &crate::config::StyleConfig,
    x11_width: Option<u32>,
) -> SurfaceConfig {
    let mut surface = SurfaceConfig::new(
        "rubar",
        Size::new(x11_width.unwrap_or(0), style.height.max(1)),
    );
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
    tracing::debug!(widgets = ?names, "creating widget group");
    names
        .iter()
        .map(|name| create_widget(name, config, ipc_client))
        .collect()
}

enum WidgetPlan {
    Reuse(usize),
    New(Box<dyn crate::widget::Widget>),
}

fn plan_widgets(
    names: &[String],
    config: &Config,
    previous: &Config,
    ipc_client: &Arc<dyn WmClient>,
    available_names: &[String],
    reserved: &mut [bool],
) -> Result<Vec<WidgetPlan>, Box<dyn Error>> {
    names
        .iter()
        .map(|name| {
            if reusable(name, previous, config)
                && let Some(index) = available_names
                    .iter()
                    .enumerate()
                    .position(|(index, old_name)| !reserved[index] && old_name == name)
            {
                reserved[index] = true;
                return Ok(WidgetPlan::Reuse(index));
            }
            create_widget(name, config, ipc_client).map(WidgetPlan::New)
        })
        .collect()
}

fn materialize_widgets(
    plans: Vec<WidgetPlan>,
    available: &mut [Option<Box<dyn crate::widget::Widget>>],
    config: &Config,
) -> Vec<Box<dyn crate::widget::Widget>> {
    plans
        .into_iter()
        .map(|plan| match plan {
            WidgetPlan::Reuse(index) => {
                let mut widget = available[index].take().expect("reserved widget exists");
                widget.reload_config(config);
                widget
            }
            WidgetPlan::New(widget) => widget,
        })
        .collect()
}

fn reusable(name: &str, old: &Config, new: &Config) -> bool {
    match ModuleKind::parse(name) {
        Some(ModuleKind::Clock) => old.clock == new.clock,
        Some(ModuleKind::Battery) => old.battery == new.battery,
        Some(ModuleKind::Memory) => old.memory == new.memory,
        Some(ModuleKind::Cpu) => old.cpu == new.cpu,
        Some(ModuleKind::Volume) => old.volume == new.volume,
        Some(ModuleKind::Workspace) => old.workspace == new.workspace,
        Some(ModuleKind::Window) => {
            old.window == new.window
                && old.workspace.persistent_workspaces == new.workspace.persistent_workspaces
        }
        Some(ModuleKind::Tray) => old.tray.provider == new.tray.provider,
        None => false,
    }
}

fn create_widget(
    name: &str,
    config: &Config,
    ipc_client: &Arc<dyn WmClient>,
) -> Result<Box<dyn crate::widget::Widget>, Box<dyn Error>> {
    let kind = ModuleKind::parse(name).ok_or_else(|| format!("unknown module `{name}`"))?;
    match kind {
        ModuleKind::Clock => {
            Ok(Box::new(Clock::new(&config.clock)) as Box<dyn crate::widget::Widget>)
        }
        ModuleKind::Battery => {
            Ok(Box::new(Battery::new(&config.battery)) as Box<dyn crate::widget::Widget>)
        }
        ModuleKind::Memory => {
            Ok(Box::new(Memory::new(&config.memory)) as Box<dyn crate::widget::Widget>)
        }
        ModuleKind::Cpu => Ok(Box::new(Cpu::new(&config.cpu)) as Box<dyn crate::widget::Widget>),
        ModuleKind::Volume => {
            Ok(Box::new(Volume::new(&config.volume)) as Box<dyn crate::widget::Widget>)
        }
        ModuleKind::Workspace => Ok(Box::new(Workspace::new(
            &config.workspace,
            Arc::clone(ipc_client),
        )) as Box<dyn crate::widget::Widget>),
        ModuleKind::Window => Ok(
            Box::new(Window::new(&config.window, Arc::clone(ipc_client)))
                as Box<dyn crate::widget::Widget>,
        ),
        ModuleKind::Tray => Ok(Box::new(Tray::new(&config.tray, config.style.clone())?)
            as Box<dyn crate::widget::Widget>),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_reload_preserves_existing_widgets() {
        let mut config = Config::default();
        config.modules.left.push("clock".to_string());
        let mut app = App::new(&config).unwrap();

        let mut invalid = config.clone();
        invalid.modules.left = vec!["tray".to_string()];
        invalid.tray.provider = "missing".to_string();

        assert!(app.apply_config(&invalid).is_err());
        assert_eq!(app.widgets.left.len(), 1);
        assert_eq!(app.active_config, config);
    }
}
