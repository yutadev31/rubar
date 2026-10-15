use std::{sync::mpsc, thread, time::Duration};

use crate::config::VolumeConfig;
use crate::provider::audio as provider;

use super::{MouseButton, ScrollDirection, Widget, WidgetButton, WidgetContent};

pub struct Volume {
    commands: mpsc::Sender<VolumeCommand>,
    updates: mpsc::Receiver<VolumeUpdate>,
    format_out: String,
    format_out_muted: String,
    format_in: String,
    format_in_muted: String,
    scroll_step: u8,
    state: Option<provider::VolumeState>,
    visible_channels: Vec<bool>,
    revision: u64,
}

struct VolumeCommand {
    revision: u64,
    action: VolumeAction,
}

enum VolumeAction {
    SetMuted { input: bool, muted: bool },
    SetPercent { input: bool, percent: u8 },
}

struct VolumeUpdate {
    revision: u64,
    state: provider::VolumeState,
}

impl Volume {
    pub fn new(config: &VolumeConfig) -> Self {
        let (commands, command_receiver) = mpsc::channel();
        let (state_sender, updates) = mpsc::channel();
        let name = config.provider.clone();
        if let Err(error) = thread::Builder::new()
            .name("rubar-audio".to_string())
            .spawn(move || run_audio_worker(&name, command_receiver, state_sender))
        {
            tracing::error!("rubar: could not start audio worker: {error}");
        }
        Self {
            commands,
            updates,
            format_out: config.format_out.clone(),
            format_out_muted: config.format_out_muted.clone(),
            format_in: config.format_in.clone(),
            format_in_muted: config.format_in_muted.clone(),
            scroll_step: config.scroll_step,
            state: None,
            visible_channels: Vec::new(),
            revision: 0,
        }
    }
}

impl Widget for Volume {
    fn poll_changed(&mut self) -> bool {
        let mut changed = false;
        while let Ok(update) = self.updates.try_recv() {
            if update.revision >= self.revision && self.state != Some(update.state) {
                self.state = Some(update.state);
                changed = true;
            }
        }
        changed
    }

    fn content(&mut self) -> WidgetContent {
        self.visible_channels.clear();
        let Some(state) = self.state else {
            return WidgetContent::Buttons(Vec::new());
        };
        let mut buttons = Vec::new();
        if let Some(output) = state.output {
            self.visible_channels.push(false);
            buttons.push(volume_button(format_volume(
                &self.format_out,
                &self.format_out_muted,
                output.percent,
                output.muted,
            )));
        }
        if let Some(input) = state.input {
            self.visible_channels.push(true);
            buttons.push(volume_button(format_volume(
                &self.format_in,
                &self.format_in_muted,
                input.percent,
                input.muted,
            )));
        }
        WidgetContent::Buttons(buttons)
    }

    fn on_click(&mut self, button: MouseButton, item: usize) {
        if button != MouseButton::Left {
            return;
        }
        let Some(input) = self.visible_channels.get(item).copied() else {
            return;
        };
        let Some(mut state) = self.state else {
            return;
        };
        let Some(channel) = (if input {
            &mut state.input
        } else {
            &mut state.output
        })
        .as_mut() else {
            return;
        };
        channel.muted = !channel.muted;
        let revision = self.revision.wrapping_add(1);
        if let Err(error) = self.commands.send(VolumeCommand {
            revision,
            action: VolumeAction::SetMuted {
                input,
                muted: channel.muted,
            },
        }) {
            tracing::warn!("rubar: could not queue mute change: {error}");
        } else {
            self.revision = revision;
            self.state = Some(state);
        }
    }

    fn on_scroll(&mut self, direction: ScrollDirection, item: usize) {
        let Some(input) = self.visible_channels.get(item).copied() else {
            return;
        };
        let Some(mut state) = self.state else {
            return;
        };
        let Some(channel) = (if input {
            &mut state.input
        } else {
            &mut state.output
        })
        .as_mut() else {
            return;
        };
        let change = match direction {
            ScrollDirection::Up => i16::from(self.scroll_step),
            ScrollDirection::Down => -i16::from(self.scroll_step),
        };
        channel.percent = (channel.percent as i16 + change).clamp(0, 100) as u8;
        let revision = self.revision.wrapping_add(1);
        if let Err(error) = self.commands.send(VolumeCommand {
            revision,
            action: VolumeAction::SetPercent {
                input,
                percent: channel.percent,
            },
        }) {
            tracing::warn!("rubar: could not queue volume change: {error}");
        } else {
            self.revision = revision;
            self.state = Some(state);
        }
    }
}

fn run_audio_worker(
    name: &str,
    commands: mpsc::Receiver<VolumeCommand>,
    updates: mpsc::Sender<VolumeUpdate>,
) {
    let mut provider = match provider::create(name) {
        Ok(provider) => provider,
        Err(error) => {
            tracing::error!("rubar: could not initialize audio provider: {error}");
            return;
        }
    };
    let mut last_state = None;
    let mut last_error: Option<String> = None;
    let mut processed_revision = 0;
    loop {
        match provider.read() {
            Ok(state) => {
                last_error = None;
                if last_state != Some(state) {
                    last_state = Some(state);
                    if updates
                        .send(VolumeUpdate {
                            revision: processed_revision,
                            state,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            }
            Err(error) => {
                if last_error.as_deref() != Some(error.as_str()) {
                    tracing::warn!("rubar: could not read audio state: {error}");
                    last_error = Some(error);
                }
            }
        }
        match commands.recv_timeout(Duration::from_millis(100)) {
            Ok(command) => {
                processed_revision = command.revision;
                let result = match command.action {
                    VolumeAction::SetMuted {
                        input: false,
                        muted,
                    } => provider.set_muted(muted),
                    VolumeAction::SetMuted { input: true, muted } => provider.set_in_muted(muted),
                    VolumeAction::SetPercent {
                        input: false,
                        percent,
                    } => provider.set_percent(percent),
                    VolumeAction::SetPercent {
                        input: true,
                        percent,
                    } => provider.set_in_percent(percent),
                };
                if let Err(error) = result {
                    tracing::warn!("rubar: could not change audio state: {error}");
                    if let Some(state) = last_state
                        && updates
                            .send(VolumeUpdate {
                                revision: processed_revision,
                                state,
                            })
                            .is_err()
                    {
                        return;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn volume_button(text: String) -> WidgetButton {
    WidgetButton {
        text: Some(text),
        icon: None,
        padding: None,
        max_width: None,
        bold: None,
        color: None,
        background: None,
        indicator: None,
    }
}

fn format_volume(format: &str, muted_format: &str, percent: u8, muted: bool) -> String {
    let format = if muted { muted_format } else { format };
    format
        .replace("{volume}", &percent.to_string())
        .replace("{muted}", if muted { " [M]" } else { "" })
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::{ScrollDirection, Volume, VolumeAction, VolumeUpdate, Widget};
    use crate::{
        config::VolumeConfig,
        provider::audio::{VolumeChannelState, VolumeState},
    };

    #[test]
    fn input_only_button_controls_input() {
        let (commands, command_receiver) = mpsc::channel();
        let (state_sender, updates) = mpsc::channel();
        let mut config = VolumeConfig::default();
        config.scroll_step = 5;
        let mut widget = Volume {
            commands,
            updates,
            format_out: config.format_out,
            format_out_muted: config.format_out_muted,
            format_in: config.format_in,
            format_in_muted: config.format_in_muted,
            scroll_step: config.scroll_step,
            state: None,
            visible_channels: Vec::new(),
            revision: 0,
        };
        state_sender
            .send(VolumeUpdate {
                revision: 0,
                state: VolumeState {
                    output: None,
                    input: Some(VolumeChannelState {
                        percent: 40,
                        muted: false,
                    }),
                },
            })
            .unwrap();
        widget.poll_changed();
        widget.content();
        widget.on_scroll(ScrollDirection::Up, 0);
        assert!(matches!(
            command_receiver.try_recv().unwrap().action,
            VolumeAction::SetPercent {
                input: true,
                percent: 45
            }
        ));
        state_sender
            .send(VolumeUpdate {
                revision: 0,
                state: VolumeState {
                    output: None,
                    input: Some(VolumeChannelState {
                        percent: 40,
                        muted: false,
                    }),
                },
            })
            .unwrap();
        assert!(!widget.poll_changed());
        assert_eq!(widget.state.unwrap().input.unwrap().percent, 45);
    }
}
