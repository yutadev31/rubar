use crate::config::WindowConfig;

use super::{Widget, WidgetButton, WidgetContent};
use crate::provider::wm::WindowProvider;
use std::sync::Arc;

pub struct Window {
    format: String,
    padding: u32,
    provider: Arc<dyn WindowProvider>,
}

impl Window {
    pub fn new(config: &WindowConfig, provider: Arc<dyn WindowProvider>) -> Self {
        Self {
            format: config.format.clone(),
            padding: config.padding,
            provider,
        }
    }
}

impl Widget for Window {
    fn content(&mut self) -> WidgetContent {
        let title = self.provider.title().unwrap_or_default();
        WidgetContent::Buttons(vec![WidgetButton {
            text: Some(self.format.replace("{title}", &title)),
            icon: None,
            padding: Some(self.padding),
            bold: None,
            color: None,
            background: None,
            indicator: None,
        }])
    }
}
