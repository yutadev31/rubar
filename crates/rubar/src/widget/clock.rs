use chrono::{DateTime, Local, Utc};
use chrono_tz::Tz;

use super::{Widget, WidgetContent};
use crate::config::ClockConfig;

#[derive(Debug)]
pub struct Clock {
    format: String,
    timezone: Option<Tz>,
    last_text: Option<String>,
}

impl Clock {
    pub fn new(config: &ClockConfig) -> Self {
        Self {
            format: config.format.clone().unwrap_or_else(|| "%H:%M".to_string()),
            timezone: config.timezone,
            last_text: None,
        }
    }

    fn format_at(&self, now: DateTime<Utc>) -> String {
        match self.timezone {
            Some(timezone) => now
                .with_timezone(&timezone)
                .format(&self.format)
                .to_string(),
            None => now.with_timezone(&Local).format(&self.format).to_string(),
        }
    }
}

impl Widget for Clock {
    fn poll_changed(&mut self) -> bool {
        let text = self.format_at(Utc::now());
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

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::Clock;
    use crate::config::Config;

    #[test]
    fn formats_configured_timezone_across_daylight_saving_transitions() {
        let config =
            Config::parse("[clock]\nformat = '%Y-%m-%d %H:%M %z'\ntimezone = 'America/New_York'\n")
                .unwrap();
        let clock = Clock::new(&config.clock);

        let winter = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let summer = Utc.with_ymd_and_hms(2026, 7, 1, 12, 0, 0).unwrap();
        assert_eq!(clock.format_at(winter), "2026-01-01 07:00 -0500");
        assert_eq!(clock.format_at(summer), "2026-07-01 08:00 -0400");
    }
}
