pub mod battery;
pub mod clock;
pub mod cpu;
pub mod memory;
pub mod process;
pub mod tray;
pub mod volume;
pub mod window;
pub mod workspace;

pub use crate::render::{
    MouseButton, RenderButton as WidgetButton, RenderContent as WidgetContent,
    RenderIcon as TrayIcon, RenderWidget as Widget, ScrollDirection,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WidgetKind {
    Clock,
    Battery,
    Memory,
    Cpu,
    Process,
    Volume,
    Workspace,
    Window,
    Tray,
}

impl WidgetKind {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "clock" => Self::Clock,
            "battery" => Self::Battery,
            "memory" => Self::Memory,
            "cpu" => Self::Cpu,
            "process" => Self::Process,
            "volume" => Self::Volume,
            "workspace" => Self::Workspace,
            "window" => Self::Window,
            "tray" => Self::Tray,
            _ => return None,
        })
    }
}

#[derive(Default)]
pub struct WidgetGroups {
    pub left: Vec<Box<dyn Widget>>,
    pub center: Vec<Box<dyn Widget>>,
    pub right: Vec<Box<dyn Widget>>,
}

impl WidgetGroups {
    pub fn poll_changed(&mut self) -> bool {
        let mut changed = false;
        for widget in self
            .left
            .iter_mut()
            .chain(self.center.iter_mut())
            .chain(self.right.iter_mut())
        {
            changed |= widget.poll_changed();
        }
        changed
    }

    pub fn set_monitor_name(&mut self, monitor_name: Option<&str>) {
        for widget in self
            .left
            .iter_mut()
            .chain(self.center.iter_mut())
            .chain(self.right.iter_mut())
        {
            widget.set_monitor_name(monitor_name);
        }
    }
}
