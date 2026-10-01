use std::{error::Error, thread, time::Duration};

use x11rb::{
    connection::Connection,
    protocol::xproto::{
        AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt, CreateGCAux,
        CreateWindowAux, EventMask, ImageFormat, PropMode, WindowClass,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as WrapperConnectionExt,
};

use super::Backend;
use crate::{
    render::BarRenderer,
    widget::{MouseButton, ScrollDirection, WidgetGroups},
};

/// X11 dock window for traditional WMs such as i3.
#[derive(Default)]
pub struct X11Backend;

impl Backend for X11Backend {
    fn run(
        &mut self,
        renderer: &mut BarRenderer,
        widgets: &mut WidgetGroups,
    ) -> Result<(), Box<dyn Error>> {
        let (connection, screen_number) = RustConnection::connect(None)?;
        let screen = &connection.setup().roots[screen_number];
        let root = screen.root;
        let width = u32::from(screen.width_in_pixels).max(1);
        let height = renderer.height();
        let bar_width = width.max(1);
        let window = connection.generate_id()?;
        let event_mask = EventMask::EXPOSURE
            | EventMask::STRUCTURE_NOTIFY
            | EventMask::POINTER_MOTION
            | EventMask::BUTTON_PRESS;

        connection.create_window(
            0,
            window,
            root,
            0,
            0,
            bar_width as u16,
            height as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .background_pixel(screen.black_pixel)
                .override_redirect(1)
                .event_mask(event_mask),
        )?;

        let atoms = Atoms::new(&connection)?.reply()?;
        connection.change_property32(
            PropMode::REPLACE,
            window,
            atoms.net_wm_window_type,
            AtomEnum::ATOM,
            &[atoms.net_wm_window_type_dock],
        )?;
        connection.change_property32(
            PropMode::REPLACE,
            window,
            atoms.net_wm_state,
            AtomEnum::ATOM,
            &[atoms.net_wm_state_above],
        )?;
        // Reserve the complete top edge for i3.  The range is inclusive.
        connection.change_property32(
            PropMode::REPLACE,
            window,
            atoms.net_wm_strut_partial,
            AtomEnum::CARDINAL,
            &[
                0,
                0,
                height,
                0,
                0,
                0,
                0,
                0,
                0,
                width.saturating_sub(1),
                0,
                0,
            ],
        )?;
        connection.change_property32(
            PropMode::REPLACE,
            window,
            atoms.net_wm_strut,
            AtomEnum::CARDINAL,
            &[0, 0, height, 0],
        )?;
        connection.change_property8(
            PropMode::REPLACE,
            window,
            atoms.net_wm_name,
            AtomEnum::STRING,
            b"rubar",
        )?;
        connection.map_window(window)?;
        let gc = connection.generate_id()?;
        connection.create_gc(gc, window, &CreateGCAux::new())?;
        connection.flush()?;

        let mut state = State {
            connection,
            window,
            gc,
            width: bar_width,
            screen_width: width,
            height,
            renderer,
            widgets,
        };
        state.sync_layout()?;
        state.redraw()?;

        loop {
            while let Some(event) = state.connection.poll_for_event()? {
                if !state.handle_event(event)? {
                    return Ok(());
                }
            }
            // This also keeps clock and provider-backed widgets current.
            state.redraw()?;
            thread::sleep(Duration::from_millis(250));
        }
    }
}

struct State<'a, 'config> {
    connection: RustConnection,
    window: u32,
    gc: u32,
    width: u32,
    screen_width: u32,
    height: u32,
    renderer: &'a mut BarRenderer<'config>,
    widgets: &'a mut WidgetGroups,
}

impl State<'_, '_> {
    fn redraw(&mut self) -> Result<(), Box<dyn Error>> {
        let pixels = self.renderer.render(self.width, self.height, self.widgets);
        self.connection.put_image(
            ImageFormat::Z_PIXMAP,
            self.window,
            self.gc,
            self.width as u16,
            self.height as u16,
            0,
            0,
            0,
            24,
            &pixels,
        )?;
        self.connection.flush()?;
        Ok(())
    }

    fn sync_layout(&mut self) -> Result<(), Box<dyn Error>> {
        self.width = self.screen_width.max(1);
        self.connection.configure_window(
            self.window,
            &ConfigureWindowAux::new().x(0).y(0).width(self.width),
        )?;
        self.connection.flush()?;
        Ok(())
    }

    fn handle_event(&mut self, event: x11rb::protocol::Event) -> Result<bool, Box<dyn Error>> {
        use x11rb::protocol::Event;
        match event {
            Event::Expose(_) => self.redraw()?,
            Event::ConfigureNotify(event) if event.window == self.window => {
                self.width = u32::from(event.width).max(1);
                self.height = u32::from(event.height).max(1);
                self.redraw()?;
            }
            Event::ButtonPress(event) if event.event == self.window => {
                match event.detail {
                    1..=3 => {
                        self.renderer.render(self.width, self.height, self.widgets);
                        self.renderer.handle_click(
                            f64::from(event.event_x),
                            match event.detail {
                                1 => MouseButton::Left,
                                2 => MouseButton::Middle,
                                _ => MouseButton::Right,
                            },
                            self.widgets,
                        );
                    }
                    4 | 5 => {
                        self.renderer.render(self.width, self.height, self.widgets);
                        self.renderer.handle_scroll(
                            f64::from(event.event_x),
                            if event.detail == 4 {
                                ScrollDirection::Up
                            } else {
                                ScrollDirection::Down
                            },
                            self.widgets,
                        );
                    }
                    button => {
                        self.renderer.render(self.width, self.height, self.widgets);
                        self.renderer.handle_click(
                            f64::from(event.event_x),
                            MouseButton::Other(u32::from(button)),
                            self.widgets,
                        );
                    }
                }
                self.redraw()?;
            }
            Event::ClientMessage(ClientMessageEvent { .. }) => {}
            _ => {}
        }
        Ok(true)
    }
}

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        net_wm_window_type,
        net_wm_window_type_dock,
        net_wm_state,
        net_wm_state_above,
        net_wm_strut,
        net_wm_strut_partial,
        net_wm_name,
    }
}
