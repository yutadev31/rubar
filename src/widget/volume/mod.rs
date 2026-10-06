use crate::config::VolumeConfig;

use super::{MouseButton, ScrollDirection, Widget, WidgetButton, WidgetContent};

pub mod provider;

pub struct Volume {
    provider: Box<dyn provider::VolumeProvider>,
    format_out: String,
    format_out_muted: String,
    format_in: String,
    format_in_muted: String,
    state: Option<provider::VolumeState>,
}

impl Volume {
    pub fn new(config: &VolumeConfig) -> Self {
        let provider = provider::create(&config.provider).unwrap_or_else(|error| {
            Box::new(provider::Unavailable { error }) as Box<dyn provider::VolumeProvider>
        });
        Self {
            provider,
            format_out: config.format_out.clone(),
            format_out_muted: config.format_out_muted.clone(),
            format_in: config.format_in.clone(),
            format_in_muted: config.format_in_muted.clone(),
            state: None,
        }
    }
}

impl Widget for Volume {
    fn content(&mut self) -> WidgetContent {
        if let Ok(state) = self.provider.read() {
            self.state = Some(state);
        }

        let Some(state) = self.state else {
            return WidgetContent::Buttons(Vec::new());
        };

        let mut buttons = Vec::new();
        if let Some(output) = state.output {
            buttons.push(volume_button(format_volume(
                &self.format_out,
                &self.format_out_muted,
                output.percent,
                output.muted,
            )));
        }

        if let Some(input) = state.input {
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

        let mut state = match self.current_state("mute") {
            Some(state) => state,
            None => return,
        };

        let muted = if item == 1 {
            let Some(input) = state.input.as_mut() else {
                return;
            };
            input.muted = !input.muted;
            input.muted
        } else {
            let Some(output) = state.output.as_mut() else {
                return;
            };
            output.muted = !output.muted;
            output.muted
        };

        let result = if item == 1 {
            self.provider.set_in_muted(muted)
        } else {
            self.provider.set_muted(muted)
        };

        match result {
            Ok(()) => {
                self.state = Some(state);
            }
            Err(error) => eprintln!("rubar: could not set mute: {error}"),
        }
    }

    fn on_scroll(&mut self, direction: ScrollDirection, item: usize) {
        let mut state = match self.current_state("scroll") {
            Some(state) => state,
            None => return,
        };

        let change = match direction {
            ScrollDirection::Up => 1,
            ScrollDirection::Down => -1,
        };

        let percent = if item == 1 {
            let Some(input) = state.input else {
                return;
            };
            (input.percent as i16 + change).clamp(0, 100) as u8
        } else {
            let Some(output) = state.output else {
                return;
            };
            (output.percent as i16 + change).clamp(0, 100) as u8
        };

        let result = if item == 1 {
            self.provider.set_in_percent(percent)
        } else {
            self.provider.set_percent(percent)
        };

        match result {
            Ok(()) => {
                if item == 1 {
                    if let Some(input) = state.input.as_mut() {
                        input.percent = percent;
                    }
                } else {
                    if let Some(output) = state.output.as_mut() {
                        output.percent = percent;
                    }
                }
                self.state = Some(state);
            }
            Err(error) => eprintln!("rubar: could not set volume to {percent}%: {error}"),
        }
    }
}

impl Volume {
    fn current_state(&mut self, action: &str) -> Option<provider::VolumeState> {
        match self.state {
            Some(state) => Some(state),
            None => match self.provider.read() {
                Ok(state) => Some(state),
                Err(error) => {
                    eprintln!("rubar: could not read volume before {action}: {error}");
                    None
                }
            },
        }
    }
}

fn volume_button(text: String) -> WidgetButton {
    WidgetButton {
        text: Some(text),
        icon: None,
        padding: None,
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
