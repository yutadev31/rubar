use chrono::Local;

use super::{Widget, WidgetContent};
use crate::config::ClockConfig;

#[derive(Debug)]
pub struct Clock {
    format: String,
    last_text: Option<String>,
}

impl Clock {
    pub fn new(config: &ClockConfig) -> Self {
        Self {
            format: config.format.clone().unwrap_or_else(|| "%H:%M".to_string()),
            last_text: None,
        }
    }
}

impl Widget for Clock {
    fn poll_changed(&mut self) -> bool {
        let text = Local::now().format(&self.format).to_string();
        if self.last_text.as_deref() == Some(&text) {
            return false;
        }
        self.last_text = Some(text);
        true
    }

    fn content(&mut self) -> WidgetContent {
        WidgetContent::Text(self.last_text.clone().unwrap_or_default())
    }
}
