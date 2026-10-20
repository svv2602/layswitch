//! System tray icon (StatusNotifierItem) with on/off state and a small menu.

use std::path::Path;
use std::process::Command as Process;
use std::sync::mpsc::Sender;
use std::thread;

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{MenuItem, ToolTip};
use log::warn;

use crate::config;
use crate::devices::Msg;

pub enum Command {
    SetEnabled(bool),
    SetAutocorrect(bool),
    Quit,
}

pub struct Tray {
    pub enabled: bool,
    pub autocorrect: bool,
    /// Last change made to the text, for the tooltip.
    pub last: Option<String>,
    tx: Sender<Msg>,
}

impl Tray {
    fn send(&self, cmd: Command) {
        let _ = self.tx.send(Msg::Tray(cmd));
    }
}

/// Opens a file in the default editor, creating it from `default` first.
fn open_file(path: &Path, default: &str) {
    if !path.exists() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, default) {
            warn!("create {}: {e}", path.display());
        }
    }
    match Process::new("xdg-open").arg(path).spawn() {
        Ok(mut child) => {
            thread::spawn(move || child.wait());
        }
        Err(e) => warn!("xdg-open: {e}"),
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "layswitch".into()
    }

    fn title(&self) -> String {
        "layswitch".into()
    }

    fn icon_name(&self) -> String {
        if self.enabled { "input-keyboard-virtual-on" } else { "input-keyboard-virtual-off" }.into()
    }

    fn tool_tip(&self) -> ToolTip {
        let state = if self.enabled { "Автопереключение включено" } else { "Автопереключение выключено" };
        let mut description = format!("{state}\nPause — откат, Shift+Pause — вкл/выкл");
        if let Some(last) = &self.last {
            description.push_str(&format!("\nПоследнее: {last}"));
        }
        ToolTip { title: "layswitch".into(), description, ..Default::default() }
    }

    /// Left click toggles auto switching.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.enabled = !self.enabled;
        self.send(Command::SetEnabled(self.enabled));
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            CheckmarkItem {
                label: "Автопереключение раскладки".into(),
                checked: self.enabled,
                activate: Box::new(|t: &mut Self| {
                    t.enabled = !t.enabled;
                    t.send(Command::SetEnabled(t.enabled));
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Исправление опечаток".into(),
                checked: self.autocorrect,
                activate: Box::new(|t: &mut Self| {
                    t.autocorrect = !t.autocorrect;
                    t.send(Command::SetAutocorrect(t.autocorrect));
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Исключения…".into(),
                icon_name: "view-list-text".into(),
                activate: Box::new(|_: &mut Self| open_file(&config::dir().join("ignore.txt"), "")),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Мои слова…".into(),
                icon_name: "accessories-dictionary".into(),
                activate: Box::new(|_: &mut Self| {
                    open_file(&config::dir().join("words.txt"), config::WORDS_HEADER)
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Настройки…".into(),
                icon_name: "configure".into(),
                activate: Box::new(|_: &mut Self| {
                    open_file(&config::dir().join("config.toml"), config::EXAMPLE)
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Выход".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|t: &mut Self| t.send(Command::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Starts the tray icon; returns `None` if the panel has no tray (the daemon works without it).
pub fn spawn(enabled: bool, autocorrect: bool, tx: Sender<Msg>) -> Option<Handle<Tray>> {
    Tray { enabled, autocorrect, last: None, tx }
        .spawn()
        .map_err(|e| warn!("tray icon unavailable: {e}"))
        .ok()
}
