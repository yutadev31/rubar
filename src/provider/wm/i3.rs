use super::{WmClient, Workspace, WorkspaceState};
use serde::Deserialize;
use std::{
    collections::HashMap,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    thread,
    time::Duration,
};

/// Workspace provider for i3 and Sway, which share the i3 IPC protocol.
pub(crate) struct I3Client {
    wm_name: &'static str,
    socket: PathBuf,
    state: Arc<RwLock<Option<WorkspaceState>>>,
    workspace_names: Arc<RwLock<HashMap<i64, String>>>,
    title: Arc<RwLock<Option<String>>>,
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

impl I3Client {
    pub(crate) fn new(
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
        let title = Arc::new(RwLock::new(read_window_title(&socket)?));
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
        let event_title = Arc::clone(&title);
        let event_wm_name = wm_name;
        thread::spawn(move || {
            ipc_event_loop(
                event_wm_name,
                event_socket,
                shared_state,
                shared_names,
                event_persistent_workspaces,
                event_title,
                reconnect_interval,
            )
        });

        Ok(Self {
            wm_name,
            socket,
            state,
            workspace_names,
            title,
        })
    }
}

impl WmClient for I3Client {
    fn state(&self) -> Option<WorkspaceState> {
        self.state.read().ok().and_then(|state| state.clone())
    }

    fn switch_to(
        &self,
        workspace_id: i64,
        _monitor: Option<&str>,
        _local_index: Option<i64>,
    ) -> Result<(), String> {
        let name = self
            .workspace_names
            .read()
            .ok()
            .and_then(|names| names.get(&workspace_id).cloned())
            .ok_or_else(|| format!("unknown {} workspace {workspace_id}", self.wm_name))?;
        let response = ipc_request(&self.socket, 0, format!("workspace {name}").as_bytes())?;
        let result = parse_ipc_command_response(&response).map_err(|error| {
            format!("could not parse {} command response: {error}", self.wm_name)
        })?;
        if !result {
            return Err(format!("{} rejected workspace {name}", self.wm_name));
        }
        Ok(())
    }

    fn title(&self) -> Option<String> {
        self.title.read().ok().and_then(|title| title.clone())
    }
}

#[derive(Debug, Deserialize)]
struct IpcCommandResult {
    success: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum IpcCommandResponse {
    Single(IpcCommandResult),
    Multiple(Vec<IpcCommandResult>),
}

fn parse_ipc_command_response(response: &[u8]) -> Result<bool, serde_json::Error> {
    let response: IpcCommandResponse = serde_json::from_slice(response)?;
    Ok(match response {
        IpcCommandResponse::Single(result) => result.success,
        IpcCommandResponse::Multiple(results) => {
            !results.is_empty() && results.iter().all(|result| result.success)
        }
    })
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
            Workspace {
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
    title: Arc<RwLock<Option<String>>>,
    reconnect_interval: Duration,
) {
    loop {
        match UnixStream::connect(&socket) {
            Ok(mut stream) => {
                if ipc_write_message(&mut stream, 2, br#"["workspace","output","window"]"#).is_ok()
                    && ipc_read_message(&mut stream).is_ok()
                {
                    loop {
                        match ipc_read_message(&mut stream) {
                            Ok((message_type, _payload)) if message_type & 0x8000_0000 != 0 => {
                                let event_type = message_type & 0x7fff_ffff;
                                if event_type == 3 || event_type == 0 {
                                    match read_window_title(&socket) {
                                        Ok(value) => {
                                            if let Ok(mut target) = title.write() {
                                                *target = value;
                                            }
                                        }
                                        Err(error) => eprintln!(
                                            "rubar: could not read {wm_name} focused window: {error}"
                                        ),
                                    }
                                } else {
                                    // Workspace and output events refresh the state below.
                                }
                                if event_type == 0 || event_type == 1 {
                                    let _ = update_ipc_state(
                                        wm_name,
                                        &socket,
                                        &state,
                                        &workspace_names,
                                        &persistent_workspaces,
                                    );
                                }
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

fn read_window_title(socket: &Path) -> Result<Option<String>, String> {
    let tree: serde_json::Value = serde_json::from_slice(&ipc_request(socket, 4, b"")?)
        .map_err(|e| format!("could not parse i3 tree: {e}"))?;
    fn focused(node: &serde_json::Value) -> Option<String> {
        if node.get("focused").and_then(serde_json::Value::as_bool) == Some(true) {
            return node
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
        }
        node.get("nodes")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .chain(
                node.get("floating_nodes")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten(),
            )
            .find_map(focused)
    }
    Ok(focused(&tree))
}
