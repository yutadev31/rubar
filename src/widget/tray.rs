use crate::{
    config::{Config, StyleConfig, TrayConfig},
    provider::tray::{self, MenuPosition, TrayItem, TrayProvider},
    render::ClickContext,
    widget::{MouseButton, TrayIcon, Widget, WidgetButton, WidgetContent},
};

pub struct Tray {
    provider: Box<dyn TrayProvider>,
    items: Vec<TrayItem>,
    icon_size: u32,
    style: StyleConfig,
}

impl Tray {
    pub fn new(config: &TrayConfig, style: StyleConfig) -> Result<Self, String> {
        Ok(Self {
            provider: tray::create(&config.provider)?,
            items: Vec::new(),
            icon_size: config.icon_size.max(1),
            style,
        })
    }
}

impl Widget for Tray {
    fn reload_config(&mut self, config: &Config) {
        self.icon_size = config.tray.icon_size;
        self.style = config.style.clone();
    }

    fn poll_changed(&mut self) -> bool {
        let Some(items) = self.provider.poll_items() else {
            return false;
        };
        if self.items == items {
            return false;
        }
        self.items = items;
        true
    }

    fn content(&mut self) -> WidgetContent {
        WidgetContent::Buttons(
            self.items
                .iter()
                .map(|item| {
                    let icon = if item.needs_attention {
                        item.attention_icon.clone().or_else(|| item.icon.clone())
                    } else {
                        item.icon.clone()
                    }
                    .map(|icon| resize_icon(icon, self.icon_size));
                    WidgetButton {
                        text: None,
                        icon,
                        padding: Some(0),
                        max_width: None,
                        bold: None,
                        color: None,
                        background: None,
                        indicator: None,
                    }
                })
                .collect(),
        )
    }

    fn on_click(&mut self, button: MouseButton, index: usize) {
        let Some(item) = self.items.get(index) else {
            return;
        };
        match button {
            MouseButton::Left => self.provider.activate(&item.id, false),
            MouseButton::Middle => self.provider.activate(&item.id, true),
            _ => {}
        }
    }

    fn on_click_at(&mut self, button: MouseButton, index: usize, context: ClickContext<'_>) {
        if button != MouseButton::Right {
            self.on_click(button, index);
            return;
        }
        let Some(item) = self.items.get(index) else {
            return;
        };
        self.provider.open_menu(
            &item.id,
            MenuPosition {
                button_left: context.button_left,
                surface_width: context.surface_width,
                output: context.output.map(str::to_string),
            },
            &self.style,
        );
    }
}
fn resize_icon(icon: TrayIcon, size: u32) -> TrayIcon {
    if icon.width == 0 || icon.height == 0 || size == 0 {
        return icon;
    }
    let scale = (size as f64 / icon.width as f64).min(size as f64 / icon.height as f64);
    let width = ((icon.width as f64 * scale).round() as u32).clamp(1, size);
    let height = ((icon.height as f64 * scale).round() as u32).clamp(1, size);
    let mut pixels = vec![0; size as usize * size as usize * 4];
    let left = (size - width) / 2;
    let top = (size - height) / 2;
    for y in 0..height {
        for x in 0..width {
            let source_x = (x as u64 * icon.width as u64 / width as u64) as usize;
            let source_y = (y as u64 * icon.height as u64 / height as u64) as usize;
            let source = (source_y * icon.width as usize + source_x) * 4;
            let destination = (((top + y) as usize * size as usize) + (left + x) as usize) * 4;
            if source + 4 <= icon.pixels.len() {
                pixels[destination..destination + 4]
                    .copy_from_slice(&icon.pixels[source..source + 4]);
            }
        }
    }
    TrayIcon {
        width: size,
        height: size,
        pixels,
    }
}
