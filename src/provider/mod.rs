pub(crate) mod audio;
pub(crate) mod battery;
pub(crate) mod cpu;
#[cfg(target_os = "freebsd")]
mod freebsd;
pub(crate) mod memory;
pub(crate) mod tray;
pub(crate) mod wm;
