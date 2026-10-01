use std::{
    error::Error,
    os::fd::AsFd,
    sync::Arc,
    time::{Duration, Instant},
};

use memmap2::MmapMut;
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::GlobalListContents,
    globals::registry_queue_init,
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm,
        wl_shm_pool, wl_surface,
    },
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::Backend;
use crate::widget::{MouseButton, ScrollDirection, WidgetGroups};
use crate::{config::TrayConfig, render::BarRenderer};

const WIDTH_FALLBACK: u32 = 1280;
const REDRAW_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Default)]
pub struct WaylandBackend;

impl Backend for WaylandBackend {
    fn run(
        &mut self,
        renderer: &mut BarRenderer,
        widgets: &mut WidgetGroups,
        tray: &TrayConfig,
    ) -> Result<(), Box<dyn Error>> {
        // Legacy X11 tray clients use XEmbed rather than D-Bus.  When this
        // Wayland session also has Xwayland, host those clients in parallel
        // with the native StatusNotifier widget.
        let _xembed_tray = crate::backend::x11_tray::TrayThread::spawn(tray);
        let connection = Connection::connect_to_env()?;
        let (globals, mut event_queue) = registry_queue_init::<State>(&connection)?;
        let qh = event_queue.handle();

        let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=4, ())?;
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ())?;
        let layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ())?;
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=9, ())?;

        // wl_output is a multi-instance global, so bind every advertised output
        // and create one layer surface for each of them.  Passing None here would
        // let the compositor choose a single output, which is not suitable for a
        // panel that should appear on every monitor.
        let registry = globals.registry().clone();
        let outputs = globals.contents().with_list(|list| {
            list.iter()
                .filter(|global| global.interface == "wl_output")
                .map(|global| {
                    registry.bind::<wl_output::WlOutput, _, _>(
                        global.name,
                        global.version.min(4),
                        &qh,
                        (),
                    )
                })
                .collect::<Vec<_>>()
        });
        if outputs.is_empty() {
            return Err("Wayland compositor advertised no outputs".into());
        }

        let bars = outputs
            .iter()
            .map(|output| {
                let surface = compositor.create_surface(&qh, ());
                let layer_surface = layer_shell.get_layer_surface(
                    &surface,
                    Some(output),
                    zwlr_layer_shell_v1::Layer::Top,
                    "rubar".to_string(),
                    &qh,
                    (),
                );
                layer_surface.set_anchor(
                    zwlr_layer_surface_v1::Anchor::Top
                        | zwlr_layer_surface_v1::Anchor::Left
                        | zwlr_layer_surface_v1::Anchor::Right,
                );
                layer_surface.set_size(0, renderer.height());
                layer_surface.set_exclusive_zone(renderer.height() as i32);
                layer_surface
                    .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);

                // The initial commit asks the compositor to send the configure event.
                surface.commit();
                BarState::new(output.clone(), surface, layer_surface)
            })
            .collect();
        let pointer = seat.get_pointer(&qh, ());
        let mut state = State::new(shm, outputs, bars, seat, pointer);
        event_queue.roundtrip(&mut state)?;
        state.draw_pending(&qh, renderer, widgets)?;

        while !state.closed {
            event_queue.blocking_dispatch(&mut state)?;
            if widgets.take_redraw_request() {
                state.needs_redraw = true;
                for bar in &mut state.bars {
                    bar.needs_redraw = true;
                }
            }
            let pointer_events = std::mem::take(&mut state.pending_pointer_events);
            if !pointer_events.is_empty() {
                state.needs_redraw = true;
                for bar in &mut state.bars {
                    bar.needs_redraw = true;
                }
            }
            for event in pointer_events {
                match event {
                    PointerEvent::Click { button } => {
                        state.handle_click(renderer, widgets, button);
                    }
                    PointerEvent::Scroll { direction } => {
                        if !state.handle_scroll(renderer, widgets, direction) {
                            eprintln!(
                                "rubar: scroll at x={} did not hit a widget",
                                state.pointer_x
                            );
                        }
                    }
                }
            }
            // A surface must not be redrawn while its previous frame callback
            // is outstanding.  In particular, pointer events can arrive
            // faster than the compositor presents frames; keep the newest
            // widget state and draw it when the callback is done.
            state.draw_pending(&qh, renderer, widgets)?;
        }
        Ok(())
    }
}

struct State {
    shm: wl_shm::WlShm,
    _outputs: Vec<wl_output::WlOutput>,
    bars: Vec<BarState>,
    _seat: wl_seat::WlSeat,
    _pointer: wl_pointer::WlPointer,
    pointer_x: f64,
    pending_pointer_events: Vec<PointerEvent>,
    needs_redraw: bool,
    closed: bool,
    pointer_bar: Option<usize>,
}

struct BarState {
    output: wl_output::WlOutput,
    surface: wl_surface::WlSurface,
    layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    buffer: Option<ShmBuffer>,
    frame_callback: Option<wl_callback::WlCallback>,
    width: u32,
    monitor_name: Option<String>,
    needs_redraw: bool,
    last_draw: Instant,
    last_pixels: Option<Vec<u8>>,
}

impl BarState {
    fn new(
        output: wl_output::WlOutput,
        surface: wl_surface::WlSurface,
        layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    ) -> Self {
        Self {
            output,
            surface,
            layer_surface,
            buffer: None,
            frame_callback: None,
            width: WIDTH_FALLBACK,
            monitor_name: None,
            needs_redraw: false,
            last_draw: Instant::now(),
            last_pixels: None,
        }
    }
}

impl State {
    fn new(
        shm: wl_shm::WlShm,
        outputs: Vec<wl_output::WlOutput>,
        bars: Vec<BarState>,
        seat: wl_seat::WlSeat,
        pointer: wl_pointer::WlPointer,
    ) -> Self {
        Self {
            shm,
            _outputs: outputs,
            bars,
            _seat: seat,
            _pointer: pointer,
            pointer_x: 0.0,
            pending_pointer_events: Vec::new(),
            needs_redraw: false,
            closed: false,
            pointer_bar: None,
        }
    }

    fn draw_pending(
        &mut self,
        qh: &QueueHandle<Self>,
        renderer: &mut BarRenderer,
        widgets: &mut WidgetGroups,
    ) -> Result<(), Box<dyn Error>> {
        for index in 0..self.bars.len() {
            if self.bars[index].needs_redraw && self.bars[index].frame_callback.is_none() {
                self.draw(index, qh, renderer, widgets)?;
            }
        }
        self.needs_redraw = self.bars.iter().any(|bar| bar.needs_redraw);
        Ok(())
    }

    fn draw(
        &mut self,
        index: usize,
        qh: &QueueHandle<Self>,
        renderer: &mut BarRenderer,
        widgets: &mut WidgetGroups,
    ) -> Result<(), Box<dyn Error>> {
        let width = self.bars[index].width.max(1);
        let monitor_name = self.bars[index].monitor_name.clone();
        widgets.set_monitor_name(monitor_name.as_deref());
        let height = renderer.height();
        let stride = width * 4;
        let pixels = renderer.render(width, height, widgets);

        if self.bars[index].last_pixels.as_ref() == Some(&pixels) {
            let bar = &mut self.bars[index];
            bar.frame_callback = Some(bar.surface.frame(qh, ()));
            bar.surface.damage_buffer(0, 0, 1, 1);
            bar.surface.commit();
            bar.needs_redraw = false;
            bar.last_draw = Instant::now();
            return Ok(());
        }

        let size = stride * height;
        let file = tempfile::tempfile()?;
        file.set_len(size as u64)?;
        let mut mapping = unsafe { MmapMut::map_mut(&file)? };
        mapping.copy_from_slice(&pixels);

        let pool = self.shm.create_pool(file.as_fd(), size as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            wl_shm::Format::Argb8888,
            qh,
            (),
        );
        pool.destroy();
        let bar = &mut self.bars[index];
        bar.surface.attach(Some(&buffer), 0, 0);
        bar.surface.damage_buffer(0, 0, width as i32, height as i32);
        bar.frame_callback = Some(bar.surface.frame(qh, ()));
        bar.surface.commit();
        bar.buffer = Some(ShmBuffer {
            _buffer: buffer,
            _mapping: Arc::new(mapping),
        });
        bar.needs_redraw = false;
        bar.last_draw = Instant::now();
        bar.last_pixels = Some(pixels);
        Ok(())
    }

    fn handle_click(
        &mut self,
        renderer: &mut BarRenderer,
        widgets: &mut WidgetGroups,
        button: MouseButton,
    ) {
        if let Some(index) = self.pointer_bar {
            let width = self.bars[index].width.max(1);
            let monitor_name = self.bars[index].monitor_name.clone();
            widgets.set_monitor_name(monitor_name.as_deref());
            let _ = renderer.render(width, renderer.height(), widgets);
            renderer.handle_click(self.pointer_x, button, widgets);
        }
    }

    fn handle_scroll(
        &mut self,
        renderer: &mut BarRenderer,
        widgets: &mut WidgetGroups,
        direction: ScrollDirection,
    ) -> bool {
        let Some(index) = self.pointer_bar else {
            return false;
        };
        let width = self.bars[index].width.max(1);
        let monitor_name = self.bars[index].monitor_name.clone();
        widgets.set_monitor_name(monitor_name.as_deref());
        let _ = renderer.render(width, renderer.height(), widgets);
        renderer.handle_scroll(self.pointer_x, direction, widgets)
    }
}

struct ShmBuffer {
    _buffer: wl_buffer::WlBuffer,
    _mapping: Arc<MmapMut>,
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, .. } => {
                if let Some(index) = state
                    .bars
                    .iter()
                    .position(|bar| &bar.layer_surface == proxy)
                {
                    let bar = &mut state.bars[index];
                    bar.layer_surface.ack_configure(serial);
                    if width > 0 {
                        bar.width = width;
                    }
                    bar.needs_redraw = true;
                }
            }
            zwlr_layer_surface_v1::Event::Closed => state.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &wl_buffer::WlBuffer,
        _event: wl_buffer::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_callback::Event::Done { .. }) {
            if let Some(bar) = state
                .bars
                .iter_mut()
                .find(|bar| bar.frame_callback.as_ref() == Some(proxy))
            {
                bar.frame_callback = None;
                if bar.last_draw.elapsed() < REDRAW_INTERVAL && !bar.needs_redraw {
                    // Keep the frame callback as a low-frequency timer without
                    // allocating another shm buffer for every compositor frame.
                    bar.frame_callback = Some(bar.surface.frame(_qh, ()));
                    bar.surface.damage_buffer(0, 0, 1, 1);
                    bar.surface.commit();
                } else {
                    bar.needs_redraw = true;
                }
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _proxy: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface, surface_x, ..
            } => {
                state.pointer_bar = state.bars.iter().position(|bar| bar.surface == surface);
                state.pointer_x = surface_x;
            }
            wl_pointer::Event::Motion { surface_x, .. } => {
                state.pointer_x = surface_x;
            }
            wl_pointer::Event::Button {
                button,
                state: button_state,
                ..
            } => {
                if button_state.into_result().ok() == Some(wl_pointer::ButtonState::Pressed) {
                    state.pending_pointer_events.push(PointerEvent::Click {
                        button: match button {
                            0x110 => MouseButton::Left,
                            0x111 => MouseButton::Right,
                            0x112 => MouseButton::Middle,
                            other => MouseButton::Other(other),
                        },
                    });
                }
            }
            wl_pointer::Event::Axis { axis, value, .. }
                if axis == wayland_client::WEnum::Value(wl_pointer::Axis::VerticalScroll) =>
            {
                state.pending_pointer_events.push(PointerEvent::Scroll {
                    direction: if value < 0.0 {
                        ScrollDirection::Up
                    } else {
                        ScrollDirection::Down
                    },
                });
            }
            _ => {}
        }
    }
}

enum PointerEvent {
    Click { button: MouseButton },
    Scroll { direction: ScrollDirection },
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &wayland_client::Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(bar) = state.bars.iter_mut().find(|bar| &bar.output == proxy) {
                bar.monitor_name = Some(name);
                bar.needs_redraw = true;
            }
        }
    }
}

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
