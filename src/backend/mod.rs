use std::error::Error;

use crate::render::BarRenderer;
use crate::widget::WidgetGroups;

pub mod wayland;

/// Window-system integration boundary. An X11 backend can implement this
/// trait without changing rendering or application startup code.
pub trait Backend {
    fn run(
        &mut self,
        renderer: &mut BarRenderer,
        widgets: &mut WidgetGroups,
    ) -> Result<(), Box<dyn Error>>;
}
