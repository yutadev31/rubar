use std::time::{Duration, Instant};

use crate::config::VolumeConfig;

use super::{MouseButton, ScrollDirection, Widget, WidgetButton, WidgetContent};

pub mod provider;

pub struct Volume {
    provider: Box<dyn provider::VolumeProvider>,
    format: String,
    muted_format: String,
    microphone_format: String,
    microphone_muted_format: String,
    refresh_interval: Duration,
    last_refresh: Option<Instant>,
    state: Option<provider::VolumeState>,
}

impl Volume {
    pub fn new(config: &VolumeConfig) -> Self {
        let provider = provider::create(&config.provider).unwrap_or_else(|error| {
            Box::new(provider::Unavailable { error }) as Box<dyn provider::VolumeProvider>
        });
        Self {
            provider,
            format: config.format.clone(),
            muted_format: config.muted_format.clone(),
            microphone_format: config.microphone_format.clone(),
            microphone_muted_format: config.microphone_muted_format.clone(),
            refresh_interval: Duration::from_secs(config.refresh_seconds.max(1)),
            last_refresh: None,
            state: None,
        }
    }
}

impl Widget for Volume {
    fn content(&mut self) -> WidgetContent {
        let should_refresh = self
            .last_refresh
            .is_none_or(|last| last.elapsed() >= self.refresh_interval);
        if should_refresh {
            self.state = self.provider.read().ok();
            self.last_refresh = Some(Instant::now());
        }
        let Some(state) = self.state else {
            return WidgetContent::Text("VOL --".to_string());
        };
        let mut buttons = vec![WidgetButton {
            text: format_volume(&self.format, &self.muted_format, state.percent, state.muted),
            bold: None,
            color: None,
            background: None,
        }];
        if let Some(microphone) = state.microphone {
            buttons.push(WidgetButton {
                text: format_volume(
                    &self.microphone_format,
                    &self.microphone_muted_format,
                    microphone.percent,
                    microphone.muted,
                ),
                bold: None,
                color: None,
                background: None,
            });
        }
        WidgetContent::Buttons(buttons)
    }

    fn on_click(&mut self, button: MouseButton, item: usize) {
        if button != MouseButton::Left {
            return;
        }
        let mut state = match self.state {
            Some(state) => state,
            None => match self.provider.read() {
                Ok(state) => state,
                Err(error) => {
                    eprintln!("rubar: could not read volume before mute: {error}");
                    return;
                }
            },
        };
        let muted = if item == 1 {
            let Some(microphone) = state.microphone.as_mut() else {
                return;
            };
            microphone.muted = !microphone.muted;
            microphone.muted
        } else {
            state.muted = !state.muted;
            state.muted
        };
        let result = if item == 1 {
            self.provider.set_microphone_muted(muted)
        } else {
            self.provider.set_muted(muted)
        };
        match result {
            Ok(()) => {
                self.state = Some(state);
                self.last_refresh = Some(Instant::now());
            }
            Err(error) => eprintln!("rubar: could not set mute: {error}"),
        }
    }

    fn on_scroll(&mut self, direction: ScrollDirection, item: usize) {
        let mut state = match self.state {
            Some(state) => state,
            None => match self.provider.read() {
                Ok(state) => state,
                Err(error) => {
                    eprintln!("rubar: could not read volume before scroll: {error}");
                    return;
                }
            },
        };
        let change = match direction {
            ScrollDirection::Up => 1,
            ScrollDirection::Down => -1,
        };
        let percent = if item == 1 {
            let Some(microphone) = state.microphone else {
                return;
            };
            (microphone.percent as i16 + change).clamp(0, 100) as u8
        } else {
            (state.percent as i16 + change).clamp(0, 100) as u8
        };
        let result = if item == 1 {
            self.provider.set_microphone_percent(percent)
        } else {
            self.provider.set_percent(percent)
        };
        match result {
            Ok(()) => {
                if item == 1 {
                    if let Some(microphone) = state.microphone.as_mut() {
                        microphone.percent = percent;
                    }
                } else {
                    state.percent = percent;
                }
                self.state = Some(state);
                self.last_refresh = Some(Instant::now());
            }
            Err(error) => eprintln!("rubar: could not set volume to {percent}%: {error}"),
        }
    }
}

fn format_volume(format: &str, muted_format: &str, percent: u8, muted: bool) -> String {
    let format = if muted { muted_format } else { format };
    format
        .replace("{volume}", &percent.to_string())
        .replace("{muted}", if muted { " [M]" } else { "" })
}
