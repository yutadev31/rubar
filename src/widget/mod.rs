pub mod clock;

pub trait Widget {
    fn text(&self) -> String;
}
