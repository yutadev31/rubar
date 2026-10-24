use x11rb::{
    connection::Connection,
    image::Image,
    protocol::{
        randr::ConnectionExt as _,
        xproto::{ConnectionExt as _, ImageFormat},
    },
    rust_connection::RustConnection,
};

use crate::{
    backend::{Backend, BackendTarget, PositionedFrame, stitch_positioned},
    frame::Frame,
};

pub struct X11Backend {
    connection: RustConnection,
    root: u32,
    monitors: Vec<Monitor>,
    selected_monitor: Option<usize>,
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
}

#[derive(Clone)]
struct Monitor {
    name: String,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
}

impl Backend for X11Backend {
    fn initialize(target: BackendTarget) -> crate::Result<Self> {
        let monitor_index = match target {
            BackendTarget::Monitor(index) => Some(index),
            BackendTarget::All => None,
        };
        Self::initialize_with_monitors(monitor_index)
    }

    fn capture_all_outputs(&mut self) -> crate::Result<Frame> {
        let monitors = self.monitors.clone();
        let mut frames = Vec::with_capacity(monitors.len());
        for monitor in monitors {
            let x = i32::from(monitor.x);
            let y = i32::from(monitor.y);
            frames.push(PositionedFrame {
                frame: self.capture_monitor(monitor)?,
                x,
                y,
            });
        }
        stitch_positioned(&frames)
    }

    fn capture_output(&mut self) -> crate::Result<Frame> {
        let index = self.selected_monitor.unwrap_or(0);
        let monitor = self
            .monitors
            .get(index)
            .cloned()
            .ok_or(crate::Error::NoMonitors)?;
        self.capture_monitor(monitor)
    }

    fn monitor_count(&self) -> usize {
        self.monitors.len()
    }

    fn monitor_names(&self) -> Vec<String> {
        self.monitors
            .iter()
            .map(|monitor| monitor.name.clone())
            .collect()
    }

    fn select_monitor(&mut self, monitor_index: usize) -> crate::Result<()> {
        if monitor_index >= self.monitors.len() {
            return Err(crate::Error::MonitorOutOfRange {
                index: monitor_index,
                count: self.monitors.len(),
            });
        }
        self.selected_monitor = Some(monitor_index);
        Ok(())
    }
}

impl X11Backend {
    fn initialize_with_monitors(monitor_index: Option<usize>) -> crate::Result<Self> {
        let (connection, screen_num) =
            x11rb::connect(None).map_err(|error| crate::Error::X11Connect(error.to_string()))?;
        let setup = connection.setup();
        let screen = setup
            .roots
            .get(screen_num)
            .ok_or_else(|| crate::Error::X11Screen("screen index is unavailable".to_string()))?;
        let visual = screen
            .allowed_depths
            .iter()
            .flat_map(|depth| depth.visuals.iter())
            .find(|visual| visual.visual_id == screen.root_visual)
            .ok_or_else(|| crate::Error::X11Visual("root visual is unavailable".to_string()))?;

        let monitors = connection
            .randr_get_monitors(screen.root, true)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| {
                reply
                    .monitors
                    .into_iter()
                    .map(|monitor| {
                        let name = connection
                            .get_atom_name(monitor.name)
                            .ok()
                            .and_then(|cookie| cookie.reply().ok())
                            .map(|reply| String::from_utf8_lossy(&reply.name).into_owned())
                            .unwrap_or_else(|| format!("x11-monitor-{}", monitor.name));
                        Monitor {
                            name,
                            x: monitor.x,
                            y: monitor.y,
                            width: monitor.width,
                            height: monitor.height,
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                vec![Monitor {
                    name: "x11-monitor-0".to_string(),
                    x: 0,
                    y: 0,
                    width: screen.width_in_pixels,
                    height: screen.height_in_pixels,
                }]
            });
        if let Some(index) = monitor_index
            && index >= monitors.len()
        {
            return Err(crate::Error::MonitorOutOfRange {
                index,
                count: monitors.len(),
            });
        }
        let root = screen.root;
        let red_mask = visual.red_mask;
        let green_mask = visual.green_mask;
        let blue_mask = visual.blue_mask;

        Ok(Self {
            connection,
            root,
            monitors,
            selected_monitor: monitor_index,
            red_mask,
            green_mask,
            blue_mask,
        })
    }

    fn capture_monitor(&mut self, monitor: Monitor) -> crate::Result<Frame> {
        let reply = self
            .connection
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.root,
                monitor.x,
                monitor.y,
                monitor.width,
                monitor.height,
                u32::MAX,
            )
            .map_err(|error| crate::Error::X11Image(error.to_string()))?
            .reply()
            .map_err(|error| crate::Error::X11Image(error.to_string()))?;
        let image = Image::get_from_reply(
            self.connection.setup(),
            monitor.width,
            monitor.height,
            reply,
        )
        .map_err(|error| crate::Error::X11Image(error.to_string()))?;
        let capacity = monitor.width as usize * monitor.height as usize * 4;
        let mut data = Vec::with_capacity(capacity);
        for y in 0..monitor.height {
            for x in 0..monitor.width {
                let pixel = image.get_pixel(x, y);
                data.extend_from_slice(&[
                    channel(pixel, self.red_mask),
                    channel(pixel, self.green_mask),
                    channel(pixel, self.blue_mask),
                    255,
                ]);
            }
        }
        Ok(Frame {
            data,
            width: monitor.width as u32,
            height: monitor.height as u32,
        })
    }
}

fn channel(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let value = (pixel & mask) >> shift;
    let max = mask >> shift;
    ((value * 255 + max / 2) / max) as u8
}
