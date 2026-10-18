use std::time::{Duration, Instant};

use crate::{
    config::CpuConfig,
    provider::cpu::{self, CpuState},
};

use super::{Widget, WidgetContent};

pub struct Cpu {
    provider: Box<dyn cpu::CpuProvider>,
    format: String,
    refresh_interval: Duration,
    last_refresh: Option<Instant>,
    state: Option<CpuState>,
}

impl Cpu {
    pub fn new(config: &CpuConfig) -> Self {
        let provider = cpu::create(&config.provider).unwrap_or_else(|error| {
            Box::new(cpu::Unavailable { error }) as Box<dyn cpu::CpuProvider>
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

impl Widget for Cpu {
    fn poll_changed(&mut self) -> bool {
        if self
            .last_refresh
            .is_some_and(|last| last.elapsed() < self.refresh_interval)
        {
            return false;
        }
        let state = match self.provider.read() {
            Ok(Some(state)) => Some(state),
            Ok(None) => self.state,
            Err(error) => {
                tracing::warn!("rubar: could not read CPU state: {error}");
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
        WidgetContent::Text(
            self.format
                .replace("{percent}", &state.percent().to_string()),
        )
    }
}
