mod backend;
mod capturer;
pub mod config;
mod error;
mod frame;

pub use capturer::{Capturer, serve_clipboard};
pub use error::{Error, Result};
