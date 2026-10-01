pub mod clock;
pub mod volume;

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

pub trait Widget {
    fn text(&mut self) -> String;

    fn on_click(&mut self, _button: MouseButton) {}

    fn on_scroll(&mut self, _direction: ScrollDirection) {}
}

pub struct WidgetGroups {
    pub left: Vec<Box<dyn Widget>>,
    pub center: Vec<Box<dyn Widget>>,
    pub right: Vec<Box<dyn Widget>>,
}
