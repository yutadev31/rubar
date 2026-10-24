use std::{collections::HashMap, os::fd::AsFd};

use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, WEnum,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool},
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};

use crate::{
    backend::{Backend, BackendTarget, PositionedFrame, stitch_positioned},
    frame::Frame,
};

pub struct WaylandBackend {
    state: State,
    event_queue: EventQueue<State>,

    shm: wl_shm::WlShm,
    outputs: Vec<Output>,
    names: Vec<String>,
    selected_monitor: Option<usize>,
    manager: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
}

impl Backend for WaylandBackend {
    fn initialize(target: BackendTarget) -> crate::Result<Self> {
        let monitor_index = match target {
            BackendTarget::Monitor(index) => Some(index),
            BackendTarget::All => None,
        };
        Self::initialize_with_outputs(monitor_index)
    }

    fn capture_all_outputs(&mut self) -> crate::Result<Frame> {
        let mut frames = Vec::with_capacity(self.outputs.len());
        for output in self.outputs.clone() {
            let frame = self.capture_one(&output.proxy)?;
            let layout = self
                .state
                .output_layouts
                .get(&output.global_name)
                .copied()
                .ok_or(crate::Error::WaylandScreencopyFailed)?;
            frames.push(PositionedFrame {
                frame,
                x: layout.x,
                y: layout.y,
            });
        }
        stitch_positioned(&frames)
    }

    fn capture_output(&mut self) -> crate::Result<Frame> {
        let index = self.selected_monitor.unwrap_or(0);
        let output = self
            .outputs
            .get(index)
            .cloned()
            .ok_or(crate::Error::NoMonitors)?;
        self.capture_one(&output.proxy)
    }

    fn monitor_count(&self) -> usize {
        self.outputs.len()
    }

    fn monitor_names(&self) -> Vec<String> {
        self.names.clone()
    }

    fn select_monitor(&mut self, monitor_index: usize) -> crate::Result<()> {
        if monitor_index >= self.outputs.len() {
            return Err(crate::Error::MonitorOutOfRange {
                index: monitor_index,
                count: self.outputs.len(),
            });
        }
        self.selected_monitor = Some(monitor_index);
        Ok(())
    }
}

impl WaylandBackend {
    fn initialize_with_outputs(monitor_index: Option<usize>) -> crate::Result<Self> {
        let connection = Connection::connect_to_env()?;
        let (globals, mut event_queue) = registry_queue_init::<State>(&connection)?;

        let qh = event_queue.handle();

        let shm = globals.bind::<wl_shm::WlShm, _, _>(&qh, 1..=1, ())?;

        let output_globals = globals.contents().with_list(|globals| {
            globals
                .iter()
                .filter(|global| global.interface == "wl_output")
                .map(|global| (global.name, global.version))
                .collect::<Vec<_>>()
        });
        let output_count = output_globals.len();
        if output_count == 0 {
            return Err(crate::Error::NoMonitors);
        }
        if let Some(index) = monitor_index
            && index >= output_count
        {
            return Err(crate::Error::MonitorOutOfRange {
                index,
                count: output_count,
            });
        }
        let selected_globals = output_globals.clone();
        let outputs = selected_globals
            .clone()
            .into_iter()
            .map(|(name, version)| Output {
                global_name: name,
                proxy: globals.registry().bind(name, version.min(4), &qh, name),
            })
            .collect();

        let manager = globals.bind::<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, _, _>(
            &qh,
            1..=3,
            (),
        )?;

        let mut state = State::default();

        event_queue.blocking_dispatch(&mut state)?;

        let names = selected_globals
            .iter()
            .map(|(name, _)| {
                state
                    .output_names
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| format!("wl-output-{name}"))
            })
            .collect();

        Ok(Self {
            state,
            event_queue,
            shm,
            outputs,
            names,
            selected_monitor: monitor_index,
            manager,
        })
    }

    fn capture_one(&mut self, output: &wl_output::WlOutput) -> crate::Result<Frame> {
        let qh = self.event_queue.handle();

        self.state.reset();

        let frame = self.manager.capture_output(0, output, &qh, ());

        while self.state.buffer_info.is_none() && !self.state.failed {
            self.event_queue.blocking_dispatch(&mut self.state)?;
        }

        if let Some(format) = self.state.unsupported_format {
            return Err(crate::Error::UnsupportedWaylandPixelFormat(format));
        }
        if self.state.failed {
            return Err(crate::Error::WaylandScreencopyFailed);
        }

        let info = self
            .state
            .buffer_info
            .take()
            .ok_or(crate::Error::WaylandScreencopyFailed)?;

        let row_size = info.width as usize * 4;
        let stride = info.stride as usize;
        if stride < row_size {
            return Err(crate::Error::InvalidImageBuffer);
        }
        let size = stride
            .checked_mul(info.height as usize)
            .ok_or(crate::Error::InvalidImageBuffer)?;
        let size_i32 = i32::try_from(size).map_err(|_| crate::Error::InvalidImageBuffer)?;

        let file = tempfile::tempfile()?;
        file.set_len(size as u64)?;

        // The file was resized to `size`, so mapping the complete file is valid.
        let mmap = unsafe { memmap2::MmapMut::map_mut(&file)? };

        let pool = self.shm.create_pool(file.as_fd(), size_i32, &qh, ());

        let buffer = pool.create_buffer(
            0,
            info.width as i32,
            info.height as i32,
            info.stride as i32,
            info.format,
            &qh,
            (),
        );

        frame.copy(&buffer);

        while !self.state.ready && !self.state.failed {
            self.event_queue.blocking_dispatch(&mut self.state)?;
        }

        if self.state.failed {
            return Err(crate::Error::WaylandScreencopyFailed);
        }

        let data = convert_to_rgba(&mmap, info.width, info.height, info.stride, info.format)?;

        Ok(Frame {
            data,
            width: info.width,
            height: info.height,
        })
    }
}

#[derive(Default)]
struct State {
    output_names: HashMap<u32, String>,
    output_layouts: HashMap<u32, OutputLayout>,
    buffer_info: Option<BufferInfo>,
    ready: bool,
    failed: bool,
    unsupported_format: Option<wl_shm::Format>,
}

impl State {
    fn reset(&mut self) {
        self.buffer_info = None;
        self.ready = false;
        self.failed = false;
        self.unsupported_format = None;
    }
}

impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        _output: &wl_output::WlOutput,
        event: wl_output::Event,
        global_name: &u32,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Name { name } => {
                state.output_names.insert(*global_name, name);
            }
            wl_output::Event::Geometry { x, y, .. } => {
                state.output_layouts.entry(*global_name).or_default().x = x;
                state.output_layouts.entry(*global_name).or_default().y = y;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _frame: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } => {
                if matches!(format, wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888) {
                    state.buffer_info = Some(BufferInfo {
                        format,
                        width,
                        height,
                        stride,
                    });
                } else {
                    state.unsupported_format = Some(format);
                    state.failed = true;
                }
            }

            zwlr_screencopy_frame_v1::Event::Buffer { .. } => {
                // Do not keep dispatching forever when the compositor advertises
                // an enum value this client does not know.
                state.failed = true;
            }

            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                state.ready = true;
            }

            zwlr_screencopy_frame_v1::Event::Failed => {
                state.failed = true;
            }

            _ => {}
        }
    }
}

wayland_client::delegate_noop!(State: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(State: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(State: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(State: ignore zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1);

struct BufferInfo {
    format: wl_shm::Format,
    width: u32,
    height: u32,
    stride: u32,
}

#[derive(Clone, Copy, Default)]
struct OutputLayout {
    x: i32,
    y: i32,
}

#[derive(Clone)]
struct Output {
    global_name: u32,
    proxy: wl_output::WlOutput,
}

fn convert_to_rgba(
    data: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    format: wl_shm::Format,
) -> crate::Result<Vec<u8>> {
    let width = width as usize;
    let height = height as usize;
    let stride = stride as usize;

    let mut output = Vec::with_capacity(
        width
            .checked_mul(height)
            .and_then(|size| size.checked_mul(4))
            .ok_or(crate::Error::InvalidImageBuffer)?,
    );

    for y in 0..height {
        let start = y
            .checked_mul(stride)
            .ok_or(crate::Error::InvalidImageBuffer)?;
        let end = start
            .checked_add(
                width
                    .checked_mul(4)
                    .ok_or(crate::Error::InvalidImageBuffer)?,
            )
            .ok_or(crate::Error::InvalidImageBuffer)?;
        let row = data
            .get(start..end)
            .ok_or(crate::Error::InvalidImageBuffer)?;

        let (pixels, _) = row.as_chunks::<4>();
        for pixel in pixels {
            let b = pixel[0];
            let g = pixel[1];
            let r = pixel[2];

            match format {
                wl_shm::Format::Argb8888 => {
                    let a = pixel[3];
                    output.extend_from_slice(&[r, g, b, a]);
                }
                wl_shm::Format::Xrgb8888 => {
                    output.extend_from_slice(&[r, g, b, 255]);
                }
                _ => {
                    return Err(crate::Error::UnsupportedWaylandPixelFormat(format));
                }
            }
        }
    }

    Ok(output)
}
