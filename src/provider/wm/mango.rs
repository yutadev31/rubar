use super::{WmClient, Workspace, WorkspaceState};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{Arc, RwLock},
    thread,
    time::Duration,
};

#[derive(Deserialize)]
struct MonitorsResponse {
    monitors: Vec<MangoMonitor>,
}

#[derive(Deserialize)]
struct MangoMonitor {
    name: String,
    active: bool,
    tags: Vec<MangoTag>,
}

#[derive(Deserialize)]
struct MangoTag {
    index: i64,
    is_active: bool,
    is_urgent: bool,
}

pub(crate) struct MangoClient {
    socket: PathBuf,
    state: Arc<RwLock<Option<WorkspaceState>>>,
    title: Arc<RwLock<Option<String>>>,
}

impl MangoClient {
    pub(crate) fn new(reconnect_interval: Duration) -> Result<Self, String> {
        let socket = std::env::var_os("MANGO_INSTANCE_SIGNATURE")
            .map(PathBuf::from)
            .ok_or_else(|| "MANGO_INSTANCE_SIGNATURE is not set".to_string())?;
        UnixStream::connect(&socket).map_err(|error| {
            format!(
                "could not connect to MangoWM socket {}: {error}",
                socket.display()
            )
        })?;

        let state = Arc::new(RwLock::new(None));
        let title = Arc::new(RwLock::new(None));

        let state_socket = socket.clone();
        let shared_state = Arc::clone(&state);
        thread::spawn(move || {
            watch_events(
                &state_socket,
                "watch all-monitors",
                reconnect_interval,
                |value| match parse_monitors(value) {
                    Ok(next) => match shared_state.write() {
                        Ok(mut current) => *current = Some(next),
                        Err(error) => tracing::error!(
                            "rubar: could not update MangoWM workspace state: {error}"
                        ),
                    },
                    Err(error) => {
                        tracing::warn!("rubar: could not parse MangoWM monitors: {error}")
                    }
                },
            );
        });

        let title_socket = socket.clone();
        let shared_title = Arc::clone(&title);
        thread::spawn(move || {
            watch_events(
                &title_socket,
                "watch focusing-client",
                reconnect_interval,
                |value| match parse_title(&value) {
                    Ok(next) => match shared_title.write() {
                        Ok(mut current) => *current = next,
                        Err(error) => {
                            tracing::error!("rubar: could not update MangoWM window title: {error}")
                        }
                    },
                    Err(error) => {
                        tracing::warn!("rubar: could not parse MangoWM focused client: {error}")
                    }
                },
            );
        });

        Ok(Self {
            socket,
            state,
            title,
        })
    }
}

impl WmClient for MangoClient {
    fn state(&self) -> Option<WorkspaceState> {
        match self.state.read() {
            Ok(state) => state.clone(),
            Err(error) => {
                tracing::error!("rubar: could not read MangoWM workspace state: {error}");
                None
            }
        }
    }

    fn switch_to(
        &self,
        workspace_id: i64,
        monitor: Option<&str>,
        _local_index: Option<i64>,
    ) -> Result<(), String> {
        let monitor = monitor.ok_or("MangoWM workspace has no monitor")?;
        let tag = workspace_id % 32;
        if !(1..=31).contains(&tag) {
            return Err(format!("invalid MangoWM tag {tag}"));
        }
        if monitor.contains([',', '\n', '\r']) {
            return Err(format!("invalid MangoWM monitor name {monitor:?}"));
        }
        let response = request(
            &self.socket,
            &format!("dispatch viewcrossmon,{tag},{monitor}"),
        )?;
        if response.get("success") == Some(&Value::Bool(true)) {
            Ok(())
        } else {
            Err(format!(
                "MangoWM rejected tag {tag} on {monitor}: {response}"
            ))
        }
    }

    fn title(&self) -> Option<String> {
        match self.title.read() {
            Ok(title) => title.clone(),
            Err(error) => {
                tracing::error!("rubar: could not read MangoWM window title: {error}");
                None
            }
        }
    }
}

fn parse_monitors(value: Value) -> Result<WorkspaceState, String> {
    let MonitorsResponse { mut monitors } = serde_json::from_value(value)
        .map_err(|error| format!("invalid all-monitors response: {error}"))?;
    monitors.sort_by(|left, right| left.name.cmp(&right.name));

    let mut workspaces = Vec::new();
    let mut active_ids = HashMap::new();
    let mut active_id = 0;
    let mut active_monitor_id = 0;
    for (index, monitor) in monitors.into_iter().enumerate() {
        let monitor_id = index as i64 + 1;
        if monitor.active {
            active_monitor_id = monitor_id;
        }
        for tag in monitor.tags {
            if !(1..=31).contains(&tag.index) {
                return Err(format!(
                    "invalid tag index {} on {}",
                    tag.index, monitor.name
                ));
            }
            let id = monitor_id * 32 + tag.index;
            if tag.is_active {
                active_ids.entry(monitor.name.clone()).or_insert(id);
                if monitor.active && active_id == 0 {
                    active_id = id;
                }
            }
            workspaces.push(Workspace {
                id,
                name: tag.index.to_string(),
                monitor_id,
                monitor: monitor.name.clone(),
                urgent: tag.is_urgent,
                active: tag.is_active,
            });
        }
    }
    workspaces.sort_by_key(|workspace| (workspace.monitor_id, workspace.id));
    Ok(WorkspaceState {
        workspaces,
        active_id,
        active_monitor_id,
        active_ids,
    })
}

fn parse_title(value: &Value) -> Result<Option<String>, String> {
    match value.get("title") {
        Some(Value::String(title)) => Ok(Some(title.clone())),
        Some(Value::Null) => Ok(None),
        Some(_) => Err(format!("invalid title in focused-client response: {value}")),
        None => Err(format!("missing title in focused-client response: {value}")),
    }
}

fn watch_events(
    socket: &PathBuf,
    command: &str,
    reconnect_interval: Duration,
    mut on_event: impl FnMut(Value),
) {
    loop {
        match watch_once(socket, command, &mut on_event) {
            Ok(()) => tracing::warn!("rubar: MangoWM {command} stream closed; reconnecting"),
            Err(error) => tracing::warn!("rubar: MangoWM {command} stream failed: {error}"),
        }
        thread::sleep(reconnect_interval);
    }
}

fn watch_once(
    socket: &PathBuf,
    command: &str,
    on_event: &mut impl FnMut(Value),
) -> Result<(), String> {
    let mut stream = UnixStream::connect(socket)
        .map_err(|error| format!("could not connect to {}: {error}", socket.display()))?;
    stream
        .write_all(format!("{command}\n").as_bytes())
        .map_err(|error| format!("could not send {command}: {error}"))?;
    for line in BufReader::new(stream).lines() {
        let line = line.map_err(|error| format!("could not read {command}: {error}"))?;
        let value = serde_json::from_str(&line)
            .map_err(|error| format!("could not parse {command} response: {error}"))?;
        on_event(value);
    }
    Ok(())
}

fn request(socket: &PathBuf, command: &str) -> Result<Value, String> {
    let mut stream = UnixStream::connect(socket)
        .map_err(|error| format!("could not connect to {}: {error}", socket.display()))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| format!("could not set MangoWM IPC timeout: {error}"))?;
    stream
        .write_all(format!("{command}\n").as_bytes())
        .map_err(|error| format!("could not send {command}: {error}"))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| format!("could not read {command} response: {error}"))?;
    serde_json::from_str(&line)
        .map_err(|error| format!("could not parse {command} response: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{MangoClient, parse_monitors, parse_title};
    use crate::provider::wm::WmClient;
    use serde_json::json;
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::net::UnixListener,
        sync::{Arc, RwLock},
        thread,
    };

    #[test]
    fn maps_tags_on_multiple_monitors() {
        let state = parse_monitors(json!({"monitors": [
            {"name": "HDMI-A-1", "active": false, "tags": [
                {"index": 1, "is_active": true, "is_urgent": false},
                {"index": 2, "is_active": false, "is_urgent": true}
            ]},
            {"name": "DP-1", "active": true, "tags": [
                {"index": 1, "is_active": true, "is_urgent": false},
                {"index": 2, "is_active": true, "is_urgent": false}
            ]}
        ]}))
        .unwrap();
        assert_eq!(state.workspaces.len(), 4);
        assert_eq!(state.active_monitor_id, 1);
        assert_eq!(state.active_id, 33);
        assert_eq!(state.active_ids["HDMI-A-1"], 65);
        assert!(state.workspaces[1].active);
        assert!(state.workspaces[3].urgent);
    }

    #[test]
    fn focused_client_may_be_empty() {
        assert_eq!(
            parse_title(&json!({"title": "Editor"})).unwrap().as_deref(),
            Some("Editor")
        );
        assert_eq!(parse_title(&json!({"title": null})).unwrap(), None);
        assert!(parse_title(&json!({"error": "bad request"})).is_err());
    }

    #[test]
    fn switch_uses_monitor_and_checks_reply() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("mango.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = thread::spawn(move || {
            for response in [r#"{"success":true}"#, r#"{"error":"unknown monitor"}"#] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut command = String::new();
                BufReader::new(&stream).read_line(&mut command).unwrap();
                assert_eq!(command, "dispatch viewcrossmon,2,DP-1\n");
                writeln!(stream, "{response}").unwrap();
            }
        });
        let client = MangoClient {
            socket,
            state: Arc::new(RwLock::new(None)),
            title: Arc::new(RwLock::new(None)),
        };
        assert!(client.switch_to(34, Some("DP-1"), Some(2)).is_ok());
        assert!(client.switch_to(34, Some("DP-1"), Some(2)).is_err());
        server.join().unwrap();
    }
}
