//! Input devices: grabbing physical keyboards, watching pointers, virtual output devices.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, Device, EventType, InputEvent, KeyCode, LedCode, RelativeAxisCode};
use log::{info, warn};

pub const VIRTUAL_KEYBOARD: &str = "layswitch virtual keyboard";
pub const VIRTUAL_POINTER: &str = "layswitch virtual pointer";

pub enum Msg {
    Key(KeyCode, i32),
    /// Pointer events from a grabbed combo device, one SYN frame.
    Pointer(Vec<InputEvent>),
    /// A mouse button or touch: the text cursor may have moved.
    Click,
    Keyboard(PathBuf),
    Tray(crate::tray::Command),
}

/// Button codes live in the key space; keep them off the keyboard device so it is not taken for a mouse.
fn is_button(code: u16) -> bool {
    (0x100..0x160).contains(&code) || (0x2c0..0x2f0).contains(&code)
}

fn is_keyboard(dev: &Device) -> bool {
    let name = dev.name().unwrap_or_default();
    if name.starts_with("layswitch") {
        return false;
    }
    let letters = dev.supported_keys().is_some_and(|k| {
        [KeyCode::KEY_A, KeyCode::KEY_Z, KeyCode::KEY_SPACE, KeyCode::KEY_ENTER].iter().all(|c| k.contains(*c))
    });
    letters && dev.supported_absolute_axes().is_none()
}

fn is_pointer(dev: &Device) -> bool {
    !dev.name().unwrap_or_default().starts_with("layswitch")
        && dev
            .supported_keys()
            .is_some_and(|k| k.contains(KeyCode::BTN_LEFT) || k.contains(KeyCode::BTN_TOUCH))
}

pub fn virtual_keyboard() -> Result<VirtualDevice> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for code in 1..0x2f0u16 {
        if !is_button(code) {
            keys.insert(KeyCode::new(code));
        }
    }
    VirtualDevice::builder()
        .context("open /dev/uinput (see install.sh for permissions)")?
        .name(VIRTUAL_KEYBOARD)
        .with_keys(&keys)?
        .build()
        .context("create virtual keyboard")
}

pub fn virtual_pointer() -> Result<VirtualDevice> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for code in KeyCode::BTN_LEFT.code()..=KeyCode::BTN_TASK.code() {
        keys.insert(KeyCode::new(code));
    }
    let mut rel = AttributeSet::<RelativeAxisCode>::new();
    for axis in [
        RelativeAxisCode::REL_X,
        RelativeAxisCode::REL_Y,
        RelativeAxisCode::REL_WHEEL,
        RelativeAxisCode::REL_HWHEEL,
        RelativeAxisCode::REL_WHEEL_HI_RES,
        RelativeAxisCode::REL_HWHEEL_HI_RES,
    ] {
        rel.insert(axis);
    }
    Ok(VirtualDevice::builder()?.name(VIRTUAL_POINTER).with_keys(&keys)?.with_relative_axes(&rel)?.build()?)
}

/// Waits until no key is held, so we never grab a device in the middle of a keypress.
fn wait_released(dev: &Device) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        match dev.get_key_state() {
            Ok(state) if state.iter().next().is_some() => thread::sleep(Duration::from_millis(20)),
            _ => return,
        }
    }
}

fn read_keyboard(path: PathBuf, mut dev: Device, tx: Sender<Msg>, known: Arc<Mutex<HashSet<PathBuf>>>) {
    wait_released(&dev);
    if let Err(e) = dev.grab() {
        warn!("grab {}: {e}", path.display());
        known.lock().unwrap().remove(&path);
        return;
    }
    info!("grabbed {} ({})", path.display(), dev.name().unwrap_or("?"));
    let _ = tx.send(Msg::Keyboard(path.clone()));
    let mut frame = Vec::new();
    'outer: loop {
        let events = match dev.fetch_events() {
            Ok(events) => events,
            Err(e) => {
                warn!("{} gone: {e}", path.display());
                break;
            }
        };
        for ev in events {
            let msg = match ev.event_type() {
                EventType::KEY if is_button(ev.code()) => {
                    frame.push(ev);
                    None
                }
                EventType::KEY => Some(Msg::Key(KeyCode::new(ev.code()), ev.value())),
                EventType::RELATIVE => {
                    frame.push(ev);
                    None
                }
                EventType::SYNCHRONIZATION if !frame.is_empty() => Some(Msg::Pointer(std::mem::take(&mut frame))),
                _ => None,
            };
            if let Some(msg) = msg
                && tx.send(msg).is_err()
            {
                break 'outer;
            }
        }
    }
    known.lock().unwrap().remove(&path);
}

fn watch_pointer(path: PathBuf, mut dev: Device, tx: Sender<Msg>, known: Arc<Mutex<HashSet<PathBuf>>>) {
    while let Ok(events) = dev.fetch_events() {
        let click = events.into_iter().any(|ev| {
            ev.event_type() == EventType::KEY && ev.value() == 1 && is_button(ev.code())
        });
        if click && tx.send(Msg::Click).is_err() {
            break;
        }
    }
    known.lock().unwrap().remove(&path);
}

fn excluded(dev: &Device, exclude: &[String]) -> bool {
    let name = dev.name().unwrap_or_default();
    exclude.iter().any(|x| name.contains(x.as_str()))
}

/// Scans /dev/input for new devices every couple of seconds.
pub fn spawn_scanner(tx: Sender<Msg>, exclude: Vec<String>) {
    let known: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
    thread::spawn(move || {
        loop {
            for (path, dev) in evdev::enumerate() {
                if known.lock().unwrap().contains(&path) || excluded(&dev, &exclude) {
                    continue;
                }
                let (tx, known2) = (tx.clone(), known.clone());
                if is_keyboard(&dev) {
                    known.lock().unwrap().insert(path.clone());
                    thread::spawn(move || read_keyboard(path, dev, tx, known2));
                } else if is_pointer(&dev) {
                    known.lock().unwrap().insert(path.clone());
                    thread::spawn(move || watch_pointer(path, dev, tx, known2));
                }
            }
            thread::sleep(Duration::from_secs(2));
        }
    });
}

/// Second handle to a grabbed keyboard, used to mirror the Caps Lock LED.
pub struct Leds {
    devices: Vec<Device>,
}

impl Leds {
    pub fn new() -> Leds {
        Leds { devices: Vec::new() }
    }

    /// Adds a keyboard and returns its current Caps Lock state.
    pub fn add(&mut self, path: &Path) -> Option<bool> {
        let dev = Device::open(path).map_err(|e| warn!("open {} for LEDs: {e}", path.display())).ok()?;
        let caps = dev.get_led_state().ok().map(|s| s.contains(LedCode::LED_CAPSL));
        self.devices.push(dev);
        caps
    }

    pub fn set_caps(&mut self, on: bool) {
        let ev = InputEvent::new(EventType::LED.0, LedCode::LED_CAPSL.0, on as i32);
        // Unplugged keyboards fail here; drop them.
        self.devices.retain_mut(|d| d.send_events(&[ev]).is_ok());
    }
}
