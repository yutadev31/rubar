use std::time::{Duration, Instant};

use crate::{
    config::ProcessConfig,
    provider::process::{self, ProcessProvider},
};

use super::{Widget, WidgetContent};

pub struct Process {
    provider: Box<dyn ProcessProvider>,
    format: String,
    refresh_interval: Duration,
    last_refresh: Option<Instant>,
    count: Option<usize>,
}

impl Process {
    pub fn new(config: &ProcessConfig) -> Self {
        let provider = process::create(&config.provider).unwrap_or_else(|error| {
            Box::new(process::Unavailable { error }) as Box<dyn ProcessProvider>
        });
        Self {
            provider,
            format: config.format.clone(),
            refresh_interval: Duration::from_secs(config.refresh_seconds),
            last_refresh: None,
            count: None,
        }
    }
}

impl Widget for Process {
    fn poll_changed(&mut self) -> bool {
        if self
            .last_refresh
            .is_some_and(|last| last.elapsed() < self.refresh_interval)
        {
            return false;
        }
        let count = match self.provider.read() {
            Ok(count) => Some(count),
            Err(error) => {
                tracing::warn!("rubar: could not read process count: {error}");
                None
            }
        };
        self.last_refresh = Some(Instant::now());
        if self.count == count {
            return false;
        }
        self.count = count;
        true
    }

    fn content(&mut self) -> WidgetContent {
        let Some(count) = self.count else {
            return WidgetContent::Buttons(Vec::new());
        };
        WidgetContent::Text(self.format.replace("{count}", &count.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedCount;

    impl ProcessProvider for FixedCount {
        fn read(&mut self) -> Result<usize, String> {
            Ok(42)
        }
    }

    #[test]
    fn renders_count_and_only_reports_changes_when_value_changes() {
        let mut widget = Process {
            provider: Box::new(FixedCount),
            format: "Processes: {count}".to_string(),
            refresh_interval: Duration::ZERO,
            last_refresh: None,
            count: None,
        };
        assert!(widget.poll_changed());
        assert!(matches!(widget.content(), WidgetContent::Text(text) if text == "Processes: 42"));
        assert!(!widget.poll_changed());
    }
}
