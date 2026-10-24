use std::time::{Duration, Instant};

use crate::config::BatteryConfig;
use crate::provider::battery as provider;

use super::{Widget, WidgetContent};

pub struct Battery {
    provider: Box<dyn provider::BatteryProvider>,
    format: String,
    refresh_interval: Duration,
    last_refresh: Option<Instant>,
    state: Option<provider::BatteryState>,
}

impl Battery {
    pub fn new(config: &BatteryConfig) -> Self {
        let provider = provider::create(&config.provider).unwrap_or_else(|error| {
            Box::new(provider::Unavailable { error }) as Box<dyn provider::BatteryProvider>
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

impl Widget for Battery {
    fn poll_changed(&mut self) -> bool {
        let should_refresh = self
            .last_refresh
            .is_none_or(|last| last.elapsed() >= self.refresh_interval);
        if should_refresh {
            let state = match self.provider.read() {
                Ok(state) => Some(state),
                Err(error) => {
                    tracing::warn!("rubar: could not read battery state: {error}");
                    None
                }
            };
            self.last_refresh = Some(Instant::now());
            if self.state != state {
                self.state = state;
                return true;
            }
        }
        false
    }

    fn content(&mut self) -> WidgetContent {
        let Some(state) = self.state else {
            return WidgetContent::Buttons(Vec::new());
        };
        let charging = state.status.is_charging();
        WidgetContent::Text(
            self.format
                .replace("{battery}", &state.percent.to_string())
                .replace("{percent}", &state.percent.to_string())
                .replace("{status}", state.status.as_str())
                .replace(
                    "{charging}",
                    if charging { "charging" } else { "discharging" },
                ),
        )
    }
}
