pub(crate) mod hyprland;
pub(crate) mod i3;
pub(crate) mod niri;

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
    #[serde(default)]
    pub(crate) urgent: bool,
}

pub(crate) trait WmClient: Send + Sync {
    fn state(&self) -> Option<WorkspaceState>;
    fn switch_to(
        &self,
        workspace_id: i64,
        monitor: Option<&str>,
        local_index: Option<i64>,
    ) -> Result<(), String>;
    fn title(&self) -> Option<String>;
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

pub(crate) fn connect(persistent_workspaces: &[i64]) -> Arc<dyn WmClient> {
    const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);
    let persistent = configured_workspace_names(persistent_workspaces);

    if let Some(desktop) = std::env::var_os("XDG_CURRENT_DESKTOP") {
        let desktop = desktop.to_string_lossy();
        let provider = desktop.split(':').find_map(|desktop| match desktop {
            "sway" => Some(
                i3::I3Client::new("Sway", "SWAYSOCK", RECONNECT_INTERVAL, persistent.clone())
                    .map(|provider| Arc::new(provider) as Arc<dyn WmClient>),
            ),
            "i3" => Some(
                i3::I3Client::new("i3", "I3SOCK", RECONNECT_INTERVAL, persistent.clone())
                    .map(|provider| Arc::new(provider) as Arc<dyn WmClient>),
            ),
            "niri" => Some(
                niri::NiriClient::new(RECONNECT_INTERVAL)
                    .map(|provider| Arc::new(provider) as Arc<dyn WmClient>),
            ),
            "hyprland" | "Hyprland" => Some(
                hyprland::HyprlandClient::new(RECONNECT_INTERVAL)
                    .map(|provider| Arc::new(provider) as Arc<dyn WmClient>),
            ),
            _ => None,
        });

        match provider {
            Some(Ok(provider)) => return provider,
            Some(Err(error)) => {
                tracing::warn!(
                    "rubar: could not connect to desktop {desktop}: {error}; falling back to compositor detection"
                );
            }
            None => tracing::warn!(
                "rubar: unrecognized XDG_CURRENT_DESKTOP value {desktop:?}; falling back to compositor detection"
            ),
        }
    } else {
        tracing::warn!(
            "rubar: XDG_CURRENT_DESKTOP is not set; falling back to compositor detection"
        );
    }

    connect_by_detection(&persistent)
}

fn connect_by_detection(persistent: &[String]) -> Arc<dyn WmClient> {
    const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);

    for (name, variable) in [("Sway", "SWAYSOCK"), ("i3", "I3SOCK")] {
        if let Ok(provider) =
            i3::I3Client::new(name, variable, RECONNECT_INTERVAL, persistent.to_vec())
        {
            return Arc::new(provider);
        }
    }

    if let Ok(provider) = niri::NiriClient::new(RECONNECT_INTERVAL) {
        return Arc::new(provider);
    }

    match hyprland::HyprlandClient::new(RECONNECT_INTERVAL) {
        Ok(client) => Arc::new(client),
        Err(error) => {
            tracing::warn!("rubar: could not connect to compositor IPC: {error}");
            Arc::new(UnavailableClient)
        }
    }
}

struct UnavailableClient;
impl WmClient for UnavailableClient {
    fn state(&self) -> Option<WorkspaceState> {
        None
    }
    fn switch_to(&self, _: i64, _: Option<&str>, _: Option<i64>) -> Result<(), String> {
        Err("workspace IPC is unavailable".to_string())
    }
    fn title(&self) -> Option<String> {
        None
    }
}
