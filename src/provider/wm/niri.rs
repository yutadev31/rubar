use super::{WmClient, Workspace, WorkspaceState};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{Arc, RwLock},
    thread,
    time::Duration,
};

pub(crate) struct NiriClient {
    socket: PathBuf,
    state: Arc<RwLock<Option<WorkspaceState>>>,
    title: Arc<RwLock<Option<String>>>,
}

#[derive(Debug, Clone, Deserialize)]
struct NiriWorkspace {
    id: u64,
    idx: u8,
    name: Option<String>,
    output: Option<String>,
    #[serde(default)]
    is_urgent: bool,
    #[serde(default)]
    is_active: bool,
    #[serde(default)]
    is_focused: bool,
}

impl NiriClient {
    pub(crate) fn new(reconnect_interval: Duration) -> Result<Self, String> {
        let socket = std::env::var_os("NIRI_SOCKET")
            .map(PathBuf::from)
            .ok_or_else(|| "NIRI_SOCKET is not set".to_string())?;
        if !socket.exists() {
            return Err(format!("niri socket does not exist: {}", socket.display()));
        }

        let state = Arc::new(RwLock::new(None));
        let title = Arc::new(RwLock::new(None));
        let event_socket = socket.clone();
        let shared_state = Arc::clone(&state);
        let shared_title = Arc::clone(&title);
        thread::spawn(move || {
            event_loop(event_socket, shared_state, shared_title, reconnect_interval)
        });

        Ok(Self {
            socket,
            state,
            title,
        })
    }
}

impl WmClient for NiriClient {
    fn state(&self) -> Option<WorkspaceState> {
        self.state.read().ok().and_then(|state| state.clone())
    }

    fn switch_to(
        &self,
        workspace_id: i64,
        _monitor: Option<&str>,
        _local_index: Option<i64>,
    ) -> Result<(), String> {
        let id = u64::try_from(workspace_id)
            .map_err(|_| format!("invalid niri workspace id {workspace_id}"))?;
        let request = json!({"Action": {"FocusWorkspace": {"reference": {"Id": id}}}});
        let response = request_once(&self.socket, &request)?;
        if let Some(error) = response.get("Err") {
            return Err(format!("niri rejected workspace switch: {error}"));
        }
        Ok(())
    }

    fn title(&self) -> Option<String> {
        self.title.read().ok().and_then(|title| title.clone())
    }
}

fn event_loop(
    socket: PathBuf,
    state: Arc<RwLock<Option<WorkspaceState>>>,
    title: Arc<RwLock<Option<String>>>,
    reconnect_interval: Duration,
) {
    loop {
        match stream_events(&socket, &state, &title) {
            Ok(()) => eprintln!("rubar: niri event stream closed; reconnecting"),
            Err(error) => eprintln!("rubar: niri event stream failed: {error}"),
        }
        thread::sleep(reconnect_interval);
    }
}

fn stream_events(
    socket: &PathBuf,
    state: &RwLock<Option<WorkspaceState>>,
    title: &RwLock<Option<String>>,
) -> Result<(), String> {
    let mut stream = UnixStream::connect(socket)
        .map_err(|error| format!("could not connect to niri socket: {error}"))?;
    serde_json::to_writer(&mut stream, &json!("EventStream"))
        .map_err(|error| format!("could not encode niri event request: {error}"))?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|error| error.to_string())?;
        if bytes == 0 {
            return Ok(());
        }
        let value: Value = serde_json::from_str(&line)
            .map_err(|error| format!("could not parse niri event: {error}"))?;
        if value.get("Ok").is_some() {
            continue;
        }
        if let Some(event) = value.get("WorkspacesChanged") {
            match parse_workspaces(event) {
                Ok(next) => {
                    if let Ok(mut current) = state.write() {
                        *current = Some(next);
                    }
                }
                Err(error) => eprintln!("rubar: could not parse niri workspaces: {error}"),
            }
        }
        update_title(&value, title);
    }
}

fn parse_workspaces(event: &Value) -> Result<WorkspaceState, String> {
    let raw: Vec<NiriWorkspace> = serde_json::from_value(
        event
            .get("workspaces")
            .cloned()
            .ok_or("missing workspaces")?,
    )
    .map_err(|error| error.to_string())?;
    let mut outputs: Vec<String> = raw
        .iter()
        .filter_map(|workspace| workspace.output.clone())
        .collect();
    outputs.sort_unstable();
    outputs.dedup();
    let mut active_id = 0;
    let mut active_monitor_id = 0;
    let mut active_ids = HashMap::new();
    let mut workspaces = Vec::with_capacity(raw.len());
    for workspace in raw {
        let id = i64::try_from(workspace.id)
            .map_err(|_| format!("workspace id {} exceeds supported range", workspace.id))?;
        let name = workspace.name.unwrap_or_else(|| workspace.idx.to_string());
        let monitor = workspace.output.unwrap_or_default();
        let monitor_id = outputs.binary_search(&monitor).unwrap_or(0) as i64;
        if workspace.is_focused {
            active_id = id;
            active_monitor_id = monitor_id;
        }
        if workspace.is_active {
            active_ids.insert(monitor.clone(), id);
        }
        workspaces.push(Workspace {
            id,
            name,
            monitor_id,
            monitor,
            urgent: workspace.is_urgent,
        });
    }
    workspaces.sort_by(|a, b| {
        a.monitor.cmp(&b.monitor).then_with(|| {
            a.name
                .parse::<i64>()
                .unwrap_or(i64::MAX)
                .cmp(&b.name.parse::<i64>().unwrap_or(i64::MAX))
        })
    });
    Ok(WorkspaceState {
        workspaces,
        active_id,
        active_monitor_id,
        active_ids,
    })
}

fn update_title(event: &Value, title: &RwLock<Option<String>>) {
    let windows = event
        .get("WindowsChanged")
        .and_then(|v| v.get("windows"))
        .and_then(Value::as_array);
    let changed_window = event
        .get("WindowOpenedOrChanged")
        .and_then(|v| v.get("window"));
    if let Some(value) = windows
        .and_then(|windows| {
            windows
                .iter()
                .find(|window| window.get("is_focused").and_then(Value::as_bool) == Some(true))
        })
        .or_else(|| {
            changed_window
                .filter(|window| window.get("is_focused").and_then(Value::as_bool) == Some(true))
        })
        .and_then(|window| window.get("title"))
        .and_then(Value::as_str)
        && let Ok(mut current) = title.write()
    {
        *current = Some(value.to_string());
    }
    if event
        .get("WindowFocusChanged")
        .is_some_and(|v| v.get("id").is_some_and(Value::is_null))
        && let Ok(mut current) = title.write()
    {
        *current = None;
    }
}

fn request_once(socket: &PathBuf, request: &Value) -> Result<Value, String> {
    let mut stream = UnixStream::connect(socket)
        .map_err(|error| format!("could not connect to niri socket: {error}"))?;
    serde_json::to_writer(&mut stream, request).map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|error| error.to_string())?;
    serde_json::from_str(&response).map_err(|error| format!("could not parse niri reply: {error}"))
}

#[cfg(test)]
mod tests {
    use super::parse_workspaces;

    #[test]
    fn workspace_events_map_outputs_and_active_state() {
        let event = serde_json::json!({"workspaces": [
            {"id": 8, "idx": 2, "name": null, "output": "DP-1", "is_urgent": true, "is_active": true, "is_focused": true},
            {"id": 9, "idx": 1, "name": "web", "output": "HDMI-1", "is_urgent": false, "is_active": true, "is_focused": false}
        ]});
        let state = parse_workspaces(&event).unwrap();
        assert_eq!(state.active_id, 8);
        assert_eq!(
            state.workspaces.iter().find(|w| w.id == 8).unwrap().name,
            "2"
        );
        assert!(state.workspaces.iter().find(|w| w.id == 8).unwrap().urgent);
        assert_ne!(
            state.workspaces[0].monitor_id,
            state.workspaces[1].monitor_id
        );
    }
}
