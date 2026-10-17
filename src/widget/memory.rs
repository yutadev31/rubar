use std::time::{Duration, Instant};

use crate::{
    config::MemoryConfig,
    provider::memory::{MemoryState, ProcMeminfo},
};

use super::{Widget, WidgetContent};

pub struct Memory {
    provider: ProcMeminfo,
    format: String,
    refresh_interval: Duration,
    last_refresh: Option<Instant>,
    state: Option<MemoryState>,
}

impl Memory {
    pub fn new(config: &MemoryConfig) -> Self {
        Self {
            provider: ProcMeminfo::new("/proc/meminfo"),
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
        let gib = |kib| format!("{:.1}", kib as f64 / 1_048_576.0);
        WidgetContent::Text(
            self.format
                .replace("{used}", &gib(state.used_kib()))
                .replace("{total}", &gib(state.total_kib))
                .replace("{available}", &gib(state.available_kib))
                .replace("{percent}", &state.percent().to_string()),
        )
    }
}
