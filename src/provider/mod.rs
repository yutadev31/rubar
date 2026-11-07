pub(crate) mod audio;
pub(crate) mod battery;
pub(crate) mod cpu;
#[cfg(target_os = "freebsd")]
mod freebsd;
pub(crate) mod memory;
#[cfg(target_os = "netbsd")]
mod netbsd;
pub(crate) mod process;
pub(crate) mod tray;
pub(crate) mod wm;
