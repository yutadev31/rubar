pub mod clock;
pub mod volume;

pub trait Widget {
    fn text(&mut self) -> String;
}

pub struct WidgetGroups {
    pub left: Vec<Box<dyn Widget>>,
    pub center: Vec<Box<dyn Widget>>,
    pub right: Vec<Box<dyn Widget>>,
}
