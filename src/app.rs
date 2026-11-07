use std::{
    collections::HashMap,
    error::Error,
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use shell_surface::{
    Anchors, Backend, InputEvent, MouseButton as SurfaceMouseButton, OutputSelection, Shell, Size,
    SurfaceConfig, SurfaceId,
};
use x11rb::connection::Connection;

use crate::{
    config::Config,
    provider::wm::{self, WmClient},
    render::BarRenderer,
    widget::{
        WidgetGroups, WidgetKind, battery::Battery, clock::Clock, cpu::Cpu, memory::Memory,
        process::Process, tray::Tray, volume::Volume, window::Window, workspace::Workspace,
    },
};

pub struct App {
    renderer: BarRenderer,
    widgets: WidgetGroups,
    ipc_client: Arc<dyn WmClient>,
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
            left: create_widgets(&config.widgets.left, config, &ipc_client)?,
            center: create_widgets(&config.widgets.center, config, &ipc_client)?,
            right: create_widgets(&config.widgets.right, config, &ipc_client)?,
        };
        let config_path = crate::config::config_path();
        let last_contents = config_path
            .as_ref()
            .and_then(|path| fs::read_to_string(path).ok());

        Ok(Self {
            renderer: BarRenderer::new(&config.style)?,
            widgets,
            ipc_client,
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

        self.last_contents.clone_from(&contents);

        let result = match contents {
            Some(contents) => Config::parse(&contents),
            None => Ok(Config::default()),
        };

        match result.and_then(|config| self.apply_config(&config)) {
            Ok(()) => tracing::info!(path = %path.display(), "configuration reloaded"),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "configuration reload failed; keeping current settings");
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
            Arc::clone(&self.ipc_client)
        } else {
            wm::connect(&config.workspace.persistent_workspaces)
        };

        let available_names: Vec<String> = self
            .active_config
            .widgets
            .left
            .iter()
            .chain(&self.active_config.widgets.center)
            .chain(&self.active_config.widgets.right)
            .cloned()
            .chain(self.dormant_trays.iter().map(|(name, _)| name.clone()))
            .collect();
        let mut reserved = vec![false; available_names.len()];

        let left = plan_widgets(
            &config.widgets.left,
            config,
            &self.active_config,
            &ipc_client,
            &available_names,
            &mut reserved,
        )?;
        let center = plan_widgets(
            &config.widgets.center,
            config,
            &self.active_config,
            &ipc_client,
            &available_names,
            &mut reserved,
        )?;

        let right = plan_widgets(
            &config.widgets.right,
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
        self.ipc_client = ipc_client;
        self.widgets = widgets;
        let same_tray_provider = self.active_config.tray.provider == config.tray.provider;
        self.dormant_trays = available_names
            .into_iter()
            .zip(available)
            .filter_map(|(name, widget)| {
                (same_tray_provider && WidgetKind::parse(&name) == Some(WidgetKind::Tray))
                    .then_some(widget)
                    .flatten()
                    .map(|widget| (name, widget))
            })
            .collect();

        self.surfaces = surfaces;
        self.active_config.clone_from(config);
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
        surface: SurfaceId,
        size: Size,
        output: Option<&str>,
    ) -> Result<Vec<u8>, shell_surface::Error> {
        (|| -> Result<Vec<u8>, Box<dyn Error>> {
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
        )?;
        self.frames.insert(
            key,
            CachedFrame {
                size,
                revision: self.revision,
                pixels: pixels.clone(),
            },
        );
        Ok(pixels)
        })().map_err(|source| shell_surface::Error::Render { surface, source })
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

                let direction = if delta_y < 0.0 {
                    crate::widget::ScrollDirection::Up
                } else {
                    crate::widget::ScrollDirection::Down
                };

                self.renderer.handle_scroll(
                    position.x,
                    direction,
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
    surface.exclusive_zone = i32::try_from(style.height.max(1)).unwrap_or(i32::MAX);
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
    match WidgetKind::parse(name) {
        Some(WidgetKind::Clock) => old.clock == new.clock,
        Some(WidgetKind::Battery) => old.battery == new.battery,
        Some(WidgetKind::Memory) => old.memory == new.memory,
        Some(WidgetKind::Cpu) => old.cpu == new.cpu,
        Some(WidgetKind::Process) => old.process == new.process,
        Some(WidgetKind::Volume) => old.volume == new.volume,
        Some(WidgetKind::Workspace) => old.workspace == new.workspace,
        Some(WidgetKind::Window) => {
            old.window == new.window
                && old.workspace.persistent_workspaces == new.workspace.persistent_workspaces
        }
        Some(WidgetKind::Tray) => old.tray.provider == new.tray.provider,
        None => false,
    }
}

fn create_widget(
    name: &str,
    config: &Config,
    ipc_client: &Arc<dyn WmClient>,
) -> Result<Box<dyn crate::widget::Widget>, Box<dyn Error>> {
    let kind = WidgetKind::parse(name).ok_or_else(|| format!("unknown widget `{name}`"))?;

    match kind {
        WidgetKind::Clock => {
            Ok(Box::new(Clock::new(&config.clock)) as Box<dyn crate::widget::Widget>)
        }
        WidgetKind::Battery => {
            Ok(Box::new(Battery::new(&config.battery)) as Box<dyn crate::widget::Widget>)
        }
        WidgetKind::Memory => {
            Ok(Box::new(Memory::new(&config.memory)) as Box<dyn crate::widget::Widget>)
        }
        WidgetKind::Cpu => Ok(Box::new(Cpu::new(&config.cpu)) as Box<dyn crate::widget::Widget>),
        WidgetKind::Process => {
            Ok(Box::new(Process::new(&config.process)) as Box<dyn crate::widget::Widget>)
        }
        WidgetKind::Volume => {
            Ok(Box::new(Volume::new(&config.volume)) as Box<dyn crate::widget::Widget>)
        }
        WidgetKind::Workspace => Ok(Box::new(Workspace::new(
            &config.workspace,
            Arc::clone(ipc_client),
        )) as Box<dyn crate::widget::Widget>),
        WidgetKind::Window => Ok(
            Box::new(Window::new(&config.window, Arc::clone(ipc_client)))
                as Box<dyn crate::widget::Widget>,
        ),
        WidgetKind::Tray => Ok(Box::new(Tray::new(&config.tray, config.style.clone())?)
            as Box<dyn crate::widget::Widget>),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn widget_ptr(widget: &dyn crate::widget::Widget) -> *const () {
        std::ptr::from_ref::<dyn crate::widget::Widget>(widget).cast::<()>()
    }

    #[test]
    fn failed_reload_preserves_existing_widgets() {
        let mut config = Config::default();
        config.widgets.left.push("clock".to_string());
        let mut app = App::new(&config).unwrap();
        let original = widget_ptr(&*app.widgets.left[0]);
        let revision = app.revision;
        let surfaces = app.surfaces.clone();

        let mut invalid = config.clone();
        invalid.widgets.left = vec!["tray".to_string()];
        invalid.tray.provider = "missing".to_string();

        assert!(app.apply_config(&invalid).is_err());
        assert_eq!(app.widgets.left.len(), 1);
        assert!(std::ptr::eq(original, widget_ptr(&*app.widgets.left[0])));
        assert_eq!(app.active_config, config);
        assert_eq!(app.revision, revision);
        assert_eq!(app.surfaces[0].size, surfaces[0].size);
    }

    #[test]
    fn reload_reuses_moved_widgets_and_invalidates_cached_frames() {
        let mut config = Config::default();
        config.widgets.left = vec!["clock".to_string()];
        let mut app = App::new(&config).unwrap();
        let original = widget_ptr(&*app.widgets.left[0]);
        app.frames.insert(
            None,
            CachedFrame {
                size: Size::new(100, 30),
                revision: app.revision,
                pixels: vec![0; 100 * 30 * 4],
            },
        );

        let mut updated = config.clone();
        updated.widgets.left.clear();
        updated.widgets.right = vec!["clock".to_string()];
        updated.style.font_size += 1.0;
        app.apply_config(&updated).unwrap();
        assert!(std::ptr::eq(original, widget_ptr(&*app.widgets.right[0])));
        assert!(app.frames.is_empty());
        assert_eq!(app.revision, 1);
        assert!(!app.should_close());

        updated.clock.format = Some("%H".to_string());
        app.apply_config(&updated).unwrap();
        assert!(!std::ptr::eq(original, widget_ptr(&*app.widgets.right[0])));

        updated.style.height += 1;
        app.apply_config(&updated).unwrap();
        assert!(app.should_close());
        assert_eq!(app.surfaces[0].size.height, updated.style.height);
    }

    #[test]
    fn polling_config_applies_valid_changes_and_keeps_the_last_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rubar.toml");
        let mut app = App::new(&Config::default()).unwrap();
        app.config_path = Some(path.clone());
        app.last_contents = None;

        fs::write(&path, "[widgets]\nleft = ['clock']\n").unwrap();
        app.last_config_check = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        app.poll_config();
        assert_eq!(app.active_config.widgets.left, ["clock"]);
        assert_eq!(app.widgets.left.len(), 1);
        let revision = app.revision;

        fs::write(&path, "[style]\nheight = 0\n").unwrap();
        app.last_config_check = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        app.poll_config();
        assert_eq!(app.active_config.widgets.left, ["clock"]);
        assert_eq!(app.revision, revision);

        fs::remove_file(&path).unwrap();
        app.last_config_check = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
        app.poll_config();
        assert_eq!(app.active_config, Config::default());
        assert!(app.widgets.left.is_empty());
    }

    #[test]
    fn dormant_trays_are_reused_only_with_the_same_provider() {
        let mut config = Config::default();
        config.widgets.left = vec!["tray".to_string()];
        let mut app = App::new(&config).unwrap();
        let original = widget_ptr(&*app.widgets.left[0]);

        let mut updated = config.clone();
        updated.widgets.left.clear();
        app.apply_config(&updated).unwrap();
        assert_eq!(app.dormant_trays.len(), 1);
        assert!(std::ptr::eq(original, widget_ptr(&*app.dormant_trays[0].1)));

        updated.widgets.right = vec!["tray".to_string()];
        app.apply_config(&updated).unwrap();
        assert!(std::ptr::eq(original, widget_ptr(&*app.widgets.right[0])));

        updated.widgets.right.clear();
        app.apply_config(&updated).unwrap();
        assert_eq!(app.dormant_trays.len(), 1);

        updated.tray.provider = "sni".to_string();
        app.apply_config(&updated).unwrap();
        assert!(app.dormant_trays.is_empty());
    }
}
