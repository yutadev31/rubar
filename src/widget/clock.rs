use chrono::Local;

use super::Widget;

#[derive(Debug, Default)]
pub struct Clock;

impl Widget for Clock {
    fn text(&self) -> String {
        Local::now().format("%H:%M:%S").to_string()
    }
}
