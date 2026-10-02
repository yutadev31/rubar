use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
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

pub(crate) fn create(
    workspace_range: Option<[i64; 2]>,
    persistent_workspaces: &[i64],
) -> Box<dyn WorkspaceProvider> {
    const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);
    let persistent_workspaces = configured_workspace_names(workspace_range, persistent_workspaces);
    // i3 and Sway intentionally share the same IPC protocol.  Prefer the
    // compositor-specific environment variable when both happen to be set.
    if let Ok(provider) = IpcProvider::new(
        "Sway",
        "SWAYSOCK",
        RECONNECT_INTERVAL,
        persistent_workspaces.clone(),
    ) {
        return Box::new(provider);
    }
    if let Ok(provider) =
        IpcProvider::new("i3", "I3SOCK", RECONNECT_INTERVAL, persistent_workspaces)
    {
        return Box::new(provider);
    }
    match HyprlandProvider::new(RECONNECT_INTERVAL) {
        Ok(provider) => Box::new(provider),
        Err(error) => {
            eprintln!("rubar: could not locate i3, Sway, or Hyprland IPC sockets: {error}");
            Box::new(Unavailable)
        }
    }
}

fn configured_workspace_names(
    workspace_range: Option<[i64; 2]>,
    persistent_workspaces: &[i64],
) -> Vec<String> {
    let mut names = Vec::new();
    if let Some([start, end]) = workspace_range {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        names.extend((start..=end).map(|id| id.to_string()));
    }
    names.extend(persistent_workspaces.iter().map(ToString::to_string));
    let mut unique = Vec::with_capacity(names.len());
    for name in names {
        if !unique.contains(&name) {
            unique.push(name);
        }
    }
    unique
}

struct IpcProvider {
    wm_name: &'static str,
    socket: PathBuf,
    state: Arc<RwLock<Option<WorkspaceState>>>,
    workspace_names: Arc<RwLock<HashMap<i64, String>>>,
}

#[derive(Debug, Clone, Deserialize)]
struct IpcWorkspace {
    name: String,
    output: String,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    visible: bool,
}

impl IpcProvider {
    fn new(
        wm_name: &'static str,
        socket_variable: &str,
        reconnect_interval: Duration,
        persistent_workspaces: Vec<String>,
    ) -> Result<Self, String> {
        let socket = std::env::var_os(socket_variable)
            .map(PathBuf::from)
            .ok_or_else(|| format!("{socket_variable} is not set"))?;
        if !socket.exists() {
            return Err(format!(
                "{wm_name} socket does not exist: {}",
                socket.display()
            ));
        }

        let state = Arc::new(RwLock::new(None));
        let workspace_names = Arc::new(RwLock::new(HashMap::new()));
        update_ipc_state(
            wm_name,
            &socket,
            &state,
            &workspace_names,
            &persistent_workspaces,
        )?;

        let event_socket = socket.clone();
        let shared_state = Arc::clone(&state);
        let shared_names = Arc::clone(&workspace_names);
        let event_persistent_workspaces = persistent_workspaces.clone();
        let event_wm_name = wm_name;
        thread::spawn(move || {
            ipc_event_loop(
                event_wm_name,
                event_socket,
                shared_state,
                shared_names,
                event_persistent_workspaces,
                reconnect_interval,
            )
        });

        Ok(Self {
            wm_name,
            socket,
            state,
            workspace_names,
        })
    }
}

impl WorkspaceProvider for IpcProvider {
    fn state(&self) -> Option<WorkspaceState> {
        self.state.read().ok().and_then(|state| state.clone())
    }

    fn switch_to(&mut self, workspace_id: i64) -> Result<(), String> {
        let name = self
            .workspace_names
            .read()
            .ok()
            .and_then(|names| names.get(&workspace_id).cloned())
            .ok_or_else(|| format!("unknown {} workspace {workspace_id}", self.wm_name))?;
        let response = ipc_request(&self.socket, 0, format!("workspace {name}").as_bytes())?;
        let result: IpcCommandResult = serde_json::from_slice(&response).map_err(|error| {
            format!("could not parse {} command response: {error}", self.wm_name)
        })?;
        if !result.success {
            return Err(format!("{} rejected workspace {name}", self.wm_name));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct IpcCommandResult {
    success: bool,
}

fn update_ipc_state(
    wm_name: &str,
    socket: &Path,
    state: &Arc<RwLock<Option<WorkspaceState>>>,
    workspace_names: &Arc<RwLock<HashMap<i64, String>>>,
    persistent_workspaces: &[String],
) -> Result<(), String> {
    let response = ipc_request(socket, 1, b"{}")?;
    let raw: Vec<IpcWorkspace> = serde_json::from_slice(&response)
        .map_err(|error| format!("could not parse {wm_name} workspaces: {error}"))?;

    let mut output_ids = HashMap::new();
    for workspace in &raw {
        let next_id = output_ids.len() as i64;
        output_ids
            .entry(workspace.output.clone())
            .or_insert(next_id);
    }

    let fallback_output = raw
        .iter()
        .find(|workspace| workspace.focused)
        .or_else(|| raw.first())
        .map(|workspace| workspace.output.clone())
        .unwrap_or_default();
    let mut used = vec![false; raw.len()];
    let mut specs = Vec::new();
    for name in persistent_workspaces {
        if let Some((index, workspace)) = raw
            .iter()
            .enumerate()
            .find(|(index, workspace)| !used[*index] && workspace.name == *name)
        {
            used[index] = true;
            specs.push((
                workspace.name.clone(),
                workspace.output.clone(),
                workspace.focused,
                workspace.visible,
            ));
        } else {
            // A missing workspace is placed on the focused output so it is
            // visible with the default all-monitors setting and can be
            // created by clicking its button.
            specs.push((name.clone(), fallback_output.clone(), false, false));
        }
    }
    specs.extend(
        raw.iter()
            .enumerate()
            .filter(|(index, _)| !used[*index])
            .map(|(_, workspace)| {
                (
                    workspace.name.clone(),
                    workspace.output.clone(),
                    workspace.focused,
                    workspace.visible,
                )
            }),
    );

    let mut names = HashMap::new();
    let workspaces = specs
        .iter()
        .enumerate()
        .map(|(index, (name, output, _, _))| {
            let id = index as i64 + 1;
            names.insert(id, name.clone());
            HyprWorkspace {
                id,
                name: name.clone(),
                monitor_id: *output_ids.get(output).unwrap_or(&0),
                monitor: output.clone(),
            }
        })
        .collect::<Vec<_>>();
    let active_index = specs.iter().position(|(_, _, focused, _)| *focused);
    let active_id = active_index
        .map(|index| index as i64 + 1)
        .unwrap_or_default();
    let active_monitor_id = active_index
        .and_then(|index| workspaces.get(index))
        .map(|workspace| workspace.monitor_id)
        .unwrap_or_default();
    let active_ids = specs
        .iter()
        .enumerate()
        .filter(|(_, (_, _, _, visible))| *visible)
        .map(|(index, (_, output, _, _))| (output.clone(), index as i64 + 1))
        .collect();

    if let Ok(mut target) = state.write() {
        *target = Some(WorkspaceState {
            workspaces,
            active_id,
            active_monitor_id,
            active_ids,
        });
    }
    if let Ok(mut target) = workspace_names.write() {
        *target = names;
    }
    Ok(())
}

fn ipc_event_loop(
    wm_name: &str,
    socket: PathBuf,
    state: Arc<RwLock<Option<WorkspaceState>>>,
    workspace_names: Arc<RwLock<HashMap<i64, String>>>,
    persistent_workspaces: Vec<String>,
    reconnect_interval: Duration,
) {
    loop {
        match UnixStream::connect(&socket) {
            Ok(mut stream) => {
                if ipc_write_message(&mut stream, 2, br#"["workspace","output"]"#).is_ok()
                    && ipc_read_message(&mut stream).is_ok()
                {
                    loop {
                        match ipc_read_message(&mut stream) {
                            Ok((message_type, _)) if message_type & 0x8000_0000 != 0 => {
                                let _ = update_ipc_state(
                                    wm_name,
                                    &socket,
                                    &state,
                                    &workspace_names,
                                    &persistent_workspaces,
                                );
                            }
                            Ok(_) => {}
                            Err(error) => {
                                eprintln!("rubar: {wm_name} event socket read failed: {error}");
                                break;
                            }
                        }
                    }
                }
            }
            Err(error) => eprintln!("rubar: could not connect to {wm_name} event socket: {error}"),
        }
        thread::sleep(reconnect_interval);
    }
}

fn ipc_request(socket: &Path, message_type: u32, payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut stream = UnixStream::connect(socket)
        .map_err(|error| format!("could not connect to {}: {error}", socket.display()))?;
    ipc_write_message(&mut stream, message_type, payload)?;
    ipc_read_message(&mut stream).map(|(_, payload)| payload)
}

fn ipc_write_message(
    stream: &mut UnixStream,
    message_type: u32,
    payload: &[u8],
) -> Result<(), String> {
    stream
        .write_all(b"i3-ipc")
        .and_then(|()| stream.write_all(&(payload.len() as u32).to_le_bytes()))
        .and_then(|()| stream.write_all(&message_type.to_le_bytes()))
        .and_then(|()| stream.write_all(payload))
        .map_err(|error| format!("could not write i3 IPC message: {error}"))
}

fn ipc_read_message(stream: &mut UnixStream) -> Result<(u32, Vec<u8>), String> {
    let mut header = [0; 14];
    stream
        .read_exact(&mut header)
        .map_err(|error| format!("could not read i3 IPC header: {error}"))?;
    if &header[..6] != b"i3-ipc" {
        return Err("invalid i3 IPC header".to_string());
    }
    let length = u32::from_le_bytes(header[6..10].try_into().unwrap()) as usize;
    let message_type = u32::from_le_bytes(header[10..14].try_into().unwrap());
    let mut payload = vec![0; length];
    stream
        .read_exact(&mut payload)
        .map_err(|error| format!("could not read i3 IPC payload: {error}"))?;
    Ok((message_type, payload))
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

struct Unavailable;

impl WorkspaceProvider for Unavailable {
    fn state(&self) -> Option<WorkspaceState> {
        None
    }

    fn switch_to(&mut self, _workspace_id: i64) -> Result<(), String> {
        Err("workspace IPC is unavailable".to_string())
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

fn update_state(socket_dir: &Path, state: &Arc<RwLock<Option<WorkspaceState>>>) {
    match read_state(socket_dir) {
        Ok(new_state) => {
            if let Ok(mut state) = state.write() {
                *state = Some(new_state);
            }
        }
        Err(error) => eprintln!("rubar: could not read Hyprland workspace state: {error}"),
    }
}

fn read_state(socket_dir: &Path) -> Result<WorkspaceState, String> {
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

#[cfg(test)]
mod tests {
    use super::{configured_workspace_names, is_workspace_event};

    #[test]
    fn configured_workspace_names_expands_and_deduplicates() {
        assert_eq!(
            configured_workspace_names(Some([5, 1]), &[3, 6, 3]),
            vec!["1", "2", "3", "4", "5", "6"]
        );
    }

    #[test]
    fn only_workspace_events_trigger_refresh() {
        assert!(is_workspace_event("workspace>>2"));
        assert!(is_workspace_event("createworkspace>>3"));
        assert!(!is_workspace_event("openwindow>>address>>workspace"));
    }
}
