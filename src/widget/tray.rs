use std::{
    collections::HashMap,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
};

use zbus::{
    blocking::{Connection, Proxy},
    interface,
    message::Header,
    object_server::SignalEmitter,
};

use crate::widget::{MouseButton, TrayIcon, Widget, WidgetButton, WidgetContent};

const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";
const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";

#[derive(Clone, Debug)]
pub struct TrayItem {
    pub service: String,
    pub path: String,
    pub icon: Option<TrayIcon>,
}
type Items = Arc<Mutex<HashMap<String, TrayItem>>>;

pub struct Tray {
    receiver: mpsc::Receiver<Vec<TrayItem>>,
    items: Vec<TrayItem>,
}

impl Tray {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("rubar-sni".to_string())
            .spawn(move || run_dbus(sender))
            .expect("failed to start SNI worker");
        Self {
            receiver,
            items: Vec::new(),
        }
    }
}

impl Widget for Tray {
    fn content(&mut self) -> WidgetContent {
        while let Ok(items) = self.receiver.try_recv() {
            self.items = items;
        }
        WidgetContent::Buttons(
            self.items
                .iter()
                .map(|item| WidgetButton {
                    text: None,
                    icon: item.icon.clone(),
                    padding: Some(4),
                    bold: None,
                    color: None,
                    background: None,
                })
                .collect(),
        )
    }

    fn on_click(&mut self, button: MouseButton, item: usize) {
        let method = match button {
            MouseButton::Left => "Activate",
            MouseButton::Middle => "SecondaryActivate",
            _ => return,
        };
        let Some(item) = self.items.get(item).cloned() else {
            return;
        };

        thread::spawn(move || {
            let result = (|| {
                let connection = Connection::session()?;
                let proxy = Proxy::new(
                    &connection,
                    item.service.as_str(),
                    item.path.as_str(),
                    ITEM_INTERFACE,
                )?;
                proxy.call_method(method, &(0_i32, 0_i32))?;
                Ok::<_, zbus::Error>(())
            })();
            if let Err(error) = result {
                eprintln!("rubar: could not call tray item {method}: {error}");
            }
        });
    }
}

struct Watcher {
    items: Items,
    sender: mpsc::Sender<Vec<TrayItem>>,
}

impl Watcher {
    fn publish(&self) {
        let mut items = self
            .items
            .lock()
            .expect("SNI item lock poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        items.sort_by(|a, b| a.service.cmp(&b.service));
        let _ = self.sender.send(items);
    }
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(
        &mut self,
        service_or_path: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let (service, path) = if service_or_path.starts_with('/') {
            (
                header
                    .sender()
                    .map_or_else(|| service_or_path.to_string(), |s| s.to_string()),
                service_or_path.to_string(),
            )
        } else {
            (
                service_or_path.to_string(),
                "/StatusNotifierItem".to_string(),
            )
        };
        let item = TrayItem {
            service: service.clone(),
            path: path.clone(),
            icon: None,
        };
        self.items
            .lock()
            .expect("SNI item lock poisoned")
            .insert(service.clone(), item);
        self.publish();
        let _ = zbus::block_on(Self::status_notifier_item_registered(
            &emitter,
            &format!("{service}{path}"),
        ));
        let items = Arc::clone(&self.items);
        let sender = self.sender.clone();
        thread::spawn(move || watch_item(service, path, items, sender));
    }

    /// Hosts may register with the watcher as well. rubar is the host, so the
    /// registration is intentionally a no-op while the standard method is
    /// still exposed for clients that expect it.
    fn register_status_notifier_host(&mut self, _service: &str) {}

    #[zbus(property)]
    fn protocol_version(&self) -> u32 {
        0
    }
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.items
            .lock()
            .expect("SNI item lock poisoned")
            .values()
            .map(|item| format!("{}{}", item.service, item.path))
            .collect()
    }
    #[zbus(property)]
    fn registered_status_notifier_hosts(&self) -> Vec<String> {
        vec!["rubar".to_string()]
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;
}

fn run_dbus(sender: mpsc::Sender<Vec<TrayItem>>) {
    let Ok(connection) = Connection::session() else {
        return;
    };
    if connection.request_name(WATCHER_INTERFACE).is_err() {
        return;
    }
    let items = Arc::new(Mutex::new(HashMap::new()));
    let watcher = Watcher {
        items: Arc::clone(&items),
        sender: sender.clone(),
    };
    if connection
        .object_server()
        .at(WATCHER_PATH, watcher)
        .is_err()
    {
        return;
    }
    let Ok(watcher_ref) = connection
        .object_server()
        .interface::<_, Watcher>(WATCHER_PATH)
    else {
        return;
    };
    let Ok(bus) = Proxy::new(
        &connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    ) else {
        return;
    };
    let Ok(owner_changes) = bus.receive_signal("NameOwnerChanged") else {
        return;
    };
    for message in owner_changes {
        let Ok((name, old_owner, new_owner)) =
            message.body().deserialize::<(String, String, String)>()
        else {
            continue;
        };
        if !old_owner.is_empty() || !new_owner.is_empty() {
            continue;
        }
        let removed = items.lock().expect("SNI item lock poisoned").remove(&name);
        if let Some(item) = removed {
            let mut current = items
                .lock()
                .expect("SNI item lock poisoned")
                .values()
                .cloned()
                .collect::<Vec<_>>();
            current.sort_by(|a, b| a.service.cmp(&b.service));
            let _ = sender.send(current);
            let _ = zbus::block_on(Watcher::status_notifier_item_unregistered(
                watcher_ref.signal_emitter(),
                &format!("{}{}", item.service, item.path),
            ));
        }
    }
}

fn watch_item(service: String, path: String, items: Items, sender: mpsc::Sender<Vec<TrayItem>>) {
    let Ok(connection) = Connection::session() else {
        return;
    };
    let Ok(proxy) = Proxy::new(&connection, service.as_str(), path.as_str(), ITEM_INTERFACE) else {
        return;
    };
    update_icon(&proxy, &service, &items, &sender);
    let Ok(mut signals) = proxy.receive_signal("NewIcon") else {
        return;
    };
    while signals.next().is_some() {
        update_icon(&proxy, &service, &items, &sender);
    }
}

fn update_icon(
    proxy: &Proxy<'_>,
    service: &str,
    items: &Items,
    sender: &mpsc::Sender<Vec<TrayItem>>,
) {
    let pixmaps = proxy
        .get_property::<Vec<(i32, i32, Vec<u8>)>>("IconPixmap")
        .unwrap_or_default();
    let icon = choose_icon(&pixmaps, 16).or_else(|| {
        proxy
            .get_property::<String>("IconName")
            .ok()
            .and_then(|name| load_named_icon(&name, 16))
    });
    if let Some(item) = items
        .lock()
        .expect("SNI item lock poisoned")
        .get_mut(service)
    {
        item.icon = icon;
    }
    publish_items(items, sender);
}

fn publish_items(items: &Items, sender: &mpsc::Sender<Vec<TrayItem>>) {
    let mut current = items
        .lock()
        .expect("SNI item lock poisoned")
        .values()
        .cloned()
        .collect::<Vec<_>>();
    current.sort_by(|a, b| a.service.cmp(&b.service));
    let _ = sender.send(current);
}

fn choose_icon(pixmaps: &[(i32, i32, Vec<u8>)], target: i32) -> Option<TrayIcon> {
    let valid = pixmaps.iter().filter(|(w, h, bytes)| {
        *w > 0
            && *h > 0
            && (*w as usize)
                .checked_mul(*h as usize)
                .and_then(|size| size.checked_mul(4))
                .is_some_and(|size| bytes.len() == size)
    });
    let (width, height, bytes) = valid.min_by_key(|(w, h, _)| (w.max(h) - target).abs())?;
    let mut pixels = Vec::with_capacity(bytes.len());
    for argb in bytes.as_chunks::<4>().0 {
        let alpha = argb[0] as u16;
        pixels.extend_from_slice(&[
            ((argb[1] as u16 * alpha) / 255) as u8,
            ((argb[2] as u16 * alpha) / 255) as u8,
            ((argb[3] as u16 * alpha) / 255) as u8,
            argb[0],
        ]);
    }
    Some(TrayIcon {
        width: *width as u32,
        height: *height as u32,
        pixels,
    })
}

fn load_named_icon(name: &str, target: u32) -> Option<TrayIcon> {
    if name.is_empty() || Path::new(name).components().count() != 1 {
        return None;
    }
    let mut roots = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .into_iter()
        .map(|path| path.join("icons"))
        .collect::<Vec<_>>();
    let data_dirs = std::env::var_os("XDG_DATA_DIRS")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_else(|| {
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
        });
    for dir in data_dirs {
        roots.push(dir.join("icons"));
        roots.push(dir.join("pixmaps"));
    }
    let sizes = [
        target.to_string(),
        "16x16".into(),
        "22x22".into(),
        "24x24".into(),
        "32x32".into(),
        "scalable".into(),
    ];
    let contexts = ["apps", "status", "actions", "devices", "places"];
    let mut candidates = Vec::new();
    for root in roots {
        for theme in ["hicolor", "Adwaita", "Papirus", "breeze"] {
            for size in &sizes {
                for context in contexts {
                    candidates.push(
                        root.join(theme)
                            .join(size)
                            .join(context)
                            .join(format!("{name}.png")),
                    );
                }
            }
        }
        candidates.push(root.join(format!("{name}.png")));
    }
    candidates.into_iter().find_map(|path| load_png_icon(&path))
}

fn load_png_icon(path: &Path) -> Option<TrayIcon> {
    let decoder = png::Decoder::new(BufReader::new(File::open(path).ok()?));
    let mut reader = decoder.read_info().ok()?;
    let mut bytes = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut bytes).ok()?;
    let rgba = match info.color_type {
        png::ColorType::Rgba => bytes[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => bytes[..info.buffer_size()]
            .chunks_exact(3)
            .flat_map(|px| [px[0], px[1], px[2], 255])
            .collect(),
        _ => return None,
    };
    let pixels = rgba
        .chunks_exact(4)
        .flat_map(|px| {
            let alpha = px[3] as u16;
            [
                ((px[0] as u16 * alpha) / 255) as u8,
                ((px[1] as u16 * alpha) / 255) as u8,
                ((px[2] as u16 * alpha) / 255) as u8,
                px[3],
            ]
        })
        .collect();
    Some(TrayIcon {
        width: info.width,
        height: info.height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::choose_icon;

    #[test]
    fn rejects_empty_and_malformed_pixmaps() {
        assert!(choose_icon(&[], 16).is_none());
        assert!(choose_icon(&[(16, 16, vec![0; 3])], 16).is_none());
        assert!(choose_icon(&[(-1, 16, vec![0; 64])], 16).is_none());
    }

    #[test]
    fn selects_closest_size_and_converts_network_argb() {
        let icon = choose_icon(
            &[
                (8, 8, vec![0; 8 * 8 * 4]),
                (16, 16, vec![128, 255, 64, 32].repeat(16 * 16)),
                (32, 32, vec![255, 1, 2, 3].repeat(32 * 32)),
            ],
            16,
        )
        .expect("valid icon");
        assert_eq!((icon.width, icon.height), (16, 16));
        assert_eq!(icon.pixels[..4], [128, 32, 16, 128]);
    }
}
