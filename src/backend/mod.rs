use std::error::Error;

use crate::widget::WidgetGroups;
use crate::{config::TrayConfig, render::BarRenderer};

pub mod wayland;
pub mod x11;
pub(crate) mod x11_tray;

/// Window-system integration boundary. An X11 backend can implement this
/// trait without changing rendering or application startup code.
pub trait Backend {
    fn run(
        &mut self,
        renderer: &mut BarRenderer,
        widgets: &mut WidgetGroups,
        tray: &TrayConfig,
    ) -> Result<(), Box<dyn Error>>;
}
