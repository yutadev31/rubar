mod menu;
mod sni;

use crate::{config::StyleConfig, widget::TrayIcon};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayItem {
    /// Stable within this provider; used to route clicks back to the item.
    pub id: String,
    pub icon: Option<TrayIcon>,
    pub attention_icon: Option<TrayIcon>,
    pub needs_attention: bool,
}

pub struct MenuPosition {
    pub button_left: i32,
    pub surface_width: u32,
    pub output: Option<String>,
}

pub trait TrayProvider {
    /// Returns the latest complete snapshot when items have changed.
    fn poll_items(&mut self) -> Option<Vec<TrayItem>>;
    fn activate(&self, id: &str, secondary: bool);
    fn open_menu(&self, id: &str, position: MenuPosition, style: &StyleConfig);
}

pub fn validate_name(name: &str) -> Result<(), String> {
    match name {
        "auto" | "sni" => Ok(()),
        _ => Err(format!("unknown tray provider `{name}`")),
    }
}

pub fn create(name: &str) -> Result<Box<dyn TrayProvider>, String> {
    validate_name(name)?;
    Ok(Box::new(sni::Sni::new()))
}
