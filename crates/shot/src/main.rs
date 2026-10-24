use std::path::PathBuf;

use clap::Parser;
use shot::{Capturer, serve_clipboard};

mod selector;

#[derive(Debug, Parser)]
#[command(name = "shot", about = "Capture a screenshot from a monitor")]
struct Cli {
    /// Monitor ID to capture (for example HDMI-A-1); choose interactively when omitted
    #[arg(short = 'm', long, value_name = "ID")]
    monitor: Option<String>,

    /// Capture and stitch all monitors into one image
    #[arg(short = 'a', long, conflicts_with = "region")]
    all: bool,

    /// Capture a rectangle on the selected monitor: X,Y,WIDTH,HEIGHT (pixels)
    #[arg(long, value_name = "X,Y,WIDTH,HEIGHT", value_parser = parse_region, conflicts_with = "all")]
    region: Option<(u32, u32, u32, u32)>,

    /// Select a monitor using rofi
    #[arg(short = 'r', long, conflicts_with_all = ["monitor", "all"])]
    rofi: bool,

    /// Save the screenshot to this path; may be specified multiple times
    #[arg(long = "file", value_name = "PATH")]
    files: Vec<PathBuf>,

    /// Copy the screenshot to the clipboard (use --clipboard false to disable)
    #[arg(long, value_name = "BOOL", num_args = 0..=1, default_missing_value = "true")]
    clipboard: Option<bool>,

    #[arg(long, hide = true, value_name = "PATH")]
    clipboard_daemon: Option<PathBuf>,

    #[arg(long, hide = true, value_name = "PATH")]
    clipboard_daemon_temp: Option<PathBuf>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    if let Some(path) = cli.clipboard_daemon {
        serve_clipboard(&path)?;
        return Ok(());
    }
    if let Some(path) = cli.clipboard_daemon_temp {
        let result = serve_clipboard(&path);
        let _ = std::fs::remove_file(path);
        result?;
        return Ok(());
    }

    let config = shot::config::Config::load()?;
    let files = cli.files;
    let clipboard = cli.clipboard.unwrap_or(config.output.clipboard);
    let path_format = config.output.path_format;
    if files.is_empty() && path_format.is_none() && !clipboard {
        return Err(Box::new(shot::Error::NoOutputDestination));
    }

    let mut capturer = if cli.all {
        Capturer::new_for_all_monitors()?
    } else if let Some(monitor) = cli.monitor {
        if let Ok(index) = monitor.parse::<usize>() {
            Capturer::new_for_monitor(index)?
        } else {
            Capturer::new_for_monitor_name(&monitor)?
        }
    } else {
        let mut capturer = Capturer::new_for_monitor_selection()?;
        let monitor_names = capturer.monitor_names();
        let monitor = if cli.rofi {
            selector::rofi::select(&monitor_names)?
        } else if capturer.monitor_count() > 1 {
            selector::cli::select(&monitor_names)?
        } else {
            Some(0)
        };
        if let Some(monitor) = monitor {
            capturer.select_monitor(monitor)?;
        } else {
            println!();
            return Ok(());
        }
        capturer
    };
    capturer.set_output_files(files);
    capturer.set_path_format(path_format);
    capturer.set_clipboard(clipboard);
    if let Some(region) = cli.region {
        capturer.set_region(region)?;
    }
    if cli.all {
        capturer.capture_all_outputs()?;
    } else {
        capturer.capture_output()?;
    }
    Ok(())
}

fn parse_region(value: &str) -> Result<(u32, u32, u32, u32), String> {
    let values = value
        .split(',')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "expected four non-negative integers: X,Y,WIDTH,HEIGHT".to_string())?;
    let [x, y, width, height] = values.as_slice() else {
        return Err("expected four values: X,Y,WIDTH,HEIGHT".to_string());
    };
    if *width == 0 || *height == 0 {
        return Err("WIDTH and HEIGHT must be greater than zero".to_string());
    }
    Ok((*x, *y, *width, *height))
}
