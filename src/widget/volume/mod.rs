use std::time::{Duration, Instant};

use crate::config::VolumeConfig;

use super::{MouseButton, ScrollDirection, Widget};

pub mod provider;

pub struct Volume {
    provider: Box<dyn provider::VolumeProvider>,
    format: String,
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
            refresh_interval: Duration::from_secs(config.refresh_seconds.max(1)),
            last_refresh: None,
            state: None,
        }
    }
}

impl Widget for Volume {
    fn text(&mut self) -> String {
        let should_refresh = self
            .last_refresh
            .is_none_or(|last| last.elapsed() >= self.refresh_interval);
        if should_refresh {
            self.state = self.provider.read().ok();
            self.last_refresh = Some(Instant::now());
        }
        let Some(state) = self.state else {
            return "VOL --".to_string();
        };
        self.format
            .replace("{volume}", &state.percent.to_string())
            .replace("{muted}", if state.muted { "muted" } else { "unmuted" })
    }

    fn on_click(&mut self, button: MouseButton) {
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
        state.muted = !state.muted;
        match self.provider.set_muted(state.muted) {
            Ok(()) => {
                self.state = Some(state);
                self.last_refresh = Some(Instant::now());
            }
            Err(error) => eprintln!("rubar: could not set mute: {error}"),
        }
    }

    fn on_scroll(&mut self, direction: ScrollDirection) {
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
        let percent = (state.percent as i16 + change).clamp(0, 100) as u8;
        match self.provider.set_percent(percent) {
            Ok(()) => {
                state.percent = percent;
                self.state = Some(state);
                self.last_refresh = Some(Instant::now());
            }
            Err(error) => eprintln!("rubar: could not set volume to {percent}%: {error}"),
        }
    }
}
