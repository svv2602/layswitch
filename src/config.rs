//! User configuration in ~/.config/layswitch/.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Commented example config, written when the user opens settings from the tray.
pub const EXAMPLE: &str = include_str!("../dist/config.toml");

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Fix words typed in the wrong layout.
    pub auto_switch: bool,
    /// Fix single-letter typos (off by default: names and slang get "fixed" too).
    pub autocorrect: bool,
    /// Hotkey: undo / convert last word. Shift+hotkey toggles auto mode, Ctrl+hotkey converts selection.
    pub hotkey: String,
    /// Pause after switching layout before retyping, in milliseconds.
    pub switch_delay_ms: u64,
    /// Margin for switching without a dictionary hit (average log-probability per char).
    pub lm_margin: f32,
    /// Minimum length of a word for the language-model fallback.
    pub min_lm_len: usize,
    pub notifications: bool,
    /// Substrings of device names to leave alone.
    pub exclude_devices: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            auto_switch: true,
            autocorrect: false,
            hotkey: "KEY_PAUSE".into(),
            switch_delay_ms: 30,
            lm_margin: 1.0,
            min_lm_len: 4,
            notifications: true,
            exclude_devices: Vec::new(),
        }
    }
}

pub fn dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("layswitch")
}

impl Config {
    pub fn load() -> Result<Config> {
        let path = dir().join("config.toml");
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("parse {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
        }
    }
}

/// Words the user reverted with the hotkey; they are never changed automatically again.
pub fn load_ignored() -> HashSet<String> {
    fs::read_to_string(dir().join("ignore.txt"))
        .map(|t| t.lines().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

/// Header of a new words.txt, written when the user opens it from the tray.
pub const WORDS_HEADER: &str = "\
# Свои слова: не исправляются и служат целью переключения (rjvvbn -> коммит).
# По одному или через пробел; строки с # игнорируются.
# После правки: systemctl --user restart layswitch
";

/// The user's own words (~/.config/layswitch/words.txt), any language.
pub fn load_words() -> Vec<String> {
    fs::read_to_string(dir().join("words.txt"))
        .map(|t| crate::dict::word_list(&t).map(String::from).collect())
        .unwrap_or_default()
}

pub fn append_ignored(word: &str) -> Result<()> {
    fs::create_dir_all(dir())?;
    let mut f = OpenOptions::new().create(true).append(true).open(dir().join("ignore.txt"))?;
    writeln!(f, "{word}")?;
    Ok(())
}
