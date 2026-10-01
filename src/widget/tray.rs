//! Wayland system tray through the freedesktop StatusNotifier protocol.
//!
//! StatusNotifierItem is a D-Bus protocol, not a Wayland protocol.  The
//! watcher lives on a small thread so the Wayland event loop stays responsive.
//! Items that expose `IconPixmap` are rendered directly from their ARGB pixel
//! data. Items without a pixmap keep the service-name fallback.

use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use dbus::{Message, arg::Variant, blocking::Connection, message::MatchRule};

use super::{MouseButton, ScrollDirection, Widget, WidgetButton, WidgetContent, WidgetIcon};

#[derive(Clone, Default)]
struct State {
    items: Arc<Mutex<Vec<Item>>>,
    redraw_requested: Arc<AtomicBool>,
}

#[derive(Clone)]
struct Item {
    service: String,
    label: String,
    icon: Option<WidgetIcon>,
}

pub struct Tray {
    state: State,
    started: bool,
}

impl Tray {
    pub fn new() -> Self {
        let state = State::default();
        let thread_state = state.clone();
        let started = thread::Builder::new()
            .name("rubar-status-notifier".to_string())
            .spawn(move || run_watcher(thread_state))
            .is_ok();
        Self { state, started }
    }
}

impl Widget for Tray {
    fn content(&mut self) -> WidgetContent {
        let mut items = self.state.items.lock().unwrap().clone();
        if !self.started {
            return WidgetContent::Text("TRAY".to_string());
        }
        if items.is_empty() {
            return WidgetContent::Text(String::new());
        }
        items.sort_by(|left, right| left.label.cmp(&right.label));
        WidgetContent::Buttons(
            items
                .into_iter()
                .map(|item| WidgetButton {
                    // A successfully loaded icon is the complete tray item;
                    // keep the label only as the existing fallback for items
                    // whose icon could not be loaded.
                    text: item.icon.is_none().then_some(item.label),
                    icon: item.icon,
                    padding: Some(4),
                    bold: None,
                    color: None,
                    background: None,
                })
                .collect(),
        )
    }

    fn on_click(&mut self, _button: MouseButton, _item: usize) {}

    fn on_scroll(&mut self, _direction: ScrollDirection, _item: usize) {}

    fn take_redraw_request(&self) -> bool {
        self.state.redraw_requested.swap(false, Ordering::AcqRel)
    }
}

fn run_watcher(state: State) {
    let connection = match Connection::new_session() {
        Ok(connection) => connection,
        Err(error) => {
            eprintln!("rubar: Wayland tray could not connect to D-Bus: {error}");
            return;
        }
    };
    if let Err(error) = connection.request_name("org.kde.StatusNotifierWatcher", false, false, true)
    {
        eprintln!("rubar: Wayland tray could not claim StatusNotifierWatcher: {error}");
        return;
    }

    let path = "/StatusNotifierWatcher";
    let interface = "org.kde.StatusNotifierWatcher";
    let rule = MatchRule::new_method_call()
        .with_path(path)
        .with_interface(interface);
    let callback_state = state.clone();
    if connection
        .add_match(
            rule,
            move |args: (String,), connection: &Connection, message: &Message| {
                // The watcher interface also contains
                // RegisterStatusNotifierHost. It is not a tray item and
                // must not create a visible fallback button.
                let is_item = message.member().map(|member| member.to_string())
                    == Some("RegisterStatusNotifierItem".to_string());
                if !is_item {
                    let _ = connection.channel().send(message.method_return());
                    return true;
                }
                let service = args.0;
                if !service.is_empty() {
                    // `:1.383` is D-Bus's private, temporary service name.  It
                    // is an implementation detail and must not leak into the
                    // bar as a user-facing tray label.
                    let label = if service.starts_with(':') {
                        "TRAY".to_string()
                    } else {
                        service.rsplit('.').next().unwrap_or("TRAY").to_string()
                    };
                    let item_key = message
                        .sender()
                        .map(|sender| sender.to_string())
                        .unwrap_or_else(|| service.clone());
                    let (bus_name, object_path) = if service.starts_with('/') {
                        (item_key.clone(), service.clone())
                    } else {
                        (service.clone(), "/StatusNotifierItem".to_string())
                    };
                    let mut items = callback_state.items.lock().unwrap();
                    if let Some(item) = items.iter_mut().find(|item| item.service == item_key) {
                        item.label = label;
                    } else {
                        items.push(Item {
                            service: item_key.clone(),
                            label,
                            icon: None,
                        });
                    }
                    callback_state
                        .redraw_requested
                        .store(true, Ordering::Release);
                    // Do not block the watcher callback on a remote property
                    // lookup. Some clients only expose their icon after the
                    // registration call has returned.
                    load_icon_later(callback_state.clone(), item_key, bus_name, object_path);
                }
                let _ = connection.channel().send(message.method_return());
                true
            },
        )
        .is_err()
    {
        eprintln!("rubar: Wayland tray could not register StatusNotifierWatcher");
        return;
    }

    // Some clients register first and publish IconPixmap/IconName later.
    let property_state = state.clone();
    if connection
        .add_match(
            MatchRule::new_signal("org.freedesktop.DBus.Properties", "PropertiesChanged"),
            move |_: (String, dbus::arg::PropMap, Vec<String>),
                  _connection: &Connection,
                  message: &Message| {
                let Some(sender) = message.sender().map(|sender| sender.to_string()) else {
                    return true;
                };
                let Some(path) = message.path().map(|path| path.to_string()) else {
                    return true;
                };
                let is_item = property_state
                    .items
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|item| item.service == sender);
                if is_item {
                    load_icon_later(property_state.clone(), sender.clone(), sender, path);
                }
                true
            },
        )
        .is_err()
    {
        eprintln!("rubar: Wayland tray could not watch StatusNotifierItem properties");
    }

    loop {
        if connection.process(Duration::from_millis(1000)).is_err() {
            break;
        }
    }
}

fn load_icon_later(state: State, item_key: String, bus_name: String, object_path: String) {
    thread::spawn(move || {
        for _ in 0..30 {
            let Ok(icon_connection) = Connection::new_session() else {
                return;
            };
            if let Some(icon) = fetch_icon(&icon_connection, &bus_name, &object_path) {
                let mut items = state.items.lock().unwrap();
                if let Some(item) = items.iter_mut().find(|item| item.service == item_key) {
                    item.icon = Some(icon);
                }
                state.redraw_requested.store(true, Ordering::Release);
                return;
            }
            thread::sleep(Duration::from_millis(200));
        }
    });
}

fn fetch_icon(connection: &Connection, service: &str, path: &str) -> Option<WidgetIcon> {
    let proxy = connection.with_proxy(service, path, Duration::from_millis(250));
    let result: Result<(Variant<Vec<(i32, i32, Vec<u8>)>>,), _> = proxy.method_call(
        "org.freedesktop.DBus.Properties",
        "Get",
        ("org.kde.StatusNotifierItem", "IconPixmap"),
    );
    if let Some((width, height, pixels)) = result.ok().and_then(|value| {
        value
            .0
            .0
            .into_iter()
            .filter(|(width, height, pixels)| {
                *width > 0 && *height > 0 && pixels.len() >= (*width * *height * 4) as usize
            })
            .min_by_key(|(width, height, _)| (*width - 22).abs() + (*height - 22).abs())
    }) {
        return Some(WidgetIcon {
            width: width as u32,
            height: height as u32,
            pixels,
        });
    }

    let icon_name: String = proxy
        .method_call(
            "org.freedesktop.DBus.Properties",
            "Get",
            ("org.kde.StatusNotifierItem", "IconName"),
        )
        .ok()
        .map(|value: (Variant<String>,)| value.0.0)?;
    decode_icon_name(&icon_name)
}

fn decode_icon_name(name: &str) -> Option<WidgetIcon> {
    let name = Path::new(name);
    let mut candidates = Vec::new();
    if name.is_absolute() {
        candidates.push(name.to_path_buf());
    } else {
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            });
        let mut data_dirs = std::env::var_os("XDG_DATA_DIRS")
            .map(|dirs| std::env::split_paths(&dirs).collect::<Vec<_>>())
            .unwrap_or_else(|| {
                vec![
                    PathBuf::from("/usr/local/share"),
                    PathBuf::from("/usr/share"),
                ]
            });
        if let Some(home) = data_home {
            data_dirs.insert(0, home);
        }
        for base in data_dirs {
            for size in ["22x22", "24x24", "32x32", "48x48", "scalable"] {
                for category in ["apps", "status"] {
                    candidates.push(
                        base.join("icons/hicolor")
                            .join(size)
                            .join(category)
                            .join(name)
                            .with_extension("png"),
                    );
                }
            }
            candidates.push(base.join("pixmaps").join(name).with_extension("png"));
        }
    }
    candidates.into_iter().find_map(|path| decode_png(&path))
}

fn decode_png(path: &Path) -> Option<WidgetIcon> {
    let decoder = png::Decoder::new(File::open(path).ok()?);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let output = reader.next_frame(&mut buffer).ok()?;
    let bytes = &buffer[..output.buffer_size()];
    let mut pixels = Vec::with_capacity((output.width * output.height * 4) as usize);
    match output.color_type {
        png::ColorType::Rgba => {
            for rgba in bytes.chunks_exact(4) {
                pixels.extend_from_slice(&[rgba[3], rgba[0], rgba[1], rgba[2]]);
            }
        }
        png::ColorType::Rgb => {
            for rgb in bytes.chunks_exact(3) {
                pixels.extend_from_slice(&[255, rgb[0], rgb[1], rgb[2]]);
            }
        }
        _ => return None,
    }
    Some(WidgetIcon {
        width: output.width,
        height: output.height,
        pixels,
    })
}
