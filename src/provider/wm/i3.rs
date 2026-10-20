use super::{WmClient, Workspace, WorkspaceState};
use serde::Deserialize;
use socket2::{Domain, SockAddr, SockRef, Socket, Type};
use std::{
    collections::HashMap,
    io::{ErrorKind, Read, Write},
    mem::MaybeUninit,
    os::{fd::OwnedFd, unix::net::UnixStream},
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

const STATE_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const FAILED_REFRESH_INTERVAL: Duration = Duration::from_secs(1);
const TITLE_RETRY_INTERVAL: Duration = Duration::from_secs(5);

/// Workspace provider for i3 and Sway, which share the i3 IPC protocol.
pub(crate) struct I3Client {
    wm_name: &'static str,
    socket: PathBuf,
    state: Arc<RwLock<Option<WorkspaceState>>>,
    workspace_names: Arc<RwLock<HashMap<i64, String>>>,
    title: Arc<RwLock<TitleState>>,
}

#[derive(Default)]
struct TitleState {
    value: Option<String>,
    generation: u64,
}

struct EventState {
    workspaces: Arc<RwLock<Option<WorkspaceState>>>,
    workspace_names: Arc<RwLock<HashMap<i64, String>>>,
    persistent_workspaces: Vec<String>,
    title: Arc<RwLock<TitleState>>,
    title_requests: SyncSender<()>,
}

#[derive(Deserialize)]
struct IpcWindowEvent {
    change: String,
    container: IpcWindowContainer,
}

#[derive(Deserialize)]
struct IpcWindowContainer {
    name: Option<String>,
    #[serde(default)]
    focused: bool,
}

#[derive(Deserialize)]
struct IpcWorkspaceEvent {
    change: String,
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
        let title = Arc::new(RwLock::new(TitleState::default()));
        tracing::debug!(window_manager = wm_name, socket = %socket.display(), "starting window manager IPC worker");

        let event_socket = socket.clone();
        let shared_state = Arc::clone(&state);
        let shared_names = Arc::clone(&workspace_names);
        let event_persistent_workspaces = persistent_workspaces.clone();
        let (title_requests, title_events) = sync_channel(1);
        let title_socket = socket.clone();
        let shared_title = Arc::clone(&title);
        thread::spawn(move || {
            title_loop(
                wm_name,
                title_socket,
                shared_title,
                title_events,
                TITLE_RETRY_INTERVAL,
            )
        });
        let event_title = Arc::clone(&title);
        let event_wm_name = wm_name;
        thread::spawn(move || {
            ipc_event_loop(
                event_wm_name,
                event_socket,
                EventState {
                    workspaces: shared_state,
                    workspace_names: shared_names,
                    persistent_workspaces: event_persistent_workspaces,
                    title: event_title,
                    title_requests,
                },
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
            Ok(title) => title.value.clone(),
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
            // A missing workspace is placed on the focused output so it can
            // be created by clicking its button on that output's bar.
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
                active: false,
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
    event_state: EventState,
    reconnect_interval: Duration,
) {
    let EventState {
        workspaces: state,
        workspace_names,
        persistent_workspaces,
        title,
        title_requests,
    } = event_state;
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
                    request_title_update(wm_name, &title, &title_requests);
                    // Reconcile state even if an event is missed or a snapshot fails.
                    let mut next_refresh = Instant::now() + STATE_REFRESH_INTERVAL;
                    loop {
                        let now = Instant::now();
                        if now >= next_refresh {
                            let interval = refresh_ipc_state(
                                wm_name,
                                &socket,
                                &state,
                                &workspace_names,
                                &persistent_workspaces,
                            );
                            next_refresh = Instant::now() + interval;
                            continue;
                        }
                        match read_event_until(&mut stream, next_refresh - now) {
                            Ok(Some((message_type, payload)))
                                if message_type & 0x8000_0000 != 0 =>
                            {
                                let event_type = message_type & 0x7fff_ffff;
                                tracing::trace!(
                                    window_manager = wm_name,
                                    event_type,
                                    "received window manager event"
                                );
                                if event_type == 0 || event_type == 1 {
                                    let interval = refresh_ipc_state(
                                        wm_name,
                                        &socket,
                                        &state,
                                        &workspace_names,
                                        &persistent_workspaces,
                                    );
                                    next_refresh = Instant::now() + interval;
                                }
                                handle_title_event(
                                    wm_name,
                                    event_type,
                                    &payload,
                                    &title,
                                    &title_requests,
                                );
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

fn handle_title_event(
    wm_name: &str,
    event_type: u32,
    payload: &[u8],
    title: &RwLock<TitleState>,
    requests: &SyncSender<()>,
) {
    match event_type {
        0 => match serde_json::from_slice::<IpcWorkspaceEvent>(payload) {
            Ok(event) if matches!(event.change.as_str(), "focus" | "empty") => {
                request_title_update(wm_name, title, requests)
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!("rubar: could not parse {wm_name} workspace event: {error}");
                request_title_update(wm_name, title, requests);
            }
        },
        3 => match serde_json::from_slice::<IpcWindowEvent>(payload) {
            Ok(event) => match event.change.as_str() {
                "focus" => set_title_from_event(wm_name, title, event.container.name),
                "title" if event.container.focused => {
                    set_title_from_event(wm_name, title, event.container.name)
                }
                "close" => request_title_update(wm_name, title, requests),
                "move" if event.container.focused => request_title_update(wm_name, title, requests),
                _ => {}
            },
            Err(error) => {
                tracing::warn!("rubar: could not parse {wm_name} window event: {error}");
                request_title_update(wm_name, title, requests);
            }
        },
        _ => {}
    }
}

fn set_title_from_event(wm_name: &str, title: &RwLock<TitleState>, value: Option<String>) {
    match title.write() {
        Ok(mut target) => {
            target.generation = target.generation.wrapping_add(1);
            target.value = value;
        }
        Err(error) => tracing::error!("rubar: could not update {wm_name} window title: {error}"),
    }
}

fn request_title_update(wm_name: &str, title: &RwLock<TitleState>, requests: &SyncSender<()>) {
    match title.write() {
        Ok(mut target) => target.generation = target.generation.wrapping_add(1),
        Err(error) => {
            tracing::error!("rubar: could not mark {wm_name} window title stale: {error}")
        }
    }
    match requests.try_send(()) {
        Ok(()) | Err(TrySendError::Full(())) => {}
        Err(TrySendError::Disconnected(())) => {
            tracing::error!("rubar: {wm_name} window title worker stopped");
        }
    }
}

fn title_loop(
    wm_name: &str,
    socket: PathBuf,
    title: Arc<RwLock<TitleState>>,
    requests: Receiver<()>,
    retry_interval: Duration,
) {
    let mut failed_generation = None;
    loop {
        if let Some(failed) = failed_generation.take() {
            thread::sleep(retry_interval);
            match requests.try_recv() {
                Ok(()) => {}
                Err(TryRecvError::Empty) => match title.read() {
                    Ok(current) if current.generation != failed => continue,
                    Ok(_) => {}
                    Err(error) => {
                        tracing::error!(
                            "rubar: could not read {wm_name} window title state: {error}"
                        );
                        continue;
                    }
                },
                Err(TryRecvError::Disconnected) => break,
            }
        } else if requests.recv().is_err() {
            break;
        }
        // One lookup observes the latest title for all pending events.
        while requests.try_recv().is_ok() {}
        let generation = match title.read() {
            Ok(current) => current.generation,
            Err(error) => {
                tracing::error!("rubar: could not read {wm_name} window title state: {error}");
                continue;
            }
        };
        match read_window_title(&socket) {
            Ok(value) => match title.write() {
                Ok(mut target) if target.generation == generation => target.value = value,
                Ok(_) => {}
                Err(error) => {
                    tracing::error!("rubar: could not update {wm_name} window title: {error}")
                }
            },
            Err(error) => {
                tracing::warn!("rubar: could not read {wm_name} focused window: {error}");
                failed_generation = Some(generation);
            }
        }
    }
}

fn refresh_ipc_state(
    wm_name: &str,
    socket: &Path,
    state: &Arc<RwLock<Option<WorkspaceState>>>,
    workspace_names: &Arc<RwLock<HashMap<i64, String>>>,
    persistent_workspaces: &[String],
) -> Duration {
    match update_ipc_state(
        wm_name,
        socket,
        state,
        workspace_names,
        persistent_workspaces,
    ) {
        Ok(()) => STATE_REFRESH_INTERVAL,
        Err(error) => {
            tracing::warn!("rubar: could not refresh {wm_name} workspace state: {error}");
            FAILED_REFRESH_INTERVAL
        }
    }
}

fn read_event_until(
    stream: &mut UnixStream,
    timeout: Duration,
) -> Result<Option<(u32, Vec<u8>)>, String> {
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| format!("could not set i3 event read timeout: {error}"))?;
    // An idle timeout must not consume any part of the next IPC message.
    match SockRef::from(&*stream).peek(&mut [MaybeUninit::uninit()]) {
        Ok(0) => Err("i3 event stream closed".to_string()),
        Ok(_) => ipc_read_message(stream).map(Some),
        Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            Ok(None)
        }
        Err(error) => Err(format!("could not read i3 event stream: {error}")),
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
    Ok(focused_window_title(&tree))
}

fn focused_window_title(node: &serde_json::Value) -> Option<String> {
    let is_window = node.get("window").is_some_and(serde_json::Value::is_number)
        || node.get("shell").is_some_and(serde_json::Value::is_string)
        || node.get("app_id").is_some_and(serde_json::Value::is_string);
    if is_window && node.get("focused").and_then(serde_json::Value::as_bool) == Some(true) {
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
        .find_map(focused_window_title)
}

#[cfg(test)]
mod tests {
    use super::{
        IpcWorkspace, STATE_REFRESH_INTERVAL, TitleState, focused_window_title, handle_title_event,
        ipc_read_message, ipc_request_with_timeout, ipc_write_message, read_event_until,
        refresh_ipc_state, request_title_update, title_loop, workspace_snapshot,
    };
    use socket2::{Domain, SockAddr, Socket, Type};
    use std::{
        collections::HashMap,
        io::{self, Write},
        os::unix::net::{UnixListener, UnixStream},
        sync::{Arc, RwLock, mpsc::sync_channel},
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
    fn idle_event_timeout_preserves_the_next_event() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        assert!(
            read_event_until(&mut reader, Duration::from_millis(30))
                .unwrap()
                .is_none()
        );

        writer.write_all(b"i3-ipc").unwrap();
        writer.write_all(&2_u32.to_le_bytes()).unwrap();
        writer.write_all(&0x8000_0000_u32.to_le_bytes()).unwrap();
        writer.write_all(b"{}").unwrap();
        let (message_type, payload) = read_event_until(&mut reader, Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert_eq!(message_type, 0x8000_0000);
        assert_eq!(payload, b"{}");
    }

    #[test]
    fn slow_title_reply_does_not_delay_workspace_refresh() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("i3.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let title = Arc::new(RwLock::new(TitleState::default()));
        let (requests, events) = sync_channel(1);
        let title_socket = socket.clone();
        let worker_title = Arc::clone(&title);
        let worker = thread::spawn(move || {
            title_loop(
                "i3",
                title_socket,
                worker_title,
                events,
                Duration::from_millis(30),
            )
        });
        request_title_update("i3", &title, &requests);

        let (mut stalled_title_reply, _) = listener.accept().unwrap();
        assert_eq!(ipc_read_message(&mut stalled_title_reply).unwrap().0, 4);
        let responder = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            assert_eq!(ipc_read_message(&mut stream).unwrap().0, 1);
            ipc_write_message(
                &mut stream,
                1,
                br#"[{"name":"1","output":"HDMI-1","focused":true,"visible":true}]"#,
            )
            .unwrap();
        });

        let state = Arc::new(RwLock::new(None));
        let names = Arc::new(RwLock::new(HashMap::new()));
        let start = Instant::now();
        let interval = refresh_ipc_state("i3", &socket, &state, &names, &[]);
        assert_eq!(interval, STATE_REFRESH_INTERVAL);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(state.read().unwrap().as_ref().unwrap().active_id, 1);

        handle_title_event(
            "i3",
            3,
            br#"{"change":"focus","container":{"name":"New","focused":true}}"#,
            &title,
            &requests,
        );
        ipc_write_message(
            &mut stalled_title_reply,
            4,
            br#"{"nodes":[{"window":42,"focused":true,"name":"Old"}]}"#,
        )
        .unwrap();
        drop(stalled_title_reply);
        drop(requests);
        responder.join().unwrap();
        worker.join().unwrap();
        assert_eq!(title.read().unwrap().value.as_deref(), Some("New"));
    }

    #[test]
    fn title_events_skip_unrelated_tree_requests() {
        let title = RwLock::new(TitleState::default());
        let (requests, pending) = sync_channel(1);
        handle_title_event(
            "i3",
            3,
            br#"{"change":"focus","container":{"name":"Terminal","focused":true}}"#,
            &title,
            &requests,
        );
        handle_title_event(
            "i3",
            3,
            br#"{"change":"title","container":{"name":"Other","focused":false}}"#,
            &title,
            &requests,
        );
        handle_title_event("i3", 0, br#"{"change":"rename"}"#, &title, &requests);
        assert_eq!(title.read().unwrap().value.as_deref(), Some("Terminal"));
        assert!(pending.try_recv().is_err());

        handle_title_event(
            "i3",
            3,
            br#"{"change":"title","container":{"name":"Shell","focused":true}}"#,
            &title,
            &requests,
        );
        assert_eq!(title.read().unwrap().value.as_deref(), Some("Shell"));
        assert!(pending.try_recv().is_err());

        handle_title_event("i3", 0, br#"{"change":"focus"}"#, &title, &requests);
        assert!(pending.try_recv().is_ok());
    }

    #[test]
    fn focused_window_title_ignores_empty_workspace_and_supports_sway() {
        let empty = serde_json::json!({
            "type": "workspace", "name": "1", "focused": true, "window": null,
            "nodes": [], "floating_nodes": []
        });
        assert_eq!(focused_window_title(&empty), None);

        let i3 = serde_json::json!({
            "type": "workspace", "focused": false,
            "nodes": [{"type": "con", "name": "Terminal", "focused": true, "window": 42}]
        });
        assert_eq!(focused_window_title(&i3).as_deref(), Some("Terminal"));

        let sway = serde_json::json!({
            "type": "workspace", "focused": false,
            "nodes": [{"type": "con", "name": "Editor", "focused": true,
                "window": null, "shell": "xdg_shell"}]
        });
        assert_eq!(focused_window_title(&sway).as_deref(), Some("Editor"));
    }

    #[test]
    fn title_lookup_retries_after_an_ipc_failure() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("i3.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            assert_eq!(ipc_read_message(&mut first).unwrap().0, 4);
            drop(first);

            listener.set_nonblocking(true).unwrap();
            let deadline = Instant::now() + Duration::from_secs(1);
            let mut second = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("title lookup was not retried: {error}"),
                }
            };
            assert_eq!(ipc_read_message(&mut second).unwrap().0, 4);
            ipc_write_message(
                &mut second,
                4,
                br#"{"nodes":[{"window":42,"focused":true,"name":"Recovered"}]}"#,
            )
            .unwrap();
        });

        let title = Arc::new(RwLock::new(TitleState::default()));
        let (requests, events) = sync_channel(1);
        let worker_title = Arc::clone(&title);
        let worker = thread::spawn(move || {
            title_loop(
                "i3",
                socket,
                worker_title,
                events,
                Duration::from_millis(30),
            )
        });
        request_title_update("i3", &title, &requests);
        server.join().unwrap();
        drop(requests);
        worker.join().unwrap();
        assert_eq!(title.read().unwrap().value.as_deref(), Some("Recovered"));
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
