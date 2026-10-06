use std::{collections::HashMap, error::Error};

use cosmic_text::{
    Attrs, Buffer, Color as TextColor, FontSystem, Metrics, Shaping, SwashCache, Wrap,
};
use shell_surface::{
    Anchors, Backend, InputEvent, KeyboardInteractivity, Layer, OutputSelection, Position, Shell,
    Size, SurfaceConfig, SurfaceId,
};
use tiny_skia::{Color, Pixmap};
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedObjectPath, OwnedValue, Structure, Value},
};

use crate::{config::StyleConfig, render::parse_rgba};

use super::{ITEM_INTERFACE, TrayItem};

const MENU_INTERFACE: &str = "com.canonical.dbusmenu";
const ROW_HEIGHT: u32 = 30;
const MENU_WIDTH: u32 = 320;
type MenuLayout = (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>));

struct MenuEntry {
    id: i32,
    label: String,
    enabled: bool,
    submenu: bool,
    separator: bool,
    toggle: Option<String>,
}

pub(super) fn open(
    item: TrayItem,
    button_left: i32,
    surface_width: u32,
    output: Option<String>,
    style: &StyleConfig,
) -> Result<(), Box<dyn Error>> {
    let connection = Connection::session()?;
    let item_proxy = Proxy::new(
        &connection,
        item.service.as_str(),
        item.path.as_str(),
        ITEM_INTERFACE,
    )?;
    let menu_path = item_proxy
        .get_property::<OwnedObjectPath>("Menu")
        .ok()
        .map(|path| path.to_string());
    let Some(menu_path) = menu_path.filter(|path| path != "/" && path != "/NO_DBUSMENU") else {
        item_proxy.call_method("ContextMenu", &(button_left, style.height as i32))?;
        return Ok(());
    };
    let menu_proxy = Proxy::new(
        &connection,
        item.service.as_str(),
        menu_path.as_str(),
        MENU_INTERFACE,
    )?;
    send_event(&menu_proxy, 0, "opened")?;
    let result = show_menu(
        &menu_proxy,
        button_left,
        surface_width,
        output.as_deref(),
        style,
    );
    let closed = send_event(&menu_proxy, 0, "closed");
    result?;
    closed
}

fn show_menu(
    proxy: &Proxy<'_>,
    button_left: i32,
    surface_width: u32,
    output: Option<&str>,
    style: &StyleConfig,
) -> Result<(), Box<dyn Error>> {
    let mut path = vec![0];
    loop {
        let parent = *path.last().expect("root menu exists");
        let mut rows = load_rows(proxy, parent)?;
        if path.len() > 1 {
            rows.insert(
                0,
                MenuEntry {
                    id: -1,
                    label: "‹ Back".to_string(),
                    enabled: true,
                    submenu: false,
                    separator: false,
                    toggle: None,
                },
            );
        }
        if rows.is_empty() {
            return Ok(());
        }
        let mut popup = MenuPopup::new(rows, button_left, surface_width, output, style)?;
        let mut backend = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            Box::new(shell_surface::backend::wayland::WaylandBackend) as Box<dyn Backend>
        } else {
            Box::new(shell_surface::backend::x11::X11Backend) as Box<dyn Backend>
        };
        backend.run(&mut popup)?;
        let Some(index) = popup.selection else {
            return Ok(());
        };
        let selected = &popup.rows[index];
        if selected.id == -1 {
            path.pop();
        } else if selected.submenu {
            path.push(selected.id);
        } else {
            send_event(proxy, selected.id, "clicked")?;
            return Ok(());
        }
    }
}

fn send_event(proxy: &Proxy<'_>, id: i32, kind: &str) -> Result<(), Box<dyn Error>> {
    proxy.call_method("Event", &(id, kind, Value::new(0_i32), 0_u32))?;
    Ok(())
}

fn load_rows(proxy: &Proxy<'_>, parent: i32) -> Result<Vec<MenuEntry>, Box<dyn Error>> {
    let _: bool = proxy.call("AboutToShow", &(parent,))?;
    let (_revision, (_id, _properties, children)): MenuLayout =
        proxy.call("GetLayout", &(parent, 1_i32, Vec::<String>::new()))?;
    let mut rows = Vec::new();
    for child in children {
        let structure = Structure::try_from(child)?;
        let mut fields = structure.into_fields();
        if fields.len() != 3 {
            return Err("invalid DBusMenu item layout".into());
        }
        let id = i32::try_from(fields.remove(0))?;
        let properties = HashMap::<String, OwnedValue>::try_from(fields.remove(0))?;
        if !property_bool(&properties, "visible", true) {
            continue;
        }
        let separator = property_string(&properties, "type").as_deref() == Some("separator");
        let label = property_string(&properties, "label")
            .unwrap_or_default()
            .replace('_', "");
        let toggle = match property_string(&properties, "toggle-type").as_deref() {
            Some("radio") => Some(if property_i32(&properties, "toggle-state") == Some(1) {
                "◉"
            } else {
                "○"
            }),
            Some("checkmark") => Some(if property_i32(&properties, "toggle-state") == Some(1) {
                "☑"
            } else {
                "□"
            }),
            _ => None,
        }
        .map(str::to_string);
        rows.push(MenuEntry {
            id,
            label,
            enabled: property_bool(&properties, "enabled", true),
            submenu: property_string(&properties, "children-display").as_deref() == Some("submenu"),
            separator,
            toggle,
        });
    }
    Ok(rows)
}

fn property_string(properties: &HashMap<String, OwnedValue>, name: &str) -> Option<String> {
    properties
        .get(name)
        .and_then(|value| String::try_from(value.try_clone().ok()?).ok())
}

fn property_bool(properties: &HashMap<String, OwnedValue>, name: &str, default: bool) -> bool {
    properties
        .get(name)
        .map_or(default, |value| bool::try_from(value).unwrap_or(default))
}

fn property_i32(properties: &HashMap<String, OwnedValue>, name: &str) -> Option<i32> {
    properties
        .get(name)
        .and_then(|value| i32::try_from(value).ok())
}

struct MenuPopup {
    surfaces: Vec<SurfaceConfig>,
    rows: Vec<MenuEntry>,
    selection: Option<usize>,
    closed: bool,
    hovered: Option<usize>,
    redraw: bool,
    background: [u8; 4],
    foreground: [u8; 4],
    font_size: f32,
    font_family: String,
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl MenuPopup {
    fn new(
        rows: Vec<MenuEntry>,
        button_left: i32,
        surface_width: u32,
        output: Option<&str>,
        style: &StyleConfig,
    ) -> Result<Self, Box<dyn Error>> {
        let height = ROW_HEIGHT * rows.len() as u32;
        let mut surface = SurfaceConfig::new("rubar-tray-menu", Size::new(MENU_WIDTH, height));
        let max_left = surface_width.saturating_sub(MENU_WIDTH) as i32;
        let left = button_left.clamp(0, max_left);
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            surface.anchors = Anchors::TOP | Anchors::RIGHT;
            surface.position = Position::new(max_left - left, style.height as i32);
        } else {
            surface.anchors = Anchors::TOP | Anchors::LEFT;
            surface.position = Position::new(left, style.height as i32);
        }
        surface.layer = Layer::Overlay;
        // The bar's exclusive zone already consumes its height. Ignore that
        // reservation so the top margin places this surface directly below it.
        surface.exclusive_zone = -1;
        if let Some(output) = output {
            surface.output = OutputSelection::Named(output.to_string());
        }
        surface.keyboard_interactivity = KeyboardInteractivity::OnDemand;
        surface.override_redirect = true;
        Ok(Self {
            surfaces: vec![surface],
            rows,
            selection: None,
            closed: false,
            hovered: None,
            redraw: false,
            background: parse_rgba(&style.colors.bg)?,
            foreground: parse_rgba(&style.colors.text)?,
            font_size: style.font_size.max(1.0),
            font_family: style.font_family.clone(),
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        })
    }

    fn row_at(&self, y: f64) -> Option<usize> {
        let index = (y / ROW_HEIGHT as f64) as usize;
        (y >= 0.0 && index < self.rows.len()).then_some(index)
    }

    fn draw_label(&mut self, pixmap: &mut Pixmap, text: &str, row: usize, color: [u8; 4]) {
        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(self.font_size, self.font_size * 1.3),
        );
        buffer.set_size(Some((MENU_WIDTH - 24) as f32), Some(ROW_HEIGHT as f32));
        buffer.set_wrap(Wrap::None);
        let text_color = TextColor::rgba(color[0], color[1], color[2], color[3]);
        let mut attrs = Attrs::new().color(text_color);
        if !self.font_family.is_empty() {
            attrs = attrs.family(cosmic_text::Family::Name(&self.font_family));
        }
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        let y = row as i32 * ROW_HEIGHT as i32
            + ((ROW_HEIGHT as f32 - self.font_size * 1.3) / 2.0) as i32;
        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            text_color,
            |glyph_x, glyph_y, _w, _h, color| {
                let px = 12 + glyph_x;
                let py = y + glyph_y;
                if px < 0 || py < 0 || px >= pixmap.width() as i32 || py >= pixmap.height() as i32 {
                    return;
                }
                let offset = (py as u32 * pixmap.width() + px as u32) as usize * 4;
                let [r, g, b, coverage] = color.as_rgba();
                let data = pixmap.data_mut();
                let inverse = 255 - coverage as u16;
                let blend = |src: u8, dst: u8| {
                    ((src as u16 * coverage as u16 + dst as u16 * inverse) / 255) as u8
                };
                let destination = [data[offset], data[offset + 1], data[offset + 2]];
                data[offset..offset + 4].copy_from_slice(&[
                    blend(r, destination[0]),
                    blend(g, destination[1]),
                    blend(b, destination[2]),
                    255,
                ]);
            },
        );
    }
}

impl Shell for MenuPopup {
    fn surface_configs(&self) -> &[SurfaceConfig] {
        &self.surfaces
    }

    fn render(
        &mut self,
        _surface: SurfaceId,
        size: Size,
        _output: Option<&str>,
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut pixmap = Pixmap::new(size.width, size.height).ok_or("invalid menu size")?;
        pixmap.fill(Color::from_rgba8(
            self.background[0],
            self.background[1],
            self.background[2],
            self.background[3],
        ));
        for index in 0..self.rows.len() {
            if self.hovered == Some(index)
                && self.rows[index].enabled
                && !self.rows[index].separator
            {
                let paint = tiny_skia::Paint {
                    shader: tiny_skia::Shader::SolidColor(Color::from_rgba8(255, 255, 255, 28)),
                    ..Default::default()
                };
                if let Some(rect) = tiny_skia::Rect::from_xywh(
                    0.0,
                    index as f32 * ROW_HEIGHT as f32,
                    size.width as f32,
                    ROW_HEIGHT as f32,
                ) {
                    pixmap.fill_rect(rect, &paint, tiny_skia::Transform::identity(), None);
                }
            }
            if self.rows[index].separator {
                let paint = tiny_skia::Paint {
                    shader: tiny_skia::Shader::SolidColor(Color::from_rgba8(
                        self.foreground[0],
                        self.foreground[1],
                        self.foreground[2],
                        72,
                    )),
                    ..Default::default()
                };
                if let Some(rect) = tiny_skia::Rect::from_xywh(
                    8.0,
                    index as f32 * ROW_HEIGHT as f32 + ROW_HEIGHT as f32 / 2.0,
                    size.width.saturating_sub(16) as f32,
                    1.0,
                ) {
                    pixmap.fill_rect(rect, &paint, tiny_skia::Transform::identity(), None);
                }
                continue;
            }
            let mut label = self.rows[index].label.clone();
            if let Some(toggle) = &self.rows[index].toggle {
                label = format!("{toggle}  {label}");
            }
            if self.rows[index].submenu {
                label.push_str("  ›");
            }
            let mut color = self.foreground;
            if !self.rows[index].enabled {
                color[3] /= 2;
            }
            self.draw_label(&mut pixmap, &label, index, color);
        }
        let (pixels, _) = pixmap.data().as_chunks::<4>();
        Ok(pixels
            .iter()
            .flat_map(|rgba| [rgba[2], rgba[1], rgba[0], rgba[3]])
            .collect())
    }

    fn handle_event(&mut self, _surface: SurfaceId, event: InputEvent) {
        match event {
            InputEvent::PointerMotion { position, .. }
            | InputEvent::PointerEnter { position, .. } => {
                self.hovered = self.row_at(position.y);
                self.redraw = true;
            }
            InputEvent::PointerButton {
                position,
                button: shell_surface::MouseButton::Left,
                pressed: true,
                ..
            } => {
                if let Some(index) = self.row_at(position.y)
                    && self.rows[index].enabled
                    && !self.rows[index].separator
                {
                    self.selection = Some(index);
                }
                self.closed = self.selection.is_some();
            }
            InputEvent::FocusLost | InputEvent::CloseRequested => self.closed = true,
            InputEvent::Key {
                keycode: 1 | 9,
                pressed: true,
            } => self.closed = true,
            _ => {}
        }
    }

    fn take_redraw_request(&mut self) -> bool {
        std::mem::take(&mut self.redraw)
    }
    fn should_close(&self) -> bool {
        self.closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a live StatusNotifierItem; set RUBAR_TEST_SNI_SERVICE"]
    fn loads_live_dbus_menu() {
        let service = std::env::var("RUBAR_TEST_SNI_SERVICE").expect("set RUBAR_TEST_SNI_SERVICE");
        let connection = Connection::session().unwrap();
        let item = Proxy::new(
            &connection,
            service.as_str(),
            "/StatusNotifierItem",
            ITEM_INTERFACE,
        )
        .unwrap();
        let path: OwnedObjectPath = item.get_property("Menu").unwrap();
        let proxy =
            Proxy::new(&connection, service.as_str(), path.as_str(), MENU_INTERFACE).unwrap();
        send_event(&proxy, 0, "opened").unwrap();
        let rows = load_rows(&proxy, 0).unwrap();
        send_event(&proxy, 0, "closed").unwrap();
        assert!(!rows.is_empty());
    }

    #[test]
    fn handles_selection() {
        let rows = vec![MenuEntry {
            id: 1,
            label: "Open".to_string(),
            enabled: true,
            submenu: false,
            separator: false,
            toggle: None,
        }];
        let mut popup =
            MenuPopup::new(rows, 1200, 1920, Some("eDP-1"), &StyleConfig::default()).unwrap();
        let expected_x = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            400
        } else {
            1200
        };
        assert_eq!(popup.surfaces[0].position.x, expected_x);
        assert_eq!(popup.surfaces[0].position.y, 30);
        assert_eq!(popup.surfaces[0].exclusive_zone, -1);
        assert_eq!(
            popup.surfaces[0].output,
            OutputSelection::Named("eDP-1".to_string())
        );
        popup.handle_event(
            SurfaceId(0),
            InputEvent::PointerButton {
                position: shell_surface::PositionF64::new(10.0, 10.0),
                button: shell_surface::MouseButton::Left,
                pressed: true,
                output: None,
            },
        );
        assert_eq!(popup.selection, Some(0));
        assert!(popup.should_close());
    }

    #[test]
    fn renders_separator_without_system_fonts() {
        let rows = vec![MenuEntry {
            id: 1,
            label: String::new(),
            enabled: true,
            submenu: false,
            separator: true,
            toggle: None,
        }];
        let mut popup = MenuPopup::new(rows, 0, MENU_WIDTH, None, &StyleConfig::default()).unwrap();
        assert_eq!(
            popup
                .render(SurfaceId(0), Size::new(MENU_WIDTH, ROW_HEIGHT), None)
                .unwrap()
                .len(),
            (MENU_WIDTH * ROW_HEIGHT * 4) as usize
        );
    }

    #[test]
    fn pointer_leave_does_not_close_menu() {
        let rows = vec![MenuEntry {
            id: 1,
            label: "Open".to_string(),
            enabled: true,
            submenu: false,
            separator: false,
            toggle: None,
        }];
        let mut popup = MenuPopup::new(rows, 1200, 1920, None, &StyleConfig::default()).unwrap();
        popup.handle_event(SurfaceId(0), InputEvent::PointerLeave);
        assert!(!popup.should_close());
        popup.handle_event(SurfaceId(0), InputEvent::FocusLost);
        assert!(popup.should_close());
    }
}
