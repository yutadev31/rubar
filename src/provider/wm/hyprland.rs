use super::{WmClient, Workspace, WorkspaceState};
use serde::Deserialize;
use std::{
    collections::HashSet,
    io::{BufRead, BufReader, Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    thread,
    time::Duration,
};

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
#[derive(Debug, Deserialize)]
struct HyprClient {
    address: String,
    workspace: HyprClientWorkspace,
}
#[derive(Debug, Deserialize)]
struct HyprClientWorkspace {
    id: i64,
}

pub(crate) struct HyprlandClient {
    socket_dir: PathBuf,
    state: Arc<RwLock<Option<WorkspaceState>>>,
    title: Arc<RwLock<Option<String>>>,
}

impl HyprlandClient {
    pub(crate) fn new(reconnect_interval: Duration) -> Result<Self, String> {
        let socket_dir = hyprland_socket_dir()?;
        let state = Arc::new(RwLock::new(None));
        let title = Arc::new(RwLock::new(read_window_title(&socket_dir)?));
        update_state(&socket_dir, &state, &HashSet::new());

        let event_socket = socket_dir.join(".socket2.sock");
        let command_socket = socket_dir.clone();
        let shared_state = Arc::clone(&state);
        let shared_title = Arc::clone(&title);
        thread::spawn(move || {
            event_loop(
                event_socket,
                command_socket,
                shared_state,
                shared_title,
                HashSet::new(),
                reconnect_interval,
            )
        });

        Ok(Self {
            socket_dir,
            state,
            title,
        })
    }
}

impl WmClient for HyprlandClient {
    fn state(&self) -> Option<WorkspaceState> {
        self.state.read().ok().and_then(|state| state.clone())
    }

    fn switch_to(
        &self,
        workspace_id: i64,
        monitor: Option<&str>,
        local_index: Option<i64>,
    ) -> Result<(), String> {
        let socket = self.socket_dir.join(".socket.sock");

        // split-monitor-workspaces resolves N on the currently focused
        // monitor. Select the monitor represented by the clicked button first.
        if let (Some(monitor), Some(local_index)) = (monitor, local_index) {
            let focus_response = send_command(&socket, &format!("dispatch focusmonitor {monitor}"))
                .map_err(|error| {
                    format!("could not focus monitor {monitor} for workspace switch: {error}")
                })?;
            if !focus_response.starts_with("error") {
                let split_response = send_command(
                    &socket,
                    &format!("dispatch split-workspace {local_index}"),
                )
                .map_err(|error| {
                    format!(
                        "could not switch to workspace {local_index} on monitor {monitor}: {error}"
                    )
                })?;
                if !split_response.starts_with("error") {
                    return Ok(());
                }
            }
        }

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

    fn title(&self) -> Option<String> {
        self.title.read().ok().and_then(|title| title.clone())
    }
}

fn send_command(socket: &Path, command: &str) -> Result<String, String> {
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
    title: Arc<RwLock<Option<String>>>,
    mut urgent_windows: HashSet<String>,
    reconnect_interval: Duration,
) {
    loop {
        match UnixStream::connect(&event_socket) {
            Ok(stream) => {
                let reader = BufReader::new(stream);
                for line in reader.lines() {
                    match line {
                        Ok(event) if is_workspace_event(&event) => {
                            update_state(&command_socket, &state, &urgent_windows)
                        }
                        Ok(event) if is_urgent_event(&event) => {
                            if let Some((_, address)) = event.split_once(">>") {
                                urgent_windows.insert(window_address(address).to_string());
                                update_state(&command_socket, &state, &urgent_windows);
                            }
                        }
                        Ok(event) if is_window_event(&event) => {
                            if let Some((name, address)) = event.split_once(">>")
                                && matches!(name, "activewindowv2" | "closewindow")
                            {
                                urgent_windows.remove(window_address(address));
                            }
                            if matches!(
                                event.split_once(">>").map(|(name, _)| name),
                                Some("openwindow" | "closewindow" | "activewindowv2")
                            ) {
                                update_state(&command_socket, &state, &urgent_windows);
                            }
                            match read_window_title(&command_socket) {
                                Ok(value) => {
                                    if let Ok(mut target) = title.write() {
                                        *target = value;
                                    }
                                }
                                Err(error) => eprintln!(
                                    "rubar: could not read Hyprland focused window: {error}"
                                ),
                            }
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
                | "movewindow"
                | "movewindowv2"
                | "renameworkspace"
                | "activespecial"
                | "activelayout"
        )
    )
}

fn is_window_event(event: &str) -> bool {
    matches!(
        event.split_once(">>").map(|(name, _)| name),
        Some("activewindow" | "activewindowv2" | "openwindow" | "closewindow")
    )
}

fn is_urgent_event(event: &str) -> bool {
    event
        .split_once(">>")
        .is_some_and(|(name, address)| name == "urgent" && !address.is_empty())
}

fn window_address(address: &str) -> &str {
    address.strip_prefix("0x").unwrap_or(address)
}

fn update_state(
    socket_dir: &Path,
    state: &Arc<RwLock<Option<WorkspaceState>>>,
    urgent_windows: &HashSet<String>,
) {
    match read_state(socket_dir, urgent_windows) {
        Ok(new_state) => {
            if let Ok(mut state) = state.write() {
                *state = Some(new_state);
            }
        }
        Err(error) => eprintln!("rubar: could not read Hyprland workspace state: {error}"),
    }
}

fn read_state(
    socket_dir: &Path,
    urgent_windows: &HashSet<String>,
) -> Result<WorkspaceState, String> {
    let mut workspaces = request_json::<Vec<Workspace>>(socket_dir, "j/workspaces")?;
    workspaces.sort_by_key(|workspace| workspace.id);
    let clients = request_json::<Vec<HyprClient>>(socket_dir, "j/clients")?;
    let urgent_workspace_ids = urgent_workspace_ids(clients, urgent_windows);
    for workspace in &mut workspaces {
        workspace.urgent = urgent_workspace_ids.contains(&workspace.id);
    }
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

fn urgent_workspace_ids(
    clients: Vec<HyprClient>,
    urgent_windows: &HashSet<String>,
) -> HashSet<i64> {
    clients
        .into_iter()
        .filter(|client| urgent_windows.contains(window_address(&client.address)))
        .map(|client| client.workspace.id)
        .collect()
}

fn request_json<T>(socket_dir: &Path, request: &str) -> Result<T, String>
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

fn read_window_title(dir: &Path) -> Result<Option<String>, String> {
    let value: serde_json::Value = request_json(dir, "j/activewindow")?;
    Ok(value
        .get("title")
        .and_then(serde_json::Value::as_str)
        .filter(|title| !title.is_empty())
        .map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::{HyprClient, HyprClientWorkspace, urgent_workspace_ids, window_address};
    use std::collections::HashSet;

    #[test]
    fn urgent_event_address_matches_client_address() {
        let urgent_windows = HashSet::from([window_address("abc123").to_string()]);
        let clients = vec![
            HyprClient {
                address: "0xabc123".to_string(),
                workspace: HyprClientWorkspace { id: 2 },
            },
            HyprClient {
                address: "0xdef456".to_string(),
                workspace: HyprClientWorkspace { id: 3 },
            },
        ];
        assert_eq!(
            urgent_workspace_ids(clients, &urgent_windows),
            HashSet::from([2])
        );
    }
}
