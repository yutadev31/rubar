use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use zbus::{
    blocking::{Connection, Proxy, connection::Builder as ConnectionBuilder, proxy::Builder},
    interface,
    message::Header,
    object_server::SignalEmitter,
    proxy::CacheProperties,
};

use super::{MenuPosition, TrayItem, TrayProvider, menu};
use crate::{config::StyleConfig, widget::TrayIcon};

const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";
pub(super) const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";

#[derive(Clone)]
pub(super) struct SniItem {
    pub service: String,
    pub path: String,
    icon: Option<TrayIcon>,
    attention_icon: Option<TrayIcon>,
    needs_attention: bool,
}

impl SniItem {
    fn snapshot(&self) -> TrayItem {
        TrayItem {
            id: self.service.clone(),
            icon: self.icon.clone(),
            attention_icon: self.attention_icon.clone(),
            needs_attention: self.needs_attention,
        }
    }
}

type Items = Arc<Mutex<HashMap<String, SniItem>>>;

pub(super) struct Sni {
    items: Items,
    receiver: mpsc::Receiver<Vec<TrayItem>>,
    menu_open: Arc<AtomicBool>,
}

struct MenuOpenGuard(Arc<AtomicBool>);

impl Drop for MenuOpenGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl Sni {
    pub fn new() -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let items = Arc::new(Mutex::new(HashMap::new()));
        let worker_items = Arc::clone(&items);
        thread::Builder::new()
            .name("rubar-sni".to_string())
            .spawn(move || run_dbus(&worker_items, &sender))
            .map_err(|error| format!("failed to start SNI worker: {error}"))?;
        Ok(Self {
            items,
            receiver,
            menu_open: Arc::new(AtomicBool::new(false)),
        })
    }

    fn item(&self, id: &str) -> Option<SniItem> {
        lock_items(&self.items).get(id).cloned()
    }
}

impl TrayProvider for Sni {
    fn poll_items(&mut self) -> Option<Vec<TrayItem>> {
        let mut latest = None;

        while let Ok(items) = self.receiver.try_recv() {
            latest = Some(items);
        }

        latest
    }

    fn activate(&self, id: &str, secondary: bool) {
        let Some(item) = self.item(id) else {
            return;
        };

        let method = if secondary {
            "SecondaryActivate"
        } else {
            "Activate"
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
                tracing::warn!("rubar: could not call tray item {method}: {error}");
            }
        });
    }

    fn open_menu(&self, id: &str, position: MenuPosition, style: &StyleConfig) {
        let Some(item) = self.item(id) else {
            return;
        };

        if self.menu_open.swap(true, Ordering::AcqRel) {
            return;
        }

        let style = style.clone();
        let menu_open = Arc::clone(&self.menu_open);

        thread::spawn(move || {
            let _guard = MenuOpenGuard(menu_open);
            let identity = format!("{}{}", item.service, item.path);

            if let Err(error) = menu::open(
                &item,
                position.button_left,
                position.surface_width,
                position.output.as_ref(),
                &style,
            ) {
                tracing::warn!("rubar: could not open tray menu {identity}: {error}");
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
        publish_items(&self.items, &self.sender);
    }
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    // zbus injects owned header and emitter values into interface methods.
    #[allow(clippy::needless_pass_by_value)]
    fn register_status_notifier_item(
        &mut self,
        service_or_path: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let path = if service_or_path.starts_with('/') {
            service_or_path.to_string()
        } else {
            "/StatusNotifierItem".to_string()
        };

        let service = if service_or_path.starts_with('/') {
            header.sender().map_or_else(
                || service_or_path.to_string(),
                std::string::ToString::to_string,
            )
        } else {
            service_or_path.to_string()
        };

        let item = SniItem {
            service: service.clone(),
            path: path.clone(),
            icon: None,
            attention_icon: None,
            needs_attention: false,
        };

        lock_items(&self.items).insert(service.clone(), item);
        self.publish();

        if let Err(error) = zbus::block_on(Self::status_notifier_item_registered(
            &emitter,
            &format!("{service}{path}"),
        )) {
            tracing::warn!("rubar: could not notify tray item registration: {error}");
        }

        let items = Arc::clone(&self.items);
        let sender = self.sender.clone();
        thread::spawn(move || watch_item(&service, &path, &items, &sender));
    }

    /// Hosts may register with the watcher as well. rubar is the host, so the
    /// registration is intentionally a no-op while the standard method is
    /// still exposed for clients that expect it.
    // zbus interface methods require a receiver even for constant protocol responses.
    #[allow(clippy::unused_self)]
    fn register_status_notifier_host(&mut self, service: &str) {
        tracing::debug!(service, "tray host registered");
    }

    #[zbus(property)]
    // zbus interface methods require a receiver even for constant protocol responses.
    #[allow(clippy::unused_self)]
    fn protocol_version(&self) -> u32 {
        0
    }

    #[zbus(property)]
    // zbus interface methods require a receiver even for constant protocol responses.
    #[allow(clippy::unused_self)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        lock_items(&self.items)
            .values()
            .map(|item| format!("{}{}", item.service, item.path))
            .collect()
    }

    #[zbus(property)]
    // zbus interface methods require a receiver even for constant protocol responses.
    #[allow(clippy::unused_self)]
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

fn run_dbus(items: &Items, sender: &mpsc::Sender<Vec<TrayItem>>) {
    let watcher = Watcher {
        items: Arc::clone(items),
        sender: sender.clone(),
    };
    let Ok(connection) = ConnectionBuilder::session()
        .and_then(|builder| builder.serve_at(WATCHER_PATH, watcher))
        .and_then(|builder| builder.name(WATCHER_INTERFACE))
        .and_then(ConnectionBuilder::build)
        .inspect_err(|error| tracing::warn!("rubar: tray session bus: {error}"))
    else {
        return;
    };

    let watcher_ref = match connection
        .object_server()
        .interface::<_, Watcher>(WATCHER_PATH)
    {
        Ok(watcher_ref) => watcher_ref,
        Err(error) => {
            tracing::error!("rubar: could not access tray watcher interface: {error}");
            return;
        }
    };

    let bus = match Proxy::new(
        &connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    ) {
        Ok(bus) => bus,
        Err(error) => {
            tracing::error!("rubar: could not connect to session bus daemon: {error}");

            return;
        }
    };

    let owner_changes = match bus.receive_signal("NameOwnerChanged") {
        Ok(signals) => signals,
        Err(error) => {
            tracing::error!("rubar: could not subscribe to D-Bus name changes: {error}");
            return;
        }
    };

    for message in owner_changes {
        let (name, old_owner, new_owner) =
            match message.body().deserialize::<(String, String, String)>() {
                Ok(change) => change,
                Err(error) => {
                    tracing::warn!("rubar: could not decode D-Bus name change: {error}");

                    continue;
                }
            };

        // A disappearing service has an old owner and no new owner. Ignore
        // acquisitions and ownership changes where another owner takes over.
        if old_owner.is_empty() || !new_owner.is_empty() {
            continue;
        }

        let removed = lock_items(items).remove(&name);

        if let Some(item) = removed {
            publish_items(items, sender);

            if let Err(error) = zbus::block_on(Watcher::status_notifier_item_unregistered(
                watcher_ref.signal_emitter(),
                &format!("{}{}", item.service, item.path),
            )) {
                tracing::warn!("rubar: could not notify tray item unregistration: {error}");
            }
        }
    }
}

fn watch_item(service: &str, path: &str, items: &Items, sender: &mpsc::Sender<Vec<TrayItem>>) {
    match Connection::session() {
        Ok(connection) => match item_proxy(&connection, service, path) {
            Ok(proxy) => update_icon(&proxy, service, items, sender),
            Err(error) => {
                tracing::warn!("rubar: could not create tray item proxy for {service}: {error}");
            }
        },
        Err(error) => tracing::warn!("rubar: could not connect to tray item {service}: {error}"),
    }

    for signal in ["NewIcon", "NewAttentionIcon", "NewStatus"] {
        let service = service.to_string();
        let path = path.to_string();
        let items = Arc::clone(items);
        let sender = sender.clone();
        thread::spawn(move || watch_item_signal(&service, &path, &items, &sender, signal));
    }
}

fn watch_item_signal(
    service: &str,
    path: &str,
    items: &Items,
    sender: &mpsc::Sender<Vec<TrayItem>>,
    signal: &'static str,
) {
    let connection = match Connection::session() {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!("rubar: could not connect to tray item {service} for {signal}: {error}");
            return;
        }
    };

    let proxy = match item_proxy(&connection, service, path) {
        Ok(proxy) => proxy,
        Err(error) => {
            tracing::warn!("rubar: could not create tray item proxy for {service}: {error}");
            return;
        }
    };

    let mut signals = match proxy.receive_signal(signal) {
        Ok(signals) => signals,
        Err(error) => {
            tracing::warn!(
                "rubar: could not subscribe to {signal} for tray item {service}: {error}"
            );
            return;
        }
    };

    while signals.next().is_some() {
        update_icon(&proxy, service, items, sender);
    }
}

fn item_proxy<'a>(
    connection: &Connection,
    service: &'a str,
    path: &'a str,
) -> zbus::Result<Proxy<'a>> {
    Builder::<Proxy<'a>>::new(connection)
        .destination(service)?
        .path(path)?
        .interface(ITEM_INTERFACE)?
        // SNI's NewIcon signal does not update the D-Bus properties cache.
        .cache_properties(CacheProperties::No)
        .build()
}

fn update_icon(
    proxy: &Proxy<'_>,
    service: &str,
    items: &Items,
    sender: &mpsc::Sender<Vec<TrayItem>>,
) {
    let icon = proxy
        .get_property::<String>("IconName")
        .ok()
        .and_then(|name| load_named_icon(&name, 16))
        .or_else(|| {
            let pixmaps = proxy
                .get_property::<Vec<(i32, i32, Vec<u8>)>>("IconPixmap")
                .unwrap_or_default();
            choose_icon(&pixmaps, 16)
        });

    let attention_icon = proxy
        .get_property::<String>("AttentionIconName")
        .ok()
        .and_then(|name| load_named_icon(&name, 16))
        .or_else(|| {
            let pixmaps = proxy
                .get_property::<Vec<(i32, i32, Vec<u8>)>>("AttentionIconPixmap")
                .unwrap_or_default();
            choose_icon(&pixmaps, 16)
        });
    let needs_attention = proxy
        .get_property::<String>("Status")
        .is_ok_and(|status| status == "NeedsAttention");

    if let Some(item) = lock_items(items).get_mut(service) {
        item.icon = icon;
        item.attention_icon = attention_icon;
        item.needs_attention = needs_attention;
    }

    publish_items(items, sender);
}

fn lock_items(items: &Items) -> std::sync::MutexGuard<'_, HashMap<String, SniItem>> {
    items.lock().unwrap_or_else(|error| {
        // HashMap operations leave valid owned entries when unwinding. Retain
        // the last snapshot and report the failed worker before continuing.
        tracing::error!("SNI item lock poisoned: {error}");
        error.into_inner()
    })
}

fn publish_items(items: &Items, sender: &mpsc::Sender<Vec<TrayItem>>) {
    let mut current = lock_items(items)
        .values()
        .map(SniItem::snapshot)
        .collect::<Vec<_>>();
    current.sort_by(|a, b| a.id.cmp(&b.id));

    if let Err(error) = sender.send(current) {
        tracing::debug!("tray receiver was dropped: {error}");
    }
}

fn choose_icon(pixmaps: &[(i32, i32, Vec<u8>)], target: i32) -> Option<TrayIcon> {
    let valid = pixmaps.iter().filter(|(w, h, bytes)| {
        *w > 0
            && *h > 0
            && (w.cast_unsigned() as usize)
                .checked_mul(h.cast_unsigned() as usize)
                .and_then(|size| size.checked_mul(4))
                .is_some_and(|size| bytes.len() == size)
    });
    let (width, height, bytes) = valid.min_by_key(|(w, h, _)| (w.max(h) - target).abs())?;
    let mut pixels = Vec::with_capacity(bytes.len());

    for argb in bytes.as_chunks::<4>().0 {
        let alpha = u16::from(argb[0]);

        pixels.extend_from_slice(&[
            u8::try_from((u16::from(argb[1]) * alpha) / 255)
                .expect("premultiplied channel is at most 255"),
            u8::try_from((u16::from(argb[2]) * alpha) / 255)
                .expect("premultiplied channel is at most 255"),
            u8::try_from((u16::from(argb[3]) * alpha) / 255)
                .expect("premultiplied channel is at most 255"),
            argb[0],
        ]);
    }

    Some(TrayIcon {
        width: width.cast_unsigned(),
        height: height.cast_unsigned(),
        pixels,
    })
}

fn load_named_icon(name: &str, target: u32) -> Option<TrayIcon> {
    if name.is_empty() || Path::new(name).components().count() != 1 {
        return None;
    }

    crate::desktop_icons::Loader::new(["hicolor", "Adwaita", "Papirus", "breeze"]).load(
        name,
        target,
        crate::desktop_icons::RasterSize::Original,
    )
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
                (16, 16, [128, 255, 64, 32].repeat(16 * 16)),
                (32, 32, [255, 1, 2, 3].repeat(32 * 32)),
            ],
            16,
        )
        .expect("valid icon");
        assert_eq!((icon.width, icon.height), (16, 16));
        assert_eq!(icon.pixels[..4], [128, 32, 16, 128]);
    }
}
