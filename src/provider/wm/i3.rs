use super::{WmClient, Workspace, WorkspaceState};
use serde::Deserialize;
use socket2::{Domain, SockAddr, Socket, Type};
use std::{
    collections::HashMap,
    io::{Read, Write},
    os::{fd::OwnedFd, unix::net::UnixStream},
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
    #[serde(default)]
    urgent: bool,
}

struct WorkspaceSpec {
    name: String,
    output: String,
    focused: bool,
    visible: bool,
    urgent: bool,
}

impl From<&IpcWorkspace> for WorkspaceSpec {
    fn from(workspace: &IpcWorkspace) -> Self {
        Self {
            name: workspace.name.clone(),
            output: workspace.output.clone(),
            focused: workspace.focused,
            visible: workspace.visible,
            urgent: workspace.urgent,
        }
    }
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
        let title = Arc::new(RwLock::new(None));
        tracing::debug!(window_manager = wm_name, socket = %socket.display(), "starting window manager IPC worker");

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
        match self.state.read() {
            Ok(state) => state.clone(),
            Err(error) => {
                tracing::error!(
                    "rubar: could not read {} workspace state: {error}",
                    self.wm_name
                );
                None
            }
        }
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
            .map_err(|error| format!("could not read {} workspace names: {error}", self.wm_name))?
            .get(&workspace_id)
            .cloned()
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
        match self.title.read() {
            Ok(title) => title.clone(),
            Err(error) => {
                tracing::error!(
                    "rubar: could not read {} window title: {error}",
                    self.wm_name
                );
                None
            }
        }
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

    let (new_state, names) = workspace_snapshot(&raw, persistent_workspaces);
    match state.write() {
        Ok(mut target) => *target = Some(new_state),
        Err(error) => tracing::error!("rubar: could not update {wm_name} workspace state: {error}"),
    }
    match workspace_names.write() {
        Ok(mut target) => *target = names,
        Err(error) => tracing::error!("rubar: could not update {wm_name} workspace names: {error}"),
    }
    Ok(())
}

fn workspace_snapshot(
    raw: &[IpcWorkspace],
    persistent_workspaces: &[String],
) -> (WorkspaceState, HashMap<i64, String>) {
    let mut output_ids = HashMap::new();
    for workspace in raw {
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
            specs.push(WorkspaceSpec::from(workspace));
        } else {
            // A missing workspace is placed on the focused output so it is
            // visible with the default all-monitors setting and can be
            // created by clicking its button.
            specs.push(WorkspaceSpec {
                name: name.clone(),
                output: fallback_output.clone(),
                focused: false,
                visible: false,
                urgent: false,
            });
        }
    }
    specs.extend(
        raw.iter()
            .enumerate()
            .filter(|(index, _)| !used[*index])
            .map(|(_, workspace)| WorkspaceSpec::from(workspace)),
    );

    let mut names = HashMap::new();
    let workspaces = specs
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            let id = index as i64 + 1;
            names.insert(id, spec.name.clone());
            Workspace {
                id,
                name: spec.name.clone(),
                monitor_id: *output_ids.get(&spec.output).unwrap_or(&0),
                monitor: spec.output.clone(),
                urgent: spec.urgent,
            }
        })
        .collect::<Vec<_>>();
    let active_index = specs.iter().position(|spec| spec.focused);
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
        .filter(|(_, spec)| spec.visible)
        .map(|(index, spec)| (spec.output.clone(), index as i64 + 1))
        .collect();

    (
        WorkspaceState {
            workspaces,
            active_id,
            active_monitor_id,
            active_ids,
        },
        names,
    )
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
        match subscribe_to_events(&socket) {
            Ok(mut stream) => {
                if let Err(error) = update_ipc_state(
                    wm_name,
                    &socket,
                    &state,
                    &workspace_names,
                    &persistent_workspaces,
                ) {
                    tracing::warn!(
                        "rubar: could not initialize {wm_name} workspace state: {error}"
                    );
                } else {
                    tracing::debug!(window_manager = wm_name, "window manager IPC ready");
                    match read_window_title(&socket) {
                        Ok(value) => match title.write() {
                            Ok(mut target) => *target = value,
                            Err(error) => tracing::error!(
                                "rubar: could not update {wm_name} window title: {error}"
                            ),
                        },
                        Err(error) => {
                            tracing::warn!(
                                "rubar: could not read {wm_name} focused window: {error}"
                            )
                        }
                    }
                    loop {
                        match ipc_read_message(&mut stream) {
                            Ok((message_type, _payload)) if message_type & 0x8000_0000 != 0 => {
                                let event_type = message_type & 0x7fff_ffff;
                                tracing::trace!(
                                    window_manager = wm_name,
                                    event_type,
                                    "received window manager event"
                                );
                                if event_type == 3 || event_type == 0 {
                                    match read_window_title(&socket) {
                                        Ok(value) => match title.write() {
                                            Ok(mut target) => *target = value,
                                            Err(error) => tracing::error!(
                                                "rubar: could not update {wm_name} window title: {error}"
                                            ),
                                        },
                                        Err(error) => tracing::warn!(
                                            "rubar: could not read {wm_name} focused window: {error}"
                                        ),
                                    }
                                } else {
                                    // Workspace and output events refresh the state below.
                                }
                                if (event_type == 0 || event_type == 1)
                                    && let Err(error) = update_ipc_state(
                                        wm_name,
                                        &socket,
                                        &state,
                                        &workspace_names,
                                        &persistent_workspaces,
                                    )
                                {
                                    tracing::warn!(
                                        "rubar: could not refresh {wm_name} workspace state: {error}"
                                    );
                                }
                            }
                            Ok(_) => {}
                            Err(error) => {
                                tracing::warn!(
                                    "rubar: {wm_name} event socket read failed: {error}"
                                );
                                break;
                            }
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!("rubar: could not subscribe to {wm_name} events: {error}")
            }
        }
        thread::sleep(reconnect_interval);
    }
}

fn subscribe_to_events(socket: &Path) -> Result<UnixStream, String> {
    let mut stream = connect_ipc_socket(socket, Duration::from_secs(2))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| format!("could not set {socket:?} read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| format!("could not set {socket:?} write timeout: {error}"))?;
    ipc_write_message(&mut stream, 2, br#"["workspace","output","window"]"#)?;
    ipc_read_message(&mut stream)?;
    stream
        .set_read_timeout(None)
        .map_err(|error| format!("could not clear {socket:?} read timeout: {error}"))?;
    Ok(stream)
}

fn ipc_request(socket: &Path, message_type: u32, payload: &[u8]) -> Result<Vec<u8>, String> {
    ipc_request_with_timeout(socket, message_type, payload, Duration::from_secs(2))
}

fn ipc_request_with_timeout(
    socket: &Path,
    message_type: u32,
    payload: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let mut stream = connect_ipc_socket(socket, timeout)?;
    stream.set_read_timeout(Some(timeout)).map_err(|error| {
        format!(
            "could not set read timeout for {}: {error}",
            socket.display()
        )
    })?;
    stream.set_write_timeout(Some(timeout)).map_err(|error| {
        format!(
            "could not set write timeout for {}: {error}",
            socket.display()
        )
    })?;
    ipc_write_message(&mut stream, message_type, payload)?;
    ipc_read_message(&mut stream).map(|(_, payload)| payload)
}

fn connect_ipc_socket(socket: &Path, timeout: Duration) -> Result<UnixStream, String> {
    let address = SockAddr::unix(socket)
        .map_err(|error| format!("invalid i3 IPC socket {}: {error}", socket.display()))?;
    let connection = Socket::new(Domain::UNIX, Type::STREAM, None)
        .map_err(|error| format!("could not create i3 IPC socket: {error}"))?;
    connection
        .connect_timeout(&address, timeout)
        .map_err(|error| format!("could not connect to {}: {error}", socket.display()))?;
    let descriptor: OwnedFd = connection.into();
    Ok(UnixStream::from(descriptor))
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

#[cfg(test)]
mod tests {
    use super::{IpcWorkspace, ipc_request_with_timeout, workspace_snapshot};
    use socket2::{Domain, SockAddr, Socket, Type};
    use std::{
        io,
        os::unix::net::UnixListener,
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn ipc_request_times_out_when_accept_queue_is_full() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("i3.sock");
        let address = SockAddr::unix(&path).unwrap();
        let listener = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
        listener.bind(&address).unwrap();
        listener.listen(1).unwrap();

        let mut pending = Vec::new();
        let mut queue_full = false;
        for _ in 0..16 {
            let client = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
            client.set_nonblocking(true).unwrap();
            match client.connect(&address) {
                Ok(()) => pending.push(client),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    queue_full = true;
                    break;
                }
                Err(error) => panic!("could not fill Unix socket accept queue: {error}"),
            }
        }
        assert!(queue_full, "Unix socket accept queue did not fill");

        let start = Instant::now();
        let error = ipc_request_with_timeout(&path, 1, b"", Duration::from_millis(30))
            .expect_err("a full accept queue must not block startup");
        assert!(error.contains("could not connect"), "{error}");
        assert!(start.elapsed() < Duration::from_millis(500));
    }

    #[test]
    fn ipc_request_times_out_when_server_does_not_reply() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("i3.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(150));
        });

        let error = ipc_request_with_timeout(&socket, 1, b"", Duration::from_millis(30))
            .expect_err("an unresponsive IPC server must not block startup");
        assert!(error.contains("could not read i3 IPC header"), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn persistent_workspaces_keep_order_and_focused_output() {
        let raw = vec![
            IpcWorkspace {
                name: "2".into(),
                output: "DP-1".into(),
                focused: true,
                visible: true,
                urgent: false,
            },
            IpcWorkspace {
                name: "web".into(),
                output: "HDMI-1".into(),
                focused: false,
                visible: true,
                urgent: true,
            },
        ];
        let (state, names) = workspace_snapshot(&raw, &["1".into(), "2".into()]);

        assert_eq!(names.get(&1).map(String::as_str), Some("1"));
        assert_eq!(names.get(&2).map(String::as_str), Some("2"));
        assert_eq!(names.get(&3).map(String::as_str), Some("web"));
        assert_eq!(state.workspaces[0].monitor, "DP-1");
        assert_eq!(state.active_id, 2);
        assert_eq!(state.active_ids.get("DP-1"), Some(&2));
        assert!(state.workspaces[2].urgent);
    }
}
