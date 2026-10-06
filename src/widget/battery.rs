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
    fn content(&mut self) -> WidgetContent {
        let should_refresh = self
            .last_refresh
            .is_none_or(|last| last.elapsed() >= self.refresh_interval);
        if should_refresh {
            self.state = self.provider.read().ok();
            self.last_refresh = Some(Instant::now());
        }
        let Some(state) = self.state else {
            return WidgetContent::Text("󰁹 --".to_string());
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
