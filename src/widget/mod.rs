pub mod battery;
pub mod clock;
pub mod tray;
pub mod volume;
pub mod workspace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Other(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
}

#[derive(Debug, PartialEq, Eq)]
pub enum WidgetContent {
    Text(String),
    Buttons(Vec<WidgetButton>),
}

#[derive(Debug, PartialEq, Eq)]
pub struct WidgetButton {
    pub text: Option<String>,
    pub icon: Option<WidgetIcon>,
    pub padding: Option<u32>,
    pub bold: Option<bool>,
    pub color: Option<[u8; 4]>,
    pub background: Option<[u8; 4]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WidgetIcon {
    pub width: u32,
    pub height: u32,
    /// Premultiplied-independent ARGB bytes, four bytes per pixel.
    pub pixels: Vec<u8>,
}

pub trait Widget {
    fn content(&mut self) -> WidgetContent;

    fn take_redraw_request(&self) -> bool {
        false
    }

    fn set_monitor_name(&mut self, _monitor_name: Option<&str>) {}

    fn on_click(&mut self, _button: MouseButton, _item: usize) {}

    fn on_scroll(&mut self, _direction: ScrollDirection, _item: usize) {}
}

pub struct WidgetGroups {
    pub left: Vec<Box<dyn Widget>>,
    pub center: Vec<Box<dyn Widget>>,
    pub right: Vec<Box<dyn Widget>>,
}

impl WidgetGroups {
    pub fn take_redraw_request(&self) -> bool {
        self.left
            .iter()
            .chain(self.center.iter())
            .chain(self.right.iter())
            .fold(false, |requested, widget| {
                widget.take_redraw_request() || requested
            })
    }

    pub fn set_monitor_name(&mut self, monitor_name: Option<&str>) {
        for widget in self
            .left
            .iter_mut()
            .chain(self.center.iter_mut())
            .chain(self.right.iter_mut())
        {
            widget.set_monitor_name(monitor_name);
        }
    }
}
