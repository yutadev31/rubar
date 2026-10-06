use crate::config::WindowConfig;

use super::{Widget, WidgetButton, WidgetContent};
use crate::provider::wm::WmClient;
use std::sync::Arc;

pub struct Window {
    format: String,
    padding: u32,
    client: Arc<dyn WmClient>,
}

impl Window {
    pub fn new(config: &WindowConfig, client: Arc<dyn WmClient>) -> Self {
        Self {
            format: config.format.clone(),
            padding: config.padding,
            client,
        }
    }
}

impl Widget for Window {
    fn content(&mut self) -> WidgetContent {
        let title = self.client.title().unwrap_or_default();
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
