use chrono::Local;

use super::Widget;
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
    fn text(&self) -> String {
        let format = if self.show_seconds {
            "%H:%M:%S"
        } else {
            "%H:%M"
        };
        Local::now().format(format).to_string()
    }
}
