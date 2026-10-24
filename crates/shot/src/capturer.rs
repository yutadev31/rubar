use std::{
    borrow::Cow,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use std::env;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use std::process::{Command, Stdio};

use arboard::{Clipboard, ImageData};
use image::{
    ExtendedColorType, ImageEncoder,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};

#[cfg(all(
    not(any(target_os = "windows", target_os = "macos")),
    feature = "wayland"
))]
use crate::backend::wayland::WaylandBackend;
#[cfg(all(target_os = "windows", feature = "windows"))]
use crate::backend::windows::WindowsBackend;
#[cfg(all(not(any(target_os = "windows", target_os = "macos")), feature = "x11"))]
use crate::backend::x11::X11Backend;
use crate::backend::{Backend, BackendTarget};

pub struct Capturer {
    backend: Box<dyn Backend>,
    output_files: Vec<PathBuf>,
    path_format: Option<String>,
    clipboard: bool,
    region: Option<(u32, u32, u32, u32)>,
}

impl Capturer {
    pub fn new() -> crate::Result<Self> {
        Self::new_for_monitor(0)
    }

    pub fn new_for_monitor(monitor_index: usize) -> crate::Result<Self> {
        Self::with_backend(BackendTarget::Monitor(monitor_index))
    }

    pub fn new_for_monitor_name(name: &str) -> crate::Result<Self> {
        let mut capturer = Self::new_for_monitor_selection()?;
        let names = capturer.monitor_names();
        let monitor_index = names
            .iter()
            .position(|monitor_name| monitor_name == name)
            .ok_or_else(|| crate::Error::MonitorNotFound {
                name: name.to_string(),
                available: names,
            })?;
        capturer.select_monitor(monitor_index)?;
        Ok(capturer)
    }

    pub fn new_for_monitor_selection() -> crate::Result<Self> {
        Self::with_backend(BackendTarget::All)
    }

    pub fn monitor_count(&self) -> usize {
        self.backend.monitor_count()
    }

    pub fn monitor_names(&self) -> Vec<String> {
        self.backend.monitor_names()
    }

    pub fn select_monitor(&mut self, monitor_index: usize) -> crate::Result<()> {
        self.backend.select_monitor(monitor_index)
    }

    pub fn new_for_all_monitors() -> crate::Result<Self> {
        Self::with_backend(BackendTarget::All)
    }

    pub fn capture_output(&mut self) -> crate::Result<()> {
        self.capture(|backend| backend.capture_output())
    }

    pub fn capture_all_outputs(&mut self) -> crate::Result<()> {
        self.capture(|backend| backend.capture_all_outputs())
    }

    fn with_backend(target: BackendTarget) -> crate::Result<Self> {
        Ok(Self {
            backend: Self::create_backend(target)?,
            output_files: Vec::new(),
            path_format: crate::config::OutputConfig::default().path_format,
            clipboard: true,
            region: None,
        })
    }

    pub fn set_output_files(&mut self, files: Vec<PathBuf>) {
        self.output_files = files;
    }

    pub fn set_path_format(&mut self, path_format: Option<String>) {
        self.path_format = path_format;
    }

    pub fn set_clipboard(&mut self, clipboard: bool) {
        self.clipboard = clipboard;
    }

    /// Restrict the saved image to `(x, y, width, height)` pixels within the selected monitor.
    pub fn set_region(&mut self, region: (u32, u32, u32, u32)) -> crate::Result<()> {
        if region.2 == 0 || region.3 == 0 {
            return Err(crate::Error::InvalidRegion);
        }
        self.region = Some(region);
        Ok(())
    }

    fn create_backend(target: BackendTarget) -> crate::Result<Box<dyn Backend>> {
        #[cfg(all(
            not(any(target_os = "windows", target_os = "macos")),
            feature = "wayland"
        ))]
        if env::var_os("WAYLAND_DISPLAY").is_some() {
            return Ok(Box::new(WaylandBackend::initialize(target)?));
        }

        #[cfg(all(not(any(target_os = "windows", target_os = "macos")), feature = "x11"))]
        if env::var_os("DISPLAY").is_some() {
            return Ok(Box::new(X11Backend::initialize(target)?));
        }

        #[cfg(all(target_os = "windows", feature = "windows"))]
        return Ok(Box::new(WindowsBackend::initialize(target)?));

        #[cfg(all(
            not(any(target_os = "windows", target_os = "macos")),
            feature = "wayland",
            feature = "x11"
        ))]
        return Err(crate::Error::DisplayUnavailable(
            "neither WAYLAND_DISPLAY nor DISPLAY is set".to_string(),
        ));

        #[cfg(all(
            not(any(target_os = "windows", target_os = "macos")),
            feature = "wayland",
            not(feature = "x11")
        ))]
        return Err(crate::Error::DisplayUnavailable(
            "WAYLAND_DISPLAY is not set".to_string(),
        ));

        #[cfg(all(
            not(any(target_os = "windows", target_os = "macos")),
            not(feature = "wayland"),
            feature = "x11"
        ))]
        return Err(crate::Error::DisplayUnavailable(
            "DISPLAY is not set".to_string(),
        ));

        #[cfg(not(any(
            all(
                not(any(target_os = "windows", target_os = "macos")),
                any(feature = "wayland", feature = "x11")
            ),
            all(target_os = "windows", feature = "windows")
        )))]
        return Err(crate::Error::DisplayUnavailable(
            "no backend is enabled for this operating system".to_string(),
        ));
    }

    fn capture<F>(&mut self, capture: F) -> crate::Result<()>
    where
        F: FnOnce(&mut dyn Backend) -> crate::Result<crate::frame::Frame>,
    {
        let mut frame = capture(self.backend.as_mut())?;
        if let Some((x, y, width, height)) = self.region {
            frame = crop_frame(frame, x, y, width, height)?;
        }
        let png = encode_png(&frame)?;

        let paths = save_to_files(&png, &self.output_files, self.path_format.as_deref())?;
        for path in &paths {
            println!("Saved screenshot to {}", path.display());
        }
        if self.clipboard {
            if paths.is_empty() {
                copy_png_to_clipboard(&png)?;
            } else if let Some(path) = paths.first() {
                copy_to_clipboard(path)?;
            }
            println!("Copied screenshot to clipboard");
        }

        Ok(())
    }
}

fn crop_frame(
    frame: crate::frame::Frame,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> crate::Result<crate::frame::Frame> {
    if width == 0
        || height == 0
        || x.checked_add(width).is_none_or(|right| right > frame.width)
        || y.checked_add(height)
            .is_none_or(|bottom| bottom > frame.height)
    {
        return Err(crate::Error::InvalidRegion);
    }
    let source_stride = (frame.width as usize)
        .checked_mul(4)
        .ok_or(crate::Error::InvalidImageBuffer)?;
    let output_stride = (width as usize)
        .checked_mul(4)
        .ok_or(crate::Error::InvalidImageBuffer)?;
    let capacity = output_stride
        .checked_mul(height as usize)
        .ok_or(crate::Error::InvalidImageBuffer)?;
    if frame.data.len()
        != source_stride
            .checked_mul(frame.height as usize)
            .ok_or(crate::Error::InvalidImageBuffer)?
    {
        return Err(crate::Error::InvalidImageBuffer);
    }
    let mut data = Vec::with_capacity(capacity);
    for row in y..y + height {
        let start = row as usize * source_stride + x as usize * 4;
        data.extend_from_slice(&frame.data[start..start + output_stride]);
    }
    Ok(crate::frame::Frame {
        data,
        width,
        height,
    })
}

fn encode_png(frame: &crate::frame::Frame) -> crate::Result<Vec<u8>> {
    let mut png = Cursor::new(Vec::new());

    PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Sub).write_image(
        &frame.data,
        frame.width,
        frame.height,
        ExtendedColorType::Rgba8,
    )?;

    Ok(png.into_inner())
}

fn save_to_files(
    png: &[u8],
    files: &[PathBuf],
    path_format: Option<&str>,
) -> crate::Result<Vec<PathBuf>> {
    if !files.is_empty() {
        for path in files {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, png)?;
        }
        return Ok(files.to_vec());
    }

    if path_format.is_none() {
        return Ok(Vec::new());
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            crate::Error::Io(std::io::Error::other(format!(
                "system clock is before Unix epoch: {error}"
            )))
        })?
        .as_millis();
    let path = expand_path_format(path_format.unwrap_or_default(), timestamp)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, png)?;

    Ok(vec![path])
}

fn expand_path_format(path_format: &str, timestamp: u128) -> crate::Result<PathBuf> {
    let mut path = path_format.replace("${timestamp}", &timestamp.to_string());
    for (placeholder, directory, label) in [
        ("${pictures_dir}", dirs::picture_dir(), "Pictures"),
        ("${documents_dir}", dirs::document_dir(), "Documents"),
        ("${downloads_dir}", dirs::download_dir(), "Downloads"),
        ("${home_dir}", dirs::home_dir(), "home"),
    ] {
        if path.contains(placeholder) {
            let directory = directory.ok_or_else(|| {
                crate::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("{label} directory is not available"),
                ))
            })?;
            path = path.replace(placeholder, &directory.to_string_lossy());
        }
    }

    if path.contains("${") {
        return Err(crate::Error::InvalidPathFormat(path_format.to_string()));
    }

    Ok(PathBuf::from(path))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn copy_png_to_clipboard(png: &[u8]) -> crate::Result<()> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| crate::Error::Io(std::io::Error::other(error)))?
        .as_nanos();
    let path = env::temp_dir().join(format!(
        "shot-clipboard-{}-{timestamp}.png",
        std::process::id()
    ));
    fs::write(&path, png)?;

    let executable = env::current_exe()?;
    if let Err(error) = Command::new(executable)
        .arg("--clipboard-daemon-temp")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .current_dir("/")
        .spawn()
    {
        let _ = fs::remove_file(&path);
        return Err(error.into());
    }

    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn copy_png_to_clipboard(png: &[u8]) -> crate::Result<()> {
    set_clipboard_image(png)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn copy_to_clipboard(path: &Path) -> crate::Result<()> {
    let executable = env::current_exe()?;
    Command::new(executable)
        .arg("--clipboard-daemon")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .current_dir("/")
        .spawn()?;

    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn copy_to_clipboard(path: &Path) -> crate::Result<()> {
    let png = fs::read(path)?;
    set_clipboard_image(&png)
}

pub fn serve_clipboard(path: &Path) -> crate::Result<()> {
    let png = fs::read(path)?;
    let image = image_data_from_png(&png)?;

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        use arboard::SetExtLinux;

        let mut clipboard = Clipboard::new().map_err(|_| crate::Error::ClipboardFailed)?;
        clipboard
            .set()
            .wait()
            .image(image)
            .map_err(|_| crate::Error::ClipboardFailed)?;
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        Clipboard::new()
            .map_err(|_| crate::Error::ClipboardFailed)?
            .set_image(image)
            .map_err(|_| crate::Error::ClipboardFailed)?;
    }

    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn set_clipboard_image(png: &[u8]) -> crate::Result<()> {
    let image = image_data_from_png(png)?;

    Clipboard::new()
        .map_err(|_| crate::Error::ClipboardFailed)?
        .set_image(image)
        .map_err(|_| crate::Error::ClipboardFailed)?;

    Ok(())
}

fn image_data_from_png(png: &[u8]) -> crate::Result<ImageData<'static>> {
    let image = image::load_from_memory(png)
        .map_err(|_| crate::Error::ClipboardFailed)?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Ok(ImageData {
        width: width as usize,
        height: height as usize,
        bytes: Cow::Owned(image.into_raw()),
    })
}
