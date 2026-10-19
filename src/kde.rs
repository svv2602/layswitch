//! KDE Plasma session services over D-Bus: keyboard layouts and notifications.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Result;
use zbus::blocking::Connection;
use zbus::zvariant::Value;

use crate::layout::Lang;

const KB_DEST: &str = "org.kde.keyboard";
const KB_PATH: &str = "/Layouts";
const KB_IFACE: &str = "org.kde.KeyboardLayouts";

pub struct Kde {
    conn: Connection,
    layouts: Vec<Option<Lang>>,
    fetched: Option<Instant>,
    notification: u32,
}

impl Kde {
    pub fn connect() -> Result<Kde> {
        // Keys are held up while we wait for KWin, so never wait long.
        let conn = zbus::blocking::connection::Builder::session()?.method_timeout(Duration::from_millis(300)).build()?;
        let mut kde = Kde { conn, layouts: Vec::new(), fetched: None, notification: 0 };
        let names = kde.layout_names()?;
        log::info!("KDE layouts: {names:?}");
        Ok(kde)
    }

    fn call<B, R>(&self, method: &str, body: &B) -> Result<R>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'de> serde::Deserialize<'de> + zbus::zvariant::Type,
    {
        let reply = self.conn.call_method(Some(KB_DEST), KB_PATH, Some(KB_IFACE), method, body)?;
        Ok(reply.body().deserialize()?)
    }

    pub fn layout_names(&mut self) -> Result<Vec<String>> {
        let list: Vec<(String, String, String)> = self.call("getLayoutsList", &())?;
        self.layouts = list.iter().map(|(short, _, _)| Lang::from_xkb(short)).collect();
        self.fetched = Some(Instant::now());
        Ok(list.into_iter().map(|(short, _, _)| short).collect())
    }

    /// Supported languages per layout index; refreshed now and then in case settings change.
    pub fn layouts(&mut self) -> Vec<Option<Lang>> {
        if self.fetched.is_none_or(|t| t.elapsed() > Duration::from_secs(10))
            && let Err(e) = self.layout_names()
        {
            log::warn!("getLayoutsList: {e}");
        }
        self.layouts.clone()
    }

    pub fn current(&self) -> Option<usize> {
        self.call::<_, u32>("getLayout", &()).map_err(|e| log::warn!("getLayout: {e}")).ok().map(|i| i as usize)
    }

    pub fn set(&self, idx: usize) -> bool {
        self.call::<_, bool>("setLayout", &(idx as u32)).map_err(|e| log::warn!("setLayout: {e}")).unwrap_or(false)
    }

    pub fn notify(&mut self, text: &str) {
        let hints: HashMap<&str, Value> = HashMap::new();
        let body = ("layswitch", self.notification, "input-keyboard", "layswitch", text, Vec::<&str>::new(), hints, 1500i32);
        let reply = self.conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &body,
        );
        match reply.and_then(|r| r.body().deserialize::<u32>()) {
            Ok(id) => self.notification = id,
            Err(e) => log::warn!("notify: {e}"),
        }
    }
}
