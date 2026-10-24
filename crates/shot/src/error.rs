#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(all(
        not(any(target_os = "windows", target_os = "macos")),
        feature = "wayland"
    ))]
    #[error("unsupported Wayland pixel format: {0:?}")]
    UnsupportedWaylandPixelFormat(wayland_client::protocol::wl_shm::Format),

    #[cfg(all(
        not(any(target_os = "windows", target_os = "macos")),
        feature = "wayland"
    ))]
    #[error("Wayland screencopy capture failed")]
    WaylandScreencopyFailed,

    #[error("failed to read or write data: {0}")]
    Io(#[from] std::io::Error),

    #[cfg(all(
        not(any(target_os = "windows", target_os = "macos")),
        feature = "wayland"
    ))]
    #[error("failed to connect to Wayland compositor: {0}")]
    WaylandConnect(#[from] wayland_client::ConnectError),

    #[cfg(all(
        not(any(target_os = "windows", target_os = "macos")),
        feature = "wayland"
    ))]
    #[error("failed to dispatch Wayland events: {0}")]
    WaylandDispatch(#[from] wayland_client::DispatchError),

    #[cfg(all(
        not(any(target_os = "windows", target_os = "macos")),
        feature = "wayland"
    ))]
    #[error("failed to initialize Wayland globals: {0}")]
    WaylandGlobal(#[from] wayland_client::globals::GlobalError),

    #[cfg(all(
        not(any(target_os = "windows", target_os = "macos")),
        feature = "wayland"
    ))]
    #[error("failed to bind Wayland global: {0}")]
    WaylandBind(#[from] wayland_client::globals::BindError),

    #[error("monitor index {index} is out of range (found {count} monitor(s))")]
    MonitorOutOfRange { index: usize, count: usize },

    #[error("no monitors are available")]
    NoMonitors,

    #[error("monitor {name:?} was not found (available monitors: {available:?})")]
    MonitorNotFound {
        name: String,
        available: Vec<String>,
    },

    #[error("invalid image buffer")]
    InvalidImageBuffer,

    #[error("capture region is outside the selected monitor or has invalid dimensions")]
    InvalidRegion,

    #[error("failed to encode image: {0}")]
    Image(#[from] image::ImageError),

    #[error("failed to parse configuration: {0}")]
    Config(#[from] toml::de::Error),

    #[error("unsupported path format: {0}")]
    InvalidPathFormat(String),

    #[error("failed to access clipboard")]
    ClipboardFailed,

    #[error("no output destination is configured")]
    NoOutputDestination,

    #[error("display server is unavailable: {0}")]
    DisplayUnavailable(String),

    #[error("failed to connect to X11: {0}")]
    X11Connect(String),

    #[error("X11 screen is unavailable: {0}")]
    X11Screen(String),

    #[error("X11 visual is unavailable: {0}")]
    X11Visual(String),

    #[error("X11 image operation failed: {0}")]
    X11Image(String),

    #[error("Windows screen capture failed: {0}")]
    WindowsCapture(String),
}

pub type Result<T> = std::result::Result<T, Error>;
