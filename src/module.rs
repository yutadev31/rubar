#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModuleKind {
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

impl ModuleKind {
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
