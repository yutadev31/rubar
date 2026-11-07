use std::{collections::HashMap, sync::Arc};

use super::{Widget, WidgetButton, WidgetContent};
use crate::{
    color::parse_rgba,
    config::WorkspaceConfig,
    provider::wm::{WmClient, WorkspaceState},
};

pub struct Workspace {
    format: String,
    active_color: [u8; 4],
    urgent_color: [u8; 4],
    active_indicator_height: u32,
    active_indicator_at_top: bool,
    padding: u32,
    spacing: u32,
    monitor_name: Option<String>,
    targets: HashMap<Option<String>, Vec<WorkspaceTarget>>,
    state: Option<WorkspaceState>,
    client: Arc<dyn WmClient>,
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
    urgent: bool,
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
    pub fn new(config: &WorkspaceConfig, client: Arc<dyn WmClient>) -> Self {
        Self {
            format: config.format.clone(),
            active_color: parse_rgba(&config.active_color).unwrap_or_else(|error| {
                tracing::warn!(
                    "rubar: invalid workspace active_color `{}`: {error}; using #0080ff",
                    config.active_color
                );
                [0, 128, 255, 255]
            }),
            active_indicator_height: config.active_indicator_height,
            urgent_color: parse_rgba(&config.urgent_color).unwrap_or_else(|error| {
                tracing::warn!(
                    "rubar: invalid workspace urgent_color `{}`: {error}; using #f7768e",
                    config.urgent_color
                );

                [247, 118, 142, 255]
            }),
            active_indicator_at_top: match config.active_indicator_position.as_str() {
                "top" => true,
                "bottom" => false,
                value => {
                    tracing::warn!(
                        "rubar: invalid workspace active_indicator_position `{value}`; using bottom"
                    );
                    false
                }
            },
            padding: config.padding,
            spacing: config.spacing,
            monitor_name: None,
            targets: HashMap::new(),

            state: None,
            client,
        }
    }
}

impl Widget for Workspace {
    fn poll_changed(&mut self) -> bool {
        let state = self.client.state();

        if self.state == state {
            return false;
        }

        self.state = state;
        true
    }

    fn set_monitor_name(&mut self, monitor_name: Option<&str>) {
        self.monitor_name = monitor_name.map(str::to_owned);
    }

    fn content(&mut self) -> WidgetContent {
        let Some(state) = self.state.as_ref() else {
            self.targets.remove(&self.monitor_name);
            return WidgetContent::Buttons(vec![WidgetButton {
                text: Some(self.format.replace("{workspaces}", "--")),
                icon: None,
                padding: Some(self.padding),
                max_width: None,
                bold: Some(false),
                color: None,
                background: None,
                indicator: None,
            }]);
        };

        let active_id = state.active_id.to_string();
        let workspaces = render_workspaces(state, self.monitor_name.as_deref());
        self.targets.insert(
            self.monitor_name.clone(),
            workspaces.iter().map(WorkspaceTarget::from).collect(),
        );

        WidgetContent::Buttons(
            workspaces
                .into_iter()
                .map(|workspace| {
                    let color = if workspace.urgent {
                        Some(self.urgent_color)
                    } else if workspace.active {
                        Some(self.active_color)
                    } else {
                        None
                    };

                    let text = self
                        .format
                        .replace("{workspaces}", &workspace.text)
                        .replace("{active}", &active_id);

                    WidgetButton {
                        text: Some(text),
                        icon: None,
                        padding: Some(self.padding),
                        max_width: None,
                        bold: Some(workspace.active),
                        color,
                        background: None,
                        indicator: workspace.active_on_monitor.then_some((
                            color.unwrap_or(self.active_color),
                            self.active_indicator_height,
                            self.active_indicator_at_top,
                        )),
                    }
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
            .targets
            .get(&self.monitor_name)
            .and_then(|targets| targets.get(item))
            .cloned();
        let Some(target) = target else {
            return;
        };

        if let Err(error) =
            self.client
                .switch_to(target.id, Some(&target.monitor), target.local_index)
        {
            tracing::warn!(
                "rubar: could not switch to workspace {}: {error}",
                target.id
            );
        }
    }
}

fn render_workspaces(state: &WorkspaceState, monitor_name: Option<&str>) -> Vec<RenderedWorkspace> {
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
        .filter(|workspace| monitor_name.is_none_or(|name| workspace.monitor == name))
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
                active: workspace.active || workspace.id == active_id,
                active_on_monitor: workspace.id == globally_active_id
                    || (workspace.active && workspace.monitor_id == state.active_monitor_id),
                urgent: workspace.urgent,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use super::{RenderedWorkspace, WorkspaceState, render_workspaces};
    use crate::{
        config::WorkspaceConfig,
        provider::wm::{WmClient, Workspace},
        widget::{MouseButton, Widget},
    };

    type Switch = (i64, Option<String>, Option<i64>);

    struct MockClient {
        state: WorkspaceState,
        switches: Mutex<Vec<Switch>>,
    }

    impl WmClient for MockClient {
        fn state(&self) -> Option<WorkspaceState> {
            Some(self.state.clone())
        }

        fn switch_to(
            &self,
            id: i64,
            monitor: Option<&str>,
            local_index: Option<i64>,
        ) -> Result<(), String> {
            self.switches
                .lock()
                .unwrap()
                .push((id, monitor.map(str::to_owned), local_index));
            Ok(())
        }

        fn title(&self) -> Option<String> {
            None
        }
    }

    #[test]
    fn clicks_target_the_workspace_rendered_on_each_monitor() {
        let client = Arc::new(MockClient {
            state: WorkspaceState {
                workspaces: vec![
                    Workspace {
                        id: 1,
                        name: "1".into(),
                        monitor_id: 0,
                        monitor: "DP-1".into(),
                        urgent: false,
                        active: true,
                    },
                    Workspace {
                        id: 11,
                        name: "1".into(),
                        monitor_id: 1,
                        monitor: "HDMI-A-1".into(),
                        urgent: false,
                        active: false,
                    },
                    Workspace {
                        id: 12,
                        name: "2".into(),
                        monitor_id: 1,
                        monitor: "HDMI-A-1".into(),
                        urgent: false,
                        active: true,
                    },
                ],
                active_id: 1,
                active_monitor_id: 0,
                active_ids: HashMap::from([("DP-1".into(), 1), ("HDMI-A-1".into(), 12)]),
            },
            switches: Mutex::new(Vec::new()),
        });
        let mut widget = super::Workspace::new(&WorkspaceConfig::default(), client.clone());
        assert!(widget.poll_changed());
        widget.set_monitor_name(Some("DP-1"));
        widget.content();
        widget.set_monitor_name(Some("HDMI-A-1"));
        widget.content();
        widget.on_click(MouseButton::Right, 1);
        widget.on_click(MouseButton::Left, 2);
        widget.on_click(MouseButton::Left, 1);
        widget.set_monitor_name(Some("DP-1"));
        widget.on_click(MouseButton::Left, 0);
        assert_eq!(
            *client.switches.lock().unwrap(),
            vec![
                (12, Some("HDMI-A-1".into()), Some(2)),
                (1, Some("DP-1".into()), Some(1)),
            ]
        );
    }

    #[test]
    fn workspaces_are_rendered_with_active_workspace_marked() {
        let state = WorkspaceState {
            workspaces: vec![
                Workspace {
                    id: 1,
                    name: "1".to_string(),
                    monitor_id: 0,
                    monitor: "DP-1".to_string(),
                    urgent: false,
                    active: false,
                },
                Workspace {
                    id: 2,
                    name: "dev".to_string(),

                    monitor_id: 0,
                    monitor: "DP-1".to_string(),
                    urgent: false,
                    active: false,
                },
                Workspace {
                    id: 11,
                    name: "1".to_string(),
                    monitor_id: 1,
                    monitor: "HDMI-A-1".to_string(),
                    urgent: false,
                    active: false,
                },
            ],

            active_id: 2,
            active_monitor_id: 0,
            active_ids: HashMap::from([("DP-1".to_string(), 2), ("HDMI-A-1".to_string(), 11)]),
        };
        let first = RenderedWorkspace {
            id: 1,
            monitor: "DP-1".to_string(),
            local_index: Some(1),
            text: "1".to_string(),
            active: false,
            active_on_monitor: false,
            urgent: false,
        };

        let second = RenderedWorkspace {
            id: 2,
            monitor: "DP-1".to_string(),
            local_index: None,
            text: "dev".to_string(),
            active: true,
            active_on_monitor: true,
            urgent: false,
        };

        let third = RenderedWorkspace {
            id: 11,
            monitor: "HDMI-A-1".to_string(),
            local_index: Some(1),
            text: "1".to_string(),
            active: false,
            active_on_monitor: false,
            urgent: false,
        };

        assert_eq!(render_workspaces(&state, None), vec![first, second, third]);
        assert_eq!(
            render_workspaces(&state, Some("HDMI-A-1")),
            vec![RenderedWorkspace {
                id: 11,
                monitor: "HDMI-A-1".to_string(),
                local_index: Some(1),
                text: "1".to_string(),
                active: true,
                active_on_monitor: false,
                urgent: false
            }]
        );
    }

    #[test]
    fn multiple_active_tags_are_marked_on_the_focused_monitor() {
        let state = WorkspaceState {
            workspaces: (1..=2)
                .map(|id| Workspace {
                    id,
                    name: id.to_string(),
                    monitor_id: 1,
                    monitor: "DP-1".to_string(),
                    urgent: false,
                    active: true,
                })
                .collect(),
            active_id: 1,
            active_monitor_id: 1,
            active_ids: HashMap::from([("DP-1".to_string(), 1)]),
        };

        let rendered = render_workspaces(&state, Some("DP-1"));
        assert!(rendered.iter().all(|workspace| workspace.active));
        assert!(rendered.iter().all(|workspace| workspace.active_on_monitor));
    }
}
