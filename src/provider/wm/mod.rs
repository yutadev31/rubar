pub(crate) mod hyprland;
pub(crate) mod i3;

use serde::Deserialize;
use std::{collections::HashMap, sync::Arc, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceState {
    pub(crate) workspaces: Vec<Workspace>,
    pub(crate) active_id: i64,
    pub(crate) active_monitor_id: i64,
    pub(crate) active_ids: HashMap<String, i64>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct Workspace {
    pub(crate) id: i64,
    pub(crate) name: String,
    #[serde(rename = "monitorID")]
    pub(crate) monitor_id: i64,
    #[serde(default)]
    pub(crate) monitor: String,
}

pub(crate) trait WorkspaceProvider: Send + Sync {
    fn state(&self) -> Option<WorkspaceState>;
    fn switch_to(
        &self,
        workspace_id: i64,
        monitor: Option<&str>,
        local_index: Option<i64>,
    ) -> Result<(), String>;
}

fn configured_workspace_names(values: &[i64]) -> Vec<String> {
    let mut unique = Vec::with_capacity(values.len());
    for name in values.iter().map(ToString::to_string) {
        if !unique.contains(&name) {
            unique.push(name);
        }
    }
    unique
}

pub(crate) trait WindowProvider: Send {
    fn title(&self) -> Option<String>;
}

pub(crate) struct Clients {
    pub(crate) workspace: Arc<dyn WorkspaceProvider>,
    pub(crate) window: Arc<dyn WindowProvider>,
}

impl Clients {
    pub(crate) fn connect(persistent_workspaces: &[i64]) -> Self {
        const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);
        let persistent = configured_workspace_names(persistent_workspaces);

        for (name, variable) in [("Sway", "SWAYSOCK"), ("i3", "I3SOCK")] {
            if let (Ok(workspaces), Ok(window)) = (
                i3::I3WorkspaceProvider::new(
                    name,
                    variable,
                    RECONNECT_INTERVAL,
                    persistent.clone(),
                ),
                i3::WindowClient::new(name, variable),
            ) {
                return Self {
                    workspace: Arc::new(workspaces),
                    window: Arc::new(window),
                };
            }
        }

        match (
            hyprland::HyprlandProvider::new(RECONNECT_INTERVAL),
            hyprland::WindowClient::new(),
        ) {
            (Ok(workspaces), Ok(window)) => Self {
                workspace: Arc::new(workspaces),
                window: Arc::new(window),
            },
            (Err(error), _) | (_, Err(error)) => {
                eprintln!("rubar: could not connect to compositor IPC: {error}");
                Self {
                    workspace: Arc::new(UnavailableWorkspace),
                    window: Arc::new(UnavailableWindow),
                }
            }
        }
    }
}

struct UnavailableWorkspace;
impl WorkspaceProvider for UnavailableWorkspace {
    fn state(&self) -> Option<WorkspaceState> {
        None
    }
    fn switch_to(&self, _: i64, _: Option<&str>, _: Option<i64>) -> Result<(), String> {
        Err("workspace IPC is unavailable".to_string())
    }
}

struct UnavailableWindow;
impl WindowProvider for UnavailableWindow {
    fn title(&self) -> Option<String> {
        None
    }
}
