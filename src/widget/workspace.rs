use std::time::Duration;

use crate::config::WorkspaceConfig;

use super::{Widget, WidgetButton, WidgetContent};

pub mod provider;
use provider::{WorkspaceProvider, WorkspaceState};

pub struct Workspace {
    format: String,
    active_color: [u8; 4],
    active_background_color: [u8; 4],
    all_monitors: bool,
    monitor_name: Option<String>,
    workspace_ids: Vec<i64>,
    provider: Box<dyn WorkspaceProvider>,
}

#[derive(Debug, PartialEq, Eq)]
struct RenderedWorkspace {
    id: i64,
    text: String,
    active: bool,
    active_on_monitor: bool,
}

impl Workspace {
    pub fn new(config: &WorkspaceConfig) -> Self {
        let refresh_interval = Duration::from_secs(config.refresh_seconds.max(1));
        Self {
            format: config.format.clone(),
            active_color: parse_color(&config.active_color).unwrap_or_else(|error| {
                eprintln!(
                    "rubar: invalid workspace active_color `{}`: {error}; using #0080ff",
                    config.active_color
                );
                [0, 128, 255, 255]
            }),
            active_background_color: parse_color(&config.active_background_color).unwrap_or_else(
                |error| {
                    eprintln!(
                        "rubar: invalid workspace active_background_color `{}`: {error}; using #414868",
                        config.active_background_color
                    );
                    [65, 72, 104, 255]
                },
            ),
            all_monitors: config.all_monitors,
            monitor_name: None,
            workspace_ids: Vec::new(),
            provider: provider::create(refresh_interval),
        }
    }
}

impl Widget for Workspace {
    fn set_monitor_name(&mut self, monitor_name: Option<&str>) {
        self.monitor_name = monitor_name.map(str::to_owned);
    }

    fn content(&mut self) -> WidgetContent {
        let Some(state) = self.provider.state() else {
            self.workspace_ids.clear();
            return WidgetContent::Buttons(vec![WidgetButton {
                text: self.format.replace("{workspaces}", "--"),
                bold: Some(false),
                color: None,
                background: None,
            }]);
        };

        let active_id = state.active_id.to_string();
        let workspaces = render_workspaces(&state, self.monitor_name.as_deref(), self.all_monitors);
        self.workspace_ids = workspaces.iter().map(|workspace| workspace.id).collect();
        WidgetContent::Buttons(
            workspaces
                .into_iter()
                .map(|workspace| WidgetButton {
                    text: self
                        .format
                        .replace("{workspaces}", &workspace.text)
                        .replace("{active}", &active_id),
                    bold: Some(workspace.active),
                    // Only the active workspace on the active monitor uses the
                    // configured accent color. Other monitors keep the bar's
                    // normal text color, even when their workspace is active.
                    color: if workspace.active_on_monitor {
                        Some(self.active_color)
                    } else {
                        None
                    },
                    background: workspace.active.then_some(self.active_background_color),
                })
                .collect(),
        )
    }

    fn on_click(&mut self, button: super::MouseButton, item: usize) {
        if button != super::MouseButton::Left {
            return;
        }
        let Some(workspace_id) = self.workspace_ids.get(item).copied() else {
            return;
        };
        if let Err(error) = self.provider.switch_to(workspace_id) {
            eprintln!("rubar: could not switch to workspace {workspace_id}: {error}");
        }
    }
}

fn parse_color(value: &str) -> Result<[u8; 4], String> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    let (rgb, alpha) = match hex.len() {
        6 => (hex, 255),
        8 => (
            &hex[..6],
            u8::from_str_radix(&hex[6..], 16).map_err(|error| error.to_string())?,
        ),
        _ => return Err("expected #RRGGBB or #RRGGBBAA".to_string()),
    };
    Ok([
        u8::from_str_radix(&rgb[0..2], 16).map_err(|error| error.to_string())?,
        u8::from_str_radix(&rgb[2..4], 16).map_err(|error| error.to_string())?,
        u8::from_str_radix(&rgb[4..6], 16).map_err(|error| error.to_string())?,
        alpha,
    ])
}

fn render_workspaces(
    state: &WorkspaceState,
    monitor_name: Option<&str>,
    all_monitors: bool,
) -> Vec<RenderedWorkspace> {
    let mut monitor_indices = std::collections::HashMap::new();
    // Each bar should bold the workspace currently shown on its monitor,
    // independently of which monitor is globally focused.
    let active_id = monitor_name
        .and_then(|name| state.active_ids.get(name).copied())
        .unwrap_or(state.active_id);
    // Only the globally focused workspace gets the accent color. A bar on an
    // inactive monitor must not treat that monitor's active workspace as the
    // globally active one.
    let globally_active_id = state.active_id;
    state
        .workspaces
        .iter()
        .filter(|workspace| {
            if let Some(monitor_name) = monitor_name {
                workspace.monitor == monitor_name
            } else {
                all_monitors || workspace.monitor_id == state.active_monitor_id
            }
        })
        .map(|workspace| {
            let index = monitor_indices.entry(workspace.monitor_id).or_insert(0);
            *index += 1;
            let text = if workspace.name.parse::<i64>().is_ok() {
                index.to_string()
            } else {
                workspace.name.clone()
            };
            RenderedWorkspace {
                id: workspace.id,
                text,
                active: workspace.id == active_id,
                active_on_monitor: workspace.id == globally_active_id,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{RenderedWorkspace, WorkspaceState, provider::HyprWorkspace, render_workspaces};

    #[test]
    fn workspaces_are_rendered_with_active_workspace_marked() {
        let state = WorkspaceState {
            workspaces: vec![
                HyprWorkspace {
                    id: 1,
                    name: "1".to_string(),
                    monitor_id: 0,
                    monitor: "DP-1".to_string(),
                },
                HyprWorkspace {
                    id: 2,
                    name: "dev".to_string(),
                    monitor_id: 0,
                    monitor: "DP-1".to_string(),
                },
                HyprWorkspace {
                    id: 11,
                    name: "1".to_string(),
                    monitor_id: 1,
                    monitor: "HDMI-A-1".to_string(),
                },
            ],
            active_id: 2,
            active_monitor_id: 0,
            active_ids: HashMap::from([("DP-1".to_string(), 2), ("HDMI-A-1".to_string(), 11)]),
        };
        assert_eq!(
            render_workspaces(&state, None, false),
            vec![
                RenderedWorkspace {
                    id: 1,
                    text: "1".to_string(),
                    active: false,
                    active_on_monitor: false
                },
                RenderedWorkspace {
                    id: 2,
                    text: "dev".to_string(),
                    active: true,
                    active_on_monitor: true
                },
            ]
        );
        assert_eq!(
            render_workspaces(&state, None, true),
            vec![
                RenderedWorkspace {
                    id: 1,
                    text: "1".to_string(),
                    active: false,
                    active_on_monitor: false
                },
                RenderedWorkspace {
                    id: 2,
                    text: "dev".to_string(),
                    active: true,
                    active_on_monitor: true
                },
                RenderedWorkspace {
                    id: 11,
                    text: "1".to_string(),
                    active: false,
                    active_on_monitor: false
                },
            ]
        );
        assert_eq!(
            render_workspaces(&state, Some("HDMI-A-1"), false),
            vec![RenderedWorkspace {
                id: 11,
                text: "1".to_string(),
                active: true,
                active_on_monitor: false
            }]
        );
    }
}
