//! Small XEmbed system-tray host.
//!
//! XEmbed is still the interface used by a number of traditional X11 tray
//! clients.  Keeping the host here means the bar itself does not need to know
//! anything about foreign windows; it only reserves the space they occupy.

use std::error::Error;

use x11rb::{
    connection::Connection,
    protocol::xproto::{
        Atom, ClientMessageEvent, ConfigureWindowAux, ConnectionExt, CreateWindowAux, EventMask,
        WindowClass,
    },
    rust_connection::RustConnection,
};

use crate::config::TrayConfig;

const XEMBED_EMBEDDED_NOTIFY: u32 = 0;

pub struct Tray {
    pub window: u32,
    selection: Atom,
    manager: Atom,
    opcode: Atom,
    xembed: Atom,
    icons: Vec<u32>,
    height: u32,
    config: TrayConfig,
}

impl Tray {
    pub fn new(
        connection: &RustConnection,
        screen: &x11rb::protocol::xproto::Screen,
        config: &TrayConfig,
    ) -> Result<Option<Self>, Box<dyn Error>> {
        if !config.enabled {
            return Ok(None);
        }

        let selection = intern(connection, format!("_NET_SYSTEM_TRAY_S{}", 0).as_bytes())?;
        if connection.get_selection_owner(selection)?.reply()?.owner != x11rb::NONE {
            eprintln!("rubar: another system tray is already running; tray disabled");
            return Ok(None);
        }
        let manager = intern(connection, b"MANAGER")?;
        let opcode = intern(connection, b"_NET_SYSTEM_TRAY_OPCODE")?;
        let xembed = intern(connection, b"_XEMBED")?;
        let window = connection.generate_id()?;
        let height = config.icon_size.max(1);
        connection.create_window(
            0,
            window,
            screen.root,
            0,
            0,
            1,
            height as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .background_pixel(screen.black_pixel)
                .event_mask(
                    EventMask::STRUCTURE_NOTIFY
                        | EventMask::SUBSTRUCTURE_NOTIFY
                        | EventMask::PROPERTY_CHANGE,
                ),
        )?;
        connection.set_selection_owner(window, selection, x11rb::CURRENT_TIME)?;
        if connection.get_selection_owner(selection)?.reply()?.owner != window {
            return Ok(None);
        }
        connection.map_window(window)?;
        connection.send_event(
            false,
            screen.root,
            EventMask::STRUCTURE_NOTIFY,
            ClientMessageEvent::new(
                32,
                screen.root,
                manager,
                [x11rb::CURRENT_TIME, selection, window, 0, 0],
            ),
        )?;
        connection.flush()?;

        Ok(Some(Self {
            window,
            selection,
            manager,
            opcode,
            xembed,
            icons: Vec::new(),
            height,
            config: TrayConfig {
                enabled: config.enabled,
                icon_size: config.icon_size,
                spacing: config.spacing,
            },
        }))
    }

    pub fn width(&self) -> u32 {
        if self.icons.is_empty() {
            0
        } else {
            self.icons.len() as u32 * self.height
                + (self.icons.len().saturating_sub(1) as u32) * self.config.spacing
        }
    }

    pub fn handle_event(
        &mut self,
        connection: &RustConnection,
        event: &x11rb::protocol::Event,
    ) -> Result<bool, Box<dyn Error>> {
        match event {
            x11rb::protocol::Event::ClientMessage(message)
                if message.window == self.window && message.type_ == self.manager =>
            {
                let data = message.data.as_data32();
                if data[1] == self.selection {
                    // Another owner took the selection; stop accepting icons.
                    return Ok(false);
                }
            }
            x11rb::protocol::Event::ClientMessage(message)
                if message.window == self.window && message.type_ == self.opcode =>
            {
                let data = message.data.as_data32();
                let icon = data[2];
                if data[1] == 0 && icon != 0 && !self.icons.contains(&icon) {
                    self.dock(connection, icon)?;
                    return Ok(true);
                }
            }
            x11rb::protocol::Event::DestroyNotify(notify) => {
                if let Some(index) = self.icons.iter().position(|icon| *icon == notify.window) {
                    self.icons.remove(index);
                    self.layout(connection)?;
                }
            }
            x11rb::protocol::Event::ConfigureNotify(notify) => {
                if self.icons.contains(&notify.window) {
                    self.layout(connection)?;
                }
            }
            _ => {}
        }
        Ok(true)
    }

    pub fn place(&self, connection: &RustConnection, x: u32, y: u32) -> Result<(), Box<dyn Error>> {
        connection.configure_window(
            self.window,
            &ConfigureWindowAux::new()
                .x(x as i32)
                .y(y as i32)
                .width(self.width()),
        )?;
        connection.flush()?;
        Ok(())
    }

    fn dock(&mut self, connection: &RustConnection, icon: u32) -> Result<(), Box<dyn Error>> {
        connection.reparent_window(icon, self.window, 0, 0)?;
        connection.change_window_attributes(
            icon,
            &x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
                .event_mask(EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE),
        )?;
        connection.configure_window(
            icon,
            &ConfigureWindowAux::new()
                .width(self.height)
                .height(self.height),
        )?;
        connection.map_window(icon)?;
        connection.send_event(
            false,
            icon,
            EventMask::NO_EVENT,
            ClientMessageEvent::new(
                32,
                icon,
                self.xembed,
                [
                    x11rb::CURRENT_TIME,
                    XEMBED_EMBEDDED_NOTIFY,
                    self.window,
                    0,
                    0,
                ],
            ),
        )?;
        self.icons.push(icon);
        self.layout(connection)
    }

    fn layout(&self, connection: &RustConnection) -> Result<(), Box<dyn Error>> {
        for (index, icon) in self.icons.iter().enumerate() {
            let x = index as u32 * (self.height + self.config.spacing);
            connection.configure_window(
                *icon,
                &ConfigureWindowAux::new()
                    .x(x as i32)
                    .y(0)
                    .width(self.height)
                    .height(self.height),
            )?;
        }
        connection.configure_window(
            self.window,
            &ConfigureWindowAux::new()
                .width(self.width())
                .height(self.height),
        )?;
        connection.flush()?;
        Ok(())
    }
}

fn intern(connection: &RustConnection, name: &[u8]) -> Result<Atom, Box<dyn Error>> {
    Ok(connection.intern_atom(false, name)?.reply()?.atom)
}
