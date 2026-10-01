use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{Arc, RwLock},
    thread,
    time::Duration,
};

use serde::Deserialize;

use crate::config::WorkspaceConfig;

use super::Widget;

#[derive(Debug)]
pub struct Workspace {
    format: String,
    all_monitors: bool,
    state: Arc<RwLock<Option<WorkspaceState>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceState {
    workspaces: Vec<HyprWorkspace>,
    active_id: i64,
    active_monitor_id: i64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct HyprWorkspace {
    id: i64,
    name: String,
    #[serde(rename = "monitorID")]
    monitor_id: i64,
}

#[derive(Debug, Deserialize)]
struct ActiveWorkspace {
    id: i64,
    #[serde(rename = "monitorID")]
    monitor_id: i64,
}

impl Workspace {
    pub fn new(config: &WorkspaceConfig) -> Self {
        let state = Arc::new(RwLock::new(None));
        let reconnect_interval = Duration::from_secs(config.refresh_seconds.max(1));

        match hyprland_socket_dir() {
            Ok(socket_dir) => {
                update_state(&socket_dir, &state);
                let event_socket = socket_dir.join(".socket2.sock");
                let command_socket = socket_dir.clone();
                let shared_state = Arc::clone(&state);
                thread::spawn(move || {
                    event_loop(
                        event_socket,
                        command_socket,
                        shared_state,
                        reconnect_interval,
                    )
                });
            }
            Err(error) => eprintln!("rubar: could not locate Hyprland IPC sockets: {error}"),
        }

        Self {
            format: config.format.clone(),
            all_monitors: config.all_monitors,
            state,
        }
    }
}

impl Widget for Workspace {
    fn text(&mut self) -> String {
        let state = self.state.read().ok().and_then(|state| state.clone());
        let Some(state) = state else {
            return self.format.replace("{workspaces}", "--");
        };
        let workspaces = render_workspaces(&state, self.all_monitors);
        self.format
            .replace("{workspaces}", &workspaces)
            .replace("{active}", &state.active_id.to_string())
    }
}

fn hyprland_socket_dir() -> Result<PathBuf, String> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| "XDG_RUNTIME_DIR is not set".to_string())?;
    let instance = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")
        .ok_or_else(|| "HYPRLAND_INSTANCE_SIGNATURE is not set".to_string())?;
    Ok(PathBuf::from(runtime_dir).join("hypr").join(instance))
}

fn event_loop(
    event_socket: PathBuf,
    command_socket: PathBuf,
    state: Arc<RwLock<Option<WorkspaceState>>>,
    reconnect_interval: Duration,
) {
    loop {
        match UnixStream::connect(&event_socket) {
            Ok(stream) => {
                let reader = BufReader::new(stream);
                for line in reader.lines() {
                    match line {
                        Ok(event) if is_workspace_event(&event) => {
                            update_state(&command_socket, &state);
                        }
                        Ok(_) => {}
                        Err(error) => {
                            eprintln!("rubar: Hyprland event socket read failed: {error}");
                            break;
                        }
                    }
                }
            }
            Err(error) => {
                eprintln!("rubar: could not connect to Hyprland event socket: {error}");
            }
        }
        thread::sleep(reconnect_interval);
    }
}

fn is_workspace_event(event: &str) -> bool {
    matches!(
        event.split_once(">>").map(|(name, _)| name),
        Some(
            "workspace"
                | "focusedmon"
                | "createworkspace"
                | "destroyworkspace"
                | "moveworkspace"
                | "renameworkspace"
                | "activespecial"
                | "activelayout"
        )
    )
}

fn update_state(socket_dir: &PathBuf, state: &Arc<RwLock<Option<WorkspaceState>>>) {
    match read_state(socket_dir) {
        Ok(new_state) => {
            if let Ok(mut state) = state.write() {
                *state = Some(new_state);
            }
        }
        Err(error) => eprintln!("rubar: could not read Hyprland workspace state: {error}"),
    }
}

fn read_state(socket_dir: &PathBuf) -> Result<WorkspaceState, String> {
    let mut workspaces = request_json::<Vec<HyprWorkspace>>(socket_dir, "j/workspaces")?;
    workspaces.sort_by_key(|workspace| workspace.id);
    let active = request_json::<ActiveWorkspace>(socket_dir, "j/activeworkspace")?;
    Ok(WorkspaceState {
        workspaces,
        active_id: active.id,
        active_monitor_id: active.monitor_id,
    })
}

fn request_json<T>(socket_dir: &PathBuf, request: &str) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let socket = socket_dir.join(".socket.sock");
    let mut stream = UnixStream::connect(&socket)
        .map_err(|error| format!("could not connect to {}: {error}", socket.display()))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("could not write to {}: {error}", socket.display()))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| format!("could not read from {}: {error}", socket.display()))?;
    serde_json::from_str(&response)
        .map_err(|error| format!("could not parse Hyprland response: {error}"))
}

fn render_workspaces(state: &WorkspaceState, all_monitors: bool) -> String {
    let mut monitor_indices = std::collections::HashMap::new();
    state
        .workspaces
        .iter()
        .filter(|workspace| all_monitors || workspace.monitor_id == state.active_monitor_id)
        .map(|workspace| {
            let index = monitor_indices.entry(workspace.monitor_id).or_insert(0);
            *index += 1;
            let name = if workspace.name.parse::<i64>().is_ok() {
                index.to_string()
            } else {
                workspace.name.clone()
            };
            if workspace.id == state.active_id {
                format!("[{name}]")
            } else {
                name
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{HyprWorkspace, WorkspaceState, is_workspace_event, render_workspaces};

    #[test]
    fn workspaces_are_rendered_with_active_workspace_marked() {
        let state = WorkspaceState {
            workspaces: vec![
                HyprWorkspace {
                    id: 1,
                    name: "1".to_string(),
                    monitor_id: 0,
                },
                HyprWorkspace {
                    id: 2,
                    name: "dev".to_string(),
                    monitor_id: 0,
                },
                HyprWorkspace {
                    id: 11,
                    name: "1".to_string(),
                    monitor_id: 1,
                },
            ],
            active_id: 2,
            active_monitor_id: 0,
        };
        assert_eq!(render_workspaces(&state, false), "1 [2]");
        assert_eq!(render_workspaces(&state, true), "1 [2] 1");
    }

    #[test]
    fn only_workspace_events_trigger_refresh() {
        assert!(is_workspace_event("workspace>>2"));
        assert!(is_workspace_event("createworkspace>>3"));
        assert!(!is_workspace_event("openwindow>>address>>workspace"));
    }
}
