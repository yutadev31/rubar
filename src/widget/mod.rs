pub mod clock;
pub mod volume;

pub trait Widget {
    fn text(&mut self) -> String;
}
