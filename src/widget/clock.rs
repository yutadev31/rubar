use chrono::Local;

use super::{Widget, WidgetContent};
use crate::config::ClockConfig;

#[derive(Debug)]
pub struct Clock {
    show_seconds: bool,
}

impl Clock {
    pub fn new(config: &ClockConfig) -> Self {
        Self {
            show_seconds: config.show_seconds,
        }
    }
}

impl Widget for Clock {
    fn content(&mut self) -> WidgetContent {
        let format = if self.show_seconds {
            "%H:%M:%S"
        } else {
            "%H:%M"
        };
        WidgetContent::Text(Local::now().format(format).to_string())
    }
}
