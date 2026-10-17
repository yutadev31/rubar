use std::time::{Duration, Instant};

use crate::{
    config::MemoryConfig,
    provider::memory::{self, MemoryState},
};

use super::{Widget, WidgetContent};

pub struct Memory {
    provider: Box<dyn memory::MemoryProvider>,
    format: String,
    refresh_interval: Duration,
    last_refresh: Option<Instant>,
    state: Option<MemoryState>,
}

impl Memory {
    pub fn new(config: &MemoryConfig) -> Self {
        let provider = memory::create(&config.provider).unwrap_or_else(|error| {
            Box::new(memory::Unavailable { error }) as Box<dyn memory::MemoryProvider>
        });
        Self {
            provider,
            format: config.format.clone(),
            refresh_interval: Duration::from_secs(config.refresh_seconds),
            last_refresh: None,
            state: None,
        }
    }
}

impl Widget for Memory {
    fn poll_changed(&mut self) -> bool {
        if self
            .last_refresh
            .is_some_and(|last| last.elapsed() < self.refresh_interval)
        {
            return false;
        }
        let state = match self.provider.read() {
            Ok(state) => Some(state),
            Err(error) => {
                tracing::warn!("rubar: could not read memory state: {error}");
                None
            }
        };
        self.last_refresh = Some(Instant::now());
        if self.state == state {
            return false;
        }
        self.state = state;
        true
    }

    fn content(&mut self) -> WidgetContent {
        let Some(state) = self.state else {
            return WidgetContent::Buttons(Vec::new());
        };
        let gib = |bytes| format!("{:.1}", bytes as f64 / 1_073_741_824.0);
        WidgetContent::Text(
            self.format
                .replace("{used}", &gib(state.used_bytes()))
                .replace("{total}", &gib(state.total_bytes()))
                .replace("{available}", &gib(state.available_bytes()))
                .replace("{percent}", &state.percent().to_string()),
        )
    }
}
