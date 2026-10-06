use chrono::Local;

use super::{Widget, WidgetContent};
use crate::config::ClockConfig;

#[derive(Debug)]
pub struct Clock {
    format: String,
}

impl Clock {
    pub fn new(config: &ClockConfig) -> Self {
        Self {
            format: config.format.clone().unwrap_or_else(|| "%H:%M".to_string()),
        }
    }
}

impl Widget for Clock {
    fn content(&mut self) -> WidgetContent {
        WidgetContent::Text(Local::now().format(&self.format).to_string())
    }
}
