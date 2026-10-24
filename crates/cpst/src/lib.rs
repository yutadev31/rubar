use std::io::{self, Read, Write};

use arboard::{Clipboard, ImageData};

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

enum Content {
    Text(String),
    Image(ImageData<'static>),
}

pub fn copy() -> Result<(), String> {
    let input = read_stdin()?;

    #[cfg(target_os = "linux")]
    {
        parse_content(&input)?;
        spawn_daemon(input)
    }

    #[cfg(not(target_os = "linux"))]
    {
        let content = parse_content(&input)?;
        let mut clipboard =
            Clipboard::new().map_err(|error| format!("failed to open clipboard: {error}"))?;
        match content {
            Content::Text(text) => clipboard.set_text(text),
            Content::Image(image) => clipboard.set_image(image),
        }
        .map_err(|error| format!("failed to set clipboard: {error}"))
    }
}

/// Runs the long-lived Linux clipboard provider used by `cpy` internally.
///
/// This is intentionally not exposed as a user-facing command. Wayland and X11
/// require the process that owns a selection to keep serving requests, so the
/// public `copy` function starts this function in a detached child process.
pub fn run_daemon() -> Result<(), String> {
    let input = read_stdin()?;

    #[cfg(target_os = "linux")]
    {
        use arboard::{LinuxClipboardKind, SetExtLinux};

        let content = parse_content(&input)?;
        drop(input);
        let mut clipboard =
            Clipboard::new().map_err(|error| format!("failed to open clipboard: {error}"))?;
        let operation = clipboard
            .set()
            .clipboard(LinuxClipboardKind::Clipboard)
            .wait();
        match content {
            Content::Text(text) => operation.text(text),
            Content::Image(image) => operation.image(image),
        }
        .map_err(|error| format!("failed to set clipboard: {error}"))
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = input;
        Err("the clipboard daemon is only supported on Linux".to_owned())
    }
}

fn read_stdin() -> Result<Vec<u8>, String> {
    let mut input = Vec::new();
    io::stdin()
        .read_to_end(&mut input)
        .map_err(|error| format!("failed to read stdin: {error}"))?;
    Ok(input)
}

fn parse_content(input: &[u8]) -> Result<Content, String> {
    if input.starts_with(PNG_SIGNATURE) {
        let image = image::load_from_memory_with_format(input, image::ImageFormat::Png)
            .map_err(|error| format!("failed to decode PNG from stdin: {error}"))?
            .into_rgba8();
        let (width, height) = image.dimensions();
        Ok(Content::Image(ImageData {
            width: width as usize,
            height: height as usize,
            bytes: image.into_raw().into(),
        }))
    } else {
        let text = std::str::from_utf8(input)
            .map_err(|_| "stdin is neither UTF-8 text nor a PNG image".to_owned())?;
        Ok(Content::Text(text.to_owned()))
    }
}

#[cfg(target_os = "linux")]
fn spawn_daemon(input: Vec<u8>) -> Result<(), String> {
    use std::process::{Command as ProcessCommand, Stdio};

    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to locate cpy for clipboard daemon: {error}"))?;
    let mut command = ProcessCommand::new(executable);
    command
        .arg("__internal_daemonize")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .current_dir("/");
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start clipboard daemon: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to open clipboard daemon stdin".to_owned())?;
    stdin
        .write_all(&input)
        .map_err(|error| format!("failed to send data to clipboard daemon: {error}"))
}

pub fn paste() -> Result<(), String> {
    let mut clipboard =
        Clipboard::new().map_err(|error| format!("failed to open clipboard: {error}"))?;
    let text = get_text(&mut clipboard)?;
    io::stdout()
        .write_all(text.as_bytes())
        .and_then(|_| io::stdout().flush())
        .map_err(|error| format!("failed to write stdout: {error}"))
}

#[cfg(target_os = "linux")]
fn get_text(clipboard: &mut Clipboard) -> Result<String, String> {
    use arboard::{GetExtLinux, LinuxClipboardKind};

    clipboard
        .get()
        .clipboard(LinuxClipboardKind::Clipboard)
        .text()
        .map_err(|error| format!("failed to read clipboard: {error}"))
}

#[cfg(not(target_os = "linux"))]
fn get_text(clipboard: &mut Clipboard) -> Result<String, String> {
    clipboard
        .get_text()
        .map_err(|error| format!("failed to read clipboard: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};

    #[test]
    fn parses_text() {
        let Content::Text(text) = parse_content("hello 世界".as_bytes()).unwrap() else {
            panic!("expected text");
        };
        assert_eq!(text, "hello 世界");
    }

    #[test]
    fn parses_png_as_rgba_image() {
        let pixels = [255, 0, 0, 128];
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(&pixels, 1, 1, ExtendedColorType::Rgba8)
            .unwrap();

        let Content::Image(image) = parse_content(&png).unwrap() else {
            panic!("expected image");
        };
        assert_eq!((image.width, image.height), (1, 1));
        assert_eq!(image.bytes.as_ref(), pixels);
    }

    #[test]
    fn reports_invalid_input() {
        assert!(
            parse_content(b"\x89PNG\r\n\x1a\ninvalid")
                .err()
                .unwrap()
                .contains("decode PNG")
        );
        assert!(
            parse_content(b"\xff")
                .err()
                .unwrap()
                .contains("neither UTF-8")
        );
    }
}
