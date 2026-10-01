use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{Arc, RwLock},
    thread,
    time::Duration,
};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceState {
    pub(crate) workspaces: Vec<HyprWorkspace>,
    pub(crate) active_id: i64,
    pub(crate) active_monitor_id: i64,
    pub(crate) active_ids: HashMap<String, i64>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub(crate) struct HyprWorkspace {
    pub(crate) id: i64,
    pub(crate) name: String,
    #[serde(rename = "monitorID")]
    pub(crate) monitor_id: i64,
    #[serde(default)]
    pub(crate) monitor: String,
}

#[derive(Debug, Deserialize)]
struct ActiveWorkspace {
    id: i64,
    #[serde(rename = "monitorID")]
    monitor_id: i64,
}

#[derive(Debug, Deserialize)]
struct HyprMonitor {
    name: String,
    #[serde(rename = "activeWorkspace")]
    active_workspace: HyprActiveWorkspace,
}

#[derive(Debug, Deserialize)]
struct HyprActiveWorkspace {
    id: i64,
}

pub(crate) trait WorkspaceProvider: Send {
    fn state(&self) -> Option<WorkspaceState>;
    fn switch_to(&mut self, workspace_id: i64) -> Result<(), String>;
}

pub(crate) fn create(refresh_interval: Duration) -> Box<dyn WorkspaceProvider> {
    match HyprlandProvider::new(refresh_interval) {
        Ok(provider) => Box::new(provider),
        Err(error) => {
            eprintln!("rubar: could not locate Hyprland IPC sockets: {error}");
            Box::new(Unavailable)
        }
    }
}

struct HyprlandProvider {
    socket_dir: PathBuf,
    state: Arc<RwLock<Option<WorkspaceState>>>,
}

impl HyprlandProvider {
    fn new(reconnect_interval: Duration) -> Result<Self, String> {
        let socket_dir = hyprland_socket_dir()?;
        let state = Arc::new(RwLock::new(None));
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

        Ok(Self { socket_dir, state })
    }
}

impl WorkspaceProvider for HyprlandProvider {
    fn state(&self) -> Option<WorkspaceState> {
        self.state.read().ok().and_then(|state| state.clone())
    }

    fn switch_to(&mut self, workspace_id: i64) -> Result<(), String> {
        let socket = self.socket_dir.join(".socket.sock");
        let legacy_command = format!("dispatch workspace {workspace_id}");
        let response = send_command(&socket, &legacy_command)
            .map_err(|error| format!("could not switch to workspace {workspace_id}: {error}"))?;
        if !response.starts_with("error") {
            return Ok(());
        }

        // Hyprland 0.55+ with a Lua config evaluates `dispatch` as a Lua
        // expression and no longer accepts the legacy dispatcher syntax.
        let lua_command = format!("dispatch hl.dsp.focus({{ workspace = \"{workspace_id}\" }})");
        let response = send_command(&socket, &lua_command)
            .map_err(|error| format!("could not switch to workspace {workspace_id}: {error}"))?;
        if response.starts_with("error") {
            return Err(format!(
                "Hyprland rejected workspace {workspace_id}: {response}"
            ));
        }
        Ok(())
    }
}

fn send_command(socket: &PathBuf, command: &str) -> Result<String, String> {
    let mut stream = UnixStream::connect(socket)
        .map_err(|error| format!("could not connect to {}: {error}", socket.display()))?;
    stream
        .write_all(command.as_bytes())
        .map_err(|error| format!("could not write command: {error}"))?;
    stream
        .shutdown(Shutdown::Write)
        .map_err(|error| format!("could not finish command: {error}"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| format!("could not read command response: {error}"))?;
    Ok(response)
}

struct Unavailable;

impl WorkspaceProvider for Unavailable {
    fn state(&self) -> Option<WorkspaceState> {
        None
    }

    fn switch_to(&mut self, _workspace_id: i64) -> Result<(), String> {
        Err("Hyprland IPC is unavailable".to_string())
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
    let monitors = request_json::<Vec<HyprMonitor>>(socket_dir, "j/monitors").unwrap_or_default();
    let active_ids = monitors
        .into_iter()
        .map(|monitor| (monitor.name, monitor.active_workspace.id))
        .collect();
    Ok(WorkspaceState {
        workspaces,
        active_id: active.id,
        active_monitor_id: active.monitor_id,
        active_ids,
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

#[cfg(test)]
mod tests {
    use super::is_workspace_event;

    #[test]
    fn only_workspace_events_trigger_refresh() {
        assert!(is_workspace_event("workspace>>2"));
        assert!(is_workspace_event("createworkspace>>3"));
        assert!(!is_workspace_event("openwindow>>address>>workspace"));
    }
}
