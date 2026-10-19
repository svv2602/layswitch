//! layswitch: automatic keyboard layout switching (en/ru/uk) for KDE Plasma on Wayland.

mod config;
mod detect;
mod devices;
mod dict;
mod engine;
mod kde;
mod layout;
mod tray;

use std::io::Read;
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use evdev::uinput::VirtualDevice;
use evdev::{EventType, InputEvent, KeyCode};
use log::{info, warn};

use config::Config;
use detect::Detector;
use devices::{Leds, Msg};
use dict::{Dict, Dicts};
use engine::{Engine, Options, System};
use layout::Lang;

struct Desktop {
    keyboard: VirtualDevice,
    kde: kde::Kde,
    leds: Leds,
    switch_delay: Duration,
    notifications: bool,
    tray: Option<ksni::blocking::Handle<tray::Tray>>,
}

impl System for Desktop {
    fn emit(&mut self, key: KeyCode, value: i32) {
        let ev = InputEvent::new(EventType::KEY.0, key.code(), value);
        if let Err(e) = self.keyboard.emit(&[ev]) {
            warn!("uinput: {e}");
        }
    }

    fn current_layout(&mut self) -> Option<usize> {
        self.kde.current()
    }

    fn set_layout(&mut self, idx: usize) -> bool {
        self.kde.set(idx)
    }

    fn layouts(&mut self) -> Vec<Option<Lang>> {
        self.kde.layouts()
    }

    fn settle(&mut self) {
        thread::sleep(self.switch_delay);
    }

    fn notify(&mut self, text: &str) {
        if self.notifications {
            self.kde.notify(text);
        }
    }

    fn primary_selection(&mut self) -> Option<String> {
        read_primary_selection().map_err(|e| warn!("primary selection: {e}")).ok()
    }

    fn caps_led(&mut self, on: bool) {
        self.leds.set_caps(on);
    }

    fn remember_ignored(&mut self, word: &str) {
        if let Err(e) = config::append_ignored(word) {
            warn!("save ignore list: {e}");
        }
    }

    fn state_changed(&mut self, enabled: bool, autocorrect: bool) {
        if let Some(tray) = &self.tray {
            tray.update(|t| {
                t.enabled = enabled;
                t.autocorrect = autocorrect;
            });
        }
    }

    fn corrected(&mut self, from: &str, to: &str) {
        if let Some(tray) = &self.tray {
            let last = format!("{from} → {to}");
            tray.update(|t| t.last = Some(last));
        }
    }
}

/// Runs wl-paste with a hard timeout: the keyboard is frozen while we wait.
fn read_primary_selection() -> Result<String> {
    let mut child = Command::new("wl-paste")
        .args(["--primary", "--no-newline"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("run wl-paste (apt install wl-clipboard)")?;
    let stdout = child.stdout.take().expect("piped stdout");
    let reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.take(64 * 1024).read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > Duration::from_millis(500) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!("wl-paste timed out"));
        }
        thread::sleep(Duration::from_millis(10));
    }
    let bytes = reader.join().map_err(|_| anyhow!("reader panicked"))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn detector(cfg: &Config) -> Detector {
    Detector { dicts: Dicts::builtin(), lm_margin: cfg.lm_margin, min_lm_len: cfg.min_lm_len }
}

fn run(cfg: Config) -> Result<()> {
    let hotkey = KeyCode::from_str(&cfg.hotkey).map_err(|_| anyhow!("unknown hotkey {:?}", cfg.hotkey))?;
    let started = Instant::now();
    let detector = detector(&cfg);
    info!("dictionaries loaded in {:?}", started.elapsed());

    let kde = kde::Kde::connect().context("connect to KDE keyboard service on the session bus")?;
    let keyboard = devices::virtual_keyboard()?;
    let mut pointer = devices::virtual_pointer()?;
    let (tx, rx) = mpsc::channel();
    let tray = tray::spawn(true, cfg.autocorrect, tx.clone());
    let sys = Desktop {
        keyboard,
        kde,
        leds: Leds::new(),
        switch_delay: Duration::from_millis(cfg.switch_delay_ms),
        notifications: cfg.notifications,
        tray,
    };
    let opts = Options { auto_switch: cfg.auto_switch, autocorrect: cfg.autocorrect, hotkey };
    let mut engine = Engine::new(sys, detector, opts, config::load_ignored());

    devices::spawn_scanner(tx, cfg.exclude_devices);
    info!("running; hotkey {hotkey:?}");

    for msg in rx {
        match msg {
            Msg::Key(key, value) => engine.handle_key(key, value),
            Msg::Pointer(events) => {
                if events.iter().any(|e| e.event_type() == EventType::KEY && e.value() == 1) {
                    engine.reset();
                }
                if let Err(e) = pointer.emit(&events) {
                    warn!("uinput pointer: {e}");
                }
            }
            Msg::Click => engine.reset(),
            Msg::Keyboard(path) => {
                if let Some(caps) = engine.sys.leds.add(&path) {
                    engine.set_caps(caps);
                }
            }
            Msg::Tray(tray::Command::SetEnabled(on)) => engine.set_enabled(on),
            Msg::Tray(tray::Command::SetAutocorrect(on)) => engine.set_autocorrect(on),
            Msg::Tray(tray::Command::Quit) => break,
        }
    }
    info!("quit");
    Ok(())
}

/// Prints what the detector would do with each word, as if typed in its own script's layout.
fn check(cfg: &Config, words: &[String]) {
    let det = detector(cfg);
    let layouts = [Some(Lang::En), Some(Lang::Ru), Some(Lang::Uk)];
    for w in words {
        let Some((cur, strokes)) =
            Lang::ALL.iter().enumerate().find_map(|(i, &l)| layout::encode(l, w).map(|s| (i, s)))
        else {
            println!("{w}: cannot be typed");
            continue;
        };
        match det.decide(&strokes, false, cur, &layouts, &[cur]) {
            Some(sw) => println!("{w} [{}] -> {} [{}]", Lang::ALL[cur].name(), sw.word, Lang::ALL[sw.target].name()),
            None => match det.typo_fix(Lang::ALL[cur], w) {
                Some(fix) => println!("{w} [{}]: keep (typo fix: {fix})", Lang::ALL[cur].name()),
                None => println!("{w} [{}]: keep", Lang::ALL[cur].name()),
            },
        }
    }
}

/// Measures the language-model fallback on words the dictionaries do not contain.
fn eval() {
    const TRAIN: usize = 30000;
    let texts = [
        (Lang::En, include_str!("../data/en_50k.txt")),
        (Lang::Ru, include_str!("../data/ru_50k.txt")),
        (Lang::Uk, include_str!("../data/uk_50k.txt")),
    ];
    let layouts = [Some(Lang::En), Some(Lang::Ru), Some(Lang::Uk)];
    let tests: Vec<(usize, Vec<String>)> = texts
        .iter()
        .enumerate()
        .map(|(i, (lang, text))| {
            let words = text
                .lines()
                .skip(TRAIN)
                .filter_map(|l| l.split_whitespace().next())
                .filter(|w| w.chars().count() >= 4 && w.chars().all(|c| lang.is_letter(c)))
                .map(str::to_lowercase)
                .collect();
            (i, words)
        })
        .collect();
    for margin in [0.75f32, 1.0, 1.25] {
        let det = Detector {
            dicts: Dicts::from_dicts(texts.iter().map(|(l, t)| Dict::from_freq_list(*l, t, TRAIN)).collect()),
            lm_margin: margin,
            min_lm_len: 4,
        };
        println!("margin {margin}");
        for (i, words) in &tests {
            let lang = Lang::ALL[*i];
            let mut false_switch = 0;
            for w in words {
                let Some(strokes) = layout::encode(lang, w) else { eprintln!("untypeable {w:?}"); continue };
                if det.decide(&strokes, false, *i, &layouts, &[*i]).is_some() {
                    false_switch += 1;
                }
            }
            print!("  {}: wrongly switched {:.1}%", lang.name(), 100.0 * false_switch as f32 / words.len() as f32);
            for (j, other) in Lang::ALL.iter().enumerate() {
                if j == *i || (lang.is_cyrillic() && other.is_cyrillic()) {
                    continue;
                }
                let (mut fixed, mut total) = (0, 0);
                for w in words {
                    let Some(strokes) = layout::encode(lang, w) else { eprintln!("untypeable {w:?}"); continue };
                    total += 1;
                    if det.decide(&strokes, false, j, &layouts, &[*i]).is_some_and(|s| s.word == *w) {
                        fixed += 1;
                    }
                }
                print!(", typed in {}: fixed {:.1}%", other.name(), 100.0 * fixed as f32 / total as f32);
            }
            println!();
        }
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cfg = Config::load()?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("run") => run(cfg),
        Some("check") => {
            check(&cfg, &args[1..]);
            Ok(())
        }
        Some("layouts") => {
            let mut kde = kde::Kde::connect()?;
            let names = kde.layout_names()?;
            let cur = kde.current();
            for (i, (name, lang)) in names.iter().zip(kde.layouts()).enumerate() {
                let mark = if cur == Some(i) { "*" } else { " " };
                println!("{mark} {i}: {name} ({})", lang.map_or("not supported", |l| l.name()));
            }
            Ok(())
        }
        Some("eval") => {
            eval();
            Ok(())
        }
        Some(other) => Err(anyhow!("unknown command {other:?}; use run, layouts, check WORD..., eval")),
    }
}
