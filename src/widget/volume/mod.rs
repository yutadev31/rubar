use std::time::{Duration, Instant};

use crate::config::VolumeConfig;

use super::Widget;

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
}
