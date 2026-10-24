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

#[derive(Deserialize)]
struct NiriWindow {
    id: u64,
    title: Option<String>,
    #[serde(default)]
    is_focused: bool,
}

#[derive(Default)]
struct WindowState {
    windows: HashMap<u64, Option<String>>,
    focused_id: Option<u64>,
}

impl NiriClient {
    pub(crate) fn new(reconnect_interval: Duration) -> Result<Self, String> {
        let socket = std::env::var_os("NIRI_SOCKET")
            .map(PathBuf::from)
            .ok_or_else(|| "NIRI_SOCKET is not set".to_string())?;
        if !socket.exists() {
            return Err(format!("niri socket does not exist: {}", socket.display()));
        }
        tracing::debug!(socket = %socket.display(), "connecting to niri event stream");

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
        match self.state.read() {
            Ok(state) => state.clone(),
            Err(error) => {
                tracing::error!("rubar: could not read niri workspace state: {error}");
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
        match self.title.read() {
            Ok(title) => title.clone(),
            Err(error) => {
                tracing::error!("rubar: could not read niri window title: {error}");
                None
            }
        }
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
            Ok(()) => tracing::warn!("rubar: niri event stream closed; reconnecting"),
            Err(error) => tracing::warn!("rubar: niri event stream failed: {error}"),
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
    tracing::debug!(socket = %socket.display(), "connected to niri event stream");
    serde_json::to_writer(&mut stream, &json!("EventStream"))
        .map_err(|error| format!("could not encode niri event request: {error}"))?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut workspace_state = None;
    let mut window_state = WindowState::default();
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
        tracing::trace!(event = ?value, "received niri event");
        if value.get("Ok").is_some() {
            continue;
        }
        match update_workspaces(&value, &mut workspace_state) {
            Ok(true) => match state.write() {
                Ok(mut current) => *current = workspace_state.clone(),
                Err(error) => {
                    tracing::error!("rubar: could not update niri workspace state: {error}")
                }
            },
            Ok(false) => {}
            Err(error) => tracing::warn!("rubar: could not update niri workspaces: {error}"),
        }
        match update_title(&value, &mut window_state) {
            Ok(Some(next)) => match title.write() {
                Ok(mut current) => *current = next,
                Err(error) => tracing::error!("rubar: could not update niri window title: {error}"),
            },
            Ok(None) => {}
            Err(error) => tracing::warn!("rubar: could not update niri window title: {error}"),
        }
    }
}

fn update_workspaces(value: &Value, state: &mut Option<WorkspaceState>) -> Result<bool, String> {
    if let Some(event) = value.get("WorkspacesChanged") {
        *state = Some(parse_workspaces(event)?);
        return Ok(true);
    }
    let Some(current) = state.as_mut() else {
        return Ok(false);
    };
    if let Some(event) = value.get("WorkspaceActivated") {
        let id = event_id(event, "WorkspaceActivated")?;
        let focused = event
            .get("focused")
            .and_then(Value::as_bool)
            .ok_or("WorkspaceActivated has no focused flag")?;
        let workspace = current
            .workspaces
            .iter()
            .find(|workspace| workspace.id == id)
            .ok_or_else(|| format!("WorkspaceActivated references unknown workspace {id}"))?;
        current.active_ids.insert(workspace.monitor.clone(), id);
        if focused {
            current.active_id = id;
            current.active_monitor_id = workspace.monitor_id;
        }
        return Ok(true);
    }
    if let Some(event) = value.get("WorkspaceUrgencyChanged") {
        let id = event_id(event, "WorkspaceUrgencyChanged")?;
        let urgent = event
            .get("urgent")
            .and_then(Value::as_bool)
            .ok_or("WorkspaceUrgencyChanged has no urgent flag")?;
        let workspace = current
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == id)
            .ok_or_else(|| format!("WorkspaceUrgencyChanged references unknown workspace {id}"))?;
        workspace.urgent = urgent;
        return Ok(true);
    }
    Ok(false)
}

fn event_id(event: &Value, kind: &str) -> Result<i64, String> {
    event
        .get("id")
        .and_then(Value::as_u64)
        .and_then(|id| i64::try_from(id).ok())
        .ok_or_else(|| format!("{kind} has no valid workspace id"))
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
            active: false,
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

fn update_title(event: &Value, state: &mut WindowState) -> Result<Option<Option<String>>, String> {
    if let Some(windows) = event.get("WindowsChanged") {
        let windows: Vec<NiriWindow> = serde_json::from_value(
            windows
                .get("windows")
                .cloned()
                .ok_or("WindowsChanged has no windows")?,
        )
        .map_err(|error| format!("invalid WindowsChanged event: {error}"))?;
        state.focused_id = windows
            .iter()
            .find(|window| window.is_focused)
            .map(|window| window.id);
        state.windows = windows
            .into_iter()
            .map(|window| (window.id, window.title))
            .collect();
    } else if let Some(changed) = event.get("WindowOpenedOrChanged") {
        let window: NiriWindow = serde_json::from_value(
            changed
                .get("window")
                .cloned()
                .ok_or("WindowOpenedOrChanged has no window")?,
        )
        .map_err(|error| format!("invalid WindowOpenedOrChanged event: {error}"))?;
        if window.is_focused {
            state.focused_id = Some(window.id);
        }
        state.windows.insert(window.id, window.title);
    } else if let Some(focus) = event.get("WindowFocusChanged") {
        state.focused_id = match focus.get("id") {
            Some(Value::Null) => None,
            Some(id) => Some(id.as_u64().ok_or("WindowFocusChanged has an invalid id")?),
            None => return Err("WindowFocusChanged has no id".to_string()),
        };
    } else if let Some(closed) = event.get("WindowClosed") {
        let id = closed
            .get("id")
            .and_then(Value::as_u64)
            .ok_or("WindowClosed has no valid id")?;
        state.windows.remove(&id);
        if state.focused_id == Some(id) {
            state.focused_id = None;
        }
    } else {
        return Ok(None);
    }
    Ok(Some(
        state
            .focused_id
            .and_then(|id| state.windows.get(&id).cloned().flatten()),
    ))
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
    use super::{WindowState, parse_workspaces, update_title, update_workspaces};
    use serde_json::json;

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

    #[test]
    fn incremental_workspace_events_update_active_and_urgent_state() {
        let mut state = None;
        assert!(update_workspaces(&json!({"WorkspacesChanged": {"workspaces": [
            {"id": 8, "idx": 1, "name": null, "output": "DP-1", "is_active": true, "is_focused": true},
            {"id": 9, "idx": 2, "name": null, "output": "DP-1"},
            {"id": 10, "idx": 1, "name": null, "output": "HDMI-1", "is_active": true}
        ]}}), &mut state).unwrap());

        assert!(
            update_workspaces(
                &json!({"WorkspaceActivated": {"id": 9, "focused": true}}),
                &mut state,
            )
            .unwrap()
        );
        let current = state.as_ref().unwrap();
        assert_eq!(current.active_id, 9);
        assert_eq!(current.active_ids["DP-1"], 9);
        assert_eq!(current.active_ids["HDMI-1"], 10);

        assert!(
            update_workspaces(
                &json!({"WorkspaceUrgencyChanged": {"id": 8, "urgent": true}}),
                &mut state,
            )
            .unwrap()
        );
        assert!(
            state
                .as_ref()
                .unwrap()
                .workspaces
                .iter()
                .any(|workspace| workspace.id == 8 && workspace.urgent)
        );
    }

    #[test]
    fn window_focus_events_resolve_cached_titles() {
        let mut state = WindowState::default();
        assert_eq!(
            update_title(
                &json!({"WindowsChanged": {"windows": [
                    {"id": 1, "title": "Terminal", "is_focused": true},
                    {"id": 2, "title": "Browser", "is_focused": false}
                ]}}),
                &mut state
            )
            .unwrap(),
            Some(Some("Terminal".to_string()))
        );
        assert_eq!(
            update_title(&json!({"WindowFocusChanged": {"id": 2}}), &mut state).unwrap(),
            Some(Some("Browser".to_string()))
        );
        assert_eq!(update_title(&json!({"WindowOpenedOrChanged": {"window": {"id": 2, "title": "New page", "is_focused": true}}}), &mut state).unwrap(), Some(Some("New page".to_string())));
        assert_eq!(
            update_title(&json!({"WindowClosed": {"id": 2}}), &mut state).unwrap(),
            Some(None)
        );
        assert_eq!(
            update_title(&json!({"WindowFocusChanged": {"id": 1}}), &mut state).unwrap(),
            Some(Some("Terminal".to_string()))
        );
        assert_eq!(
            update_title(&json!({"WindowFocusChanged": {"id": null}}), &mut state).unwrap(),
            Some(None)
        );
    }
}
