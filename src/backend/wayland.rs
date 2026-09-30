use std::{error::Error, os::fd::AsFd, sync::Arc};

use memmap2::MmapMut;
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::GlobalListContents,
    globals::registry_queue_init,
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool,
        wl_surface,
    },
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::Backend;
use crate::render::{BAR_HEIGHT, BarRenderer};
use crate::widget::Widget;

const WIDTH_FALLBACK: u32 = 1280;

#[derive(Default)]
pub struct WaylandBackend;

impl Backend for WaylandBackend {
    fn run(
        &mut self,
        renderer: &mut BarRenderer,
        widgets: &mut [Box<dyn Widget>],
    ) -> Result<(), Box<dyn Error>> {
        let connection = Connection::connect_to_env()?;
        let (globals, mut event_queue) = registry_queue_init::<State>(&connection)?;
        let qh = event_queue.handle();

        let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=4, ())?;
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ())?;
        let layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ())?;

        let surface = compositor.create_surface(&qh, ());
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            None::<&wl_output::WlOutput>,
            zwlr_layer_shell_v1::Layer::Top,
            "unibar".to_string(),
            &qh,
            (),
        );
        layer_surface.set_anchor(
            zwlr_layer_surface_v1::Anchor::Top
                | zwlr_layer_surface_v1::Anchor::Left
                | zwlr_layer_surface_v1::Anchor::Right,
        );
        layer_surface.set_size(0, BAR_HEIGHT);
        layer_surface.set_exclusive_zone(BAR_HEIGHT as i32);
        layer_surface
            .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);

        // The initial commit asks the compositor to send the configure event.
        surface.commit();
        let mut state = State::new(shm, surface, layer_surface);
        event_queue.roundtrip(&mut state)?;
        if state.needs_redraw {
            state.draw(&qh, renderer, widgets)?;
        }

        while !state.closed {
            event_queue.blocking_dispatch(&mut state)?;
            if state.needs_redraw {
                state.draw(&qh, renderer, widgets)?;
            }
        }
        Ok(())
    }
}

struct State {
    shm: wl_shm::WlShm,
    surface: wl_surface::WlSurface,
    layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    buffer: Option<ShmBuffer>,
    frame_callback: Option<wl_callback::WlCallback>,
    width: u32,
    needs_redraw: bool,
    closed: bool,
}

impl State {
    fn new(
        shm: wl_shm::WlShm,
        surface: wl_surface::WlSurface,
        layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    ) -> Self {
        Self {
            shm,
            surface,
            layer_surface,
            buffer: None,
            frame_callback: None,
            width: WIDTH_FALLBACK,
            needs_redraw: false,
            closed: false,
        }
    }

    fn draw(
        &mut self,
        qh: &QueueHandle<Self>,
        renderer: &mut BarRenderer,
        widgets: &mut [Box<dyn Widget>],
    ) -> Result<(), Box<dyn Error>> {
        let width = self.width.max(1);
        let stride = width * 4;
        let size = stride * BAR_HEIGHT;
        let file = tempfile::tempfile()?;
        file.set_len(size as u64)?;
        let mut mapping = unsafe { MmapMut::map_mut(&file)? };
        let pixels = renderer.render(width, BAR_HEIGHT, widgets);
        mapping.copy_from_slice(&pixels);

        let pool = self.shm.create_pool(file.as_fd(), size as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            BAR_HEIGHT as i32,
            stride as i32,
            wl_shm::Format::Argb8888,
            qh,
            (),
        );
        pool.destroy();
        self.surface.attach(Some(&buffer), 0, 0);
        self.surface
            .damage_buffer(0, 0, width as i32, BAR_HEIGHT as i32);
        self.frame_callback = Some(self.surface.frame(qh, ()));
        self.surface.commit();
        self.buffer = Some(ShmBuffer {
            _buffer: buffer,
            _mapping: Arc::new(mapping),
        });
        self.needs_redraw = false;
        Ok(())
    }
}

struct ShmBuffer {
    _buffer: wl_buffer::WlBuffer,
    _mapping: Arc<MmapMut>,
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        _proxy: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, .. } => {
                state.layer_surface.ack_configure(serial);
                if width > 0 {
                    state.width = width;
                }
                state.needs_redraw = true;
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
        _proxy: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_callback::Event::Done { .. }) {
            state.frame_callback = None;
            state.needs_redraw = true;
        }
    }
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

delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_output::WlOutput);
delegate_noop!(State: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
