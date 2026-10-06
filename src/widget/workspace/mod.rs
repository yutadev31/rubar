use crate::config::WorkspaceConfig;

use super::{Widget, WidgetButton, WidgetContent};

pub mod provider;
use provider::{WorkspaceProvider, WorkspaceState};

pub struct Workspace {
    format: String,
    active_color: [u8; 4],
    active_indicator_height: u32,
    active_indicator_at_top: bool,
    padding: u32,
    spacing: u32,
    all_monitors: bool,
    monitor_name: Option<String>,
    workspace_targets: Vec<WorkspaceTarget>,
    provider: Box<dyn WorkspaceProvider>,
}

#[derive(Debug, Clone)]
struct WorkspaceTarget {
    id: i64,
    monitor: String,
    local_index: Option<i64>,
}

#[derive(Debug, PartialEq, Eq)]
struct RenderedWorkspace {
    id: i64,
    monitor: String,
    local_index: Option<i64>,
    text: String,
    active: bool,
    active_on_monitor: bool,
}

impl From<&RenderedWorkspace> for WorkspaceTarget {
    fn from(workspace: &RenderedWorkspace) -> Self {
        Self {
            id: workspace.id,
            monitor: workspace.monitor.clone(),
            local_index: workspace.local_index,
        }
    }
}

impl Workspace {
    pub fn new(config: &WorkspaceConfig) -> Self {
        Self {
            format: config.format.clone(),
            active_color: parse_color(&config.active_color).unwrap_or_else(|error| {
                eprintln!(
                    "rubar: invalid workspace active_color `{}`: {error}; using #0080ff",
                    config.active_color
                );
                [0, 128, 255, 255]
            }),
            active_indicator_height: config.active_indicator_height,
            active_indicator_at_top: match config.active_indicator_position.as_str() {
                "top" => true,
                "bottom" => false,
                value => {
                    eprintln!(
                        "rubar: invalid workspace active_indicator_position `{value}`; using bottom"
                    );
                    false
                }
            },
            padding: config.padding,
            spacing: config.spacing,
            all_monitors: config.all_monitors,
            monitor_name: None,
            workspace_targets: Vec::new(),
            provider: provider::create(&config.persistent_workspaces),
        }
    }
}

impl Widget for Workspace {
    fn set_monitor_name(&mut self, monitor_name: Option<&str>) {
        self.monitor_name = monitor_name.map(str::to_owned);
    }

    fn content(&mut self) -> WidgetContent {
        let Some(state) = self.provider.state() else {
            self.workspace_targets.clear();
            return WidgetContent::Buttons(vec![WidgetButton {
                text: Some(self.format.replace("{workspaces}", "--")),
                icon: None,
                padding: Some(self.padding),
                bold: Some(false),
                color: None,
                background: None,
                indicator: None,
            }]);
        };

        let active_id = state.active_id.to_string();
        let workspaces = render_workspaces(&state, self.monitor_name.as_deref(), self.all_monitors);
        self.workspace_targets = workspaces.iter().map(WorkspaceTarget::from).collect();
        WidgetContent::Buttons(
            workspaces
                .into_iter()
                .map(|workspace| WidgetButton {
                    text: Some(
                        self.format
                            .replace("{workspaces}", &workspace.text)
                            .replace("{active}", &active_id),
                    ),
                    icon: None,
                    padding: Some(self.padding),
                    bold: Some(workspace.active),
                    color: if workspace.active {
                        Some(self.active_color)
                    } else {
                        None
                    },
                    background: None,
                    indicator: workspace.active_on_monitor.then_some((
                        self.active_color,
                        self.active_indicator_height,
                        self.active_indicator_at_top,
                    )),
                })
                .collect(),
        )
    }

    fn content_spacing(&self) -> Option<u32> {
        Some(self.spacing)
    }

    fn on_click(&mut self, button: super::MouseButton, item: usize) {
        if button != super::MouseButton::Left {
            return;
        }
        let target = self
            .provider
            .state()
            .and_then(|state| {
                render_workspaces(&state, self.monitor_name.as_deref(), false)
                    .get(item)
                    .map(|workspace| WorkspaceTarget::from(workspace))
            })
            .or_else(|| self.workspace_targets.get(item).cloned());
        let Some(target) = target else {
            return;
        };
        if let Err(error) =
            self.provider
                .switch_to(target.id, Some(&target.monitor), target.local_index)
        {
            eprintln!(
                "rubar: could not switch to workspace {}: {error}",
                target.id
            );
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
            let local_index = workspace.name.parse::<i64>().ok().map(|_| *index);
            let text = if local_index.is_some() {
                index.to_string()
            } else {
                workspace.name.clone()
            };
            RenderedWorkspace {
                id: workspace.id,
                monitor: workspace.monitor.clone(),
                local_index,
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
                    monitor: "DP-1".to_string(),
                    local_index: Some(1),
                    text: "1".to_string(),
                    active: false,
                    active_on_monitor: false
                },
                RenderedWorkspace {
                    id: 2,
                    monitor: "DP-1".to_string(),
                    local_index: None,
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
                    monitor: "DP-1".to_string(),
                    local_index: Some(1),
                    text: "1".to_string(),
                    active: false,
                    active_on_monitor: false
                },
                RenderedWorkspace {
                    id: 2,
                    monitor: "DP-1".to_string(),
                    local_index: None,
                    text: "dev".to_string(),
                    active: true,
                    active_on_monitor: true
                },
                RenderedWorkspace {
                    id: 11,
                    monitor: "HDMI-A-1".to_string(),
                    local_index: Some(1),
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
                monitor: "HDMI-A-1".to_string(),
                local_index: Some(1),
                text: "1".to_string(),
                active: true,
                active_on_monitor: false
            }]
        );
    }
}
