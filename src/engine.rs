//! Keystroke state machine: tracks the current word, forwards keys and rewrites text.

use std::collections::HashSet;

use evdev::KeyCode;
use log::{debug, info, warn};

use crate::detect::Detector;
use crate::layout::{self, Lang, Stroke};

/// Everything the engine needs from the outside world.
pub trait System {
    /// Sends a key event to the virtual keyboard.
    fn emit(&mut self, key: KeyCode, value: i32);
    fn current_layout(&mut self) -> Option<usize>;
    fn set_layout(&mut self, idx: usize) -> bool;
    /// Configured layouts in KDE order; `None` for unsupported ones.
    fn layouts(&mut self) -> Vec<Option<Lang>>;
    /// Waits for a layout switch to take effect.
    fn settle(&mut self) {}
    fn notify(&mut self, _text: &str) {}
    fn primary_selection(&mut self) -> Option<String> {
        None
    }
    fn caps_led(&mut self, _on: bool) {}
    fn remember_ignored(&mut self, _word: &str) {}
    /// Auto switching or autocorrection was turned on or off.
    fn state_changed(&mut self, _enabled: bool, _autocorrect: bool) {}
    /// The engine rewrote a word on screen.
    fn corrected(&mut self, _from: &str, _to: &str) {}
}

pub struct Options {
    pub auto_switch: bool,
    pub autocorrect: bool,
    pub hotkey: KeyCode,
}

const MAX_WORD: usize = 40;

/// The last finished word as it is now on screen, for undo / manual conversion.
struct Shown {
    /// Characters to erase to remove the word together with its trigger.
    erase: usize,
    /// Strokes the user originally typed for the word (without the trigger).
    strokes: Vec<Stroke>,
    /// Layout the strokes were typed in.
    layout: usize,
    /// Character the trigger key produced.
    trigger: char,
    /// Text was changed automatically, so the hotkey means "undo".
    auto: bool,
    /// Layout of the text currently on screen.
    shown_layout: usize,
}

pub struct Engine<S: System> {
    pub sys: S,
    detector: Detector,
    opts: Options,
    enabled: bool,
    buffer: Vec<Stroke>,
    /// False after the word got too long or untrackable, until the next word boundary.
    tracking: bool,
    shown: Option<Shown>,
    held: HashSet<KeyCode>,
    swallowed: HashSet<KeyCode>,
    caps: bool,
    recent: Vec<usize>,
    ignored: HashSet<String>,
}

fn is_modifier(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::KEY_LEFTSHIFT
            | KeyCode::KEY_RIGHTSHIFT
            | KeyCode::KEY_LEFTCTRL
            | KeyCode::KEY_RIGHTCTRL
            | KeyCode::KEY_LEFTALT
            | KeyCode::KEY_RIGHTALT
            | KeyCode::KEY_LEFTMETA
            | KeyCode::KEY_RIGHTMETA
    )
}

fn control_key(c: char) -> Option<KeyCode> {
    match c {
        '\n' => Some(KeyCode::KEY_ENTER),
        '\t' => Some(KeyCode::KEY_TAB),
        _ => None,
    }
}

impl<S: System> Engine<S> {
    pub fn new(sys: S, detector: Detector, opts: Options, ignored: HashSet<String>) -> Self {
        Engine {
            sys,
            detector,
            opts,
            enabled: true,
            buffer: Vec::new(),
            tracking: true,
            shown: None,
            held: HashSet::new(),
            swallowed: HashSet::new(),
            caps: false,
            recent: Vec::new(),
            ignored,
        }
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        info!("auto switching {}", if on { "enabled" } else { "disabled" });
        self.sys.state_changed(self.enabled, self.opts.autocorrect);
    }

    pub fn set_autocorrect(&mut self, on: bool) {
        self.opts.autocorrect = on;
        info!("autocorrect {}", if on { "enabled" } else { "disabled" });
        self.sys.state_changed(self.enabled, self.opts.autocorrect);
    }

    pub fn set_caps(&mut self, on: bool) {
        self.caps = on;
    }

    fn shift(&self) -> bool {
        self.held.contains(&KeyCode::KEY_LEFTSHIFT) || self.held.contains(&KeyCode::KEY_RIGHTSHIFT)
    }

    fn ctrl(&self) -> bool {
        self.held.contains(&KeyCode::KEY_LEFTCTRL) || self.held.contains(&KeyCode::KEY_RIGHTCTRL)
    }

    fn chord(&self) -> bool {
        self.ctrl()
            || [KeyCode::KEY_LEFTALT, KeyCode::KEY_RIGHTALT, KeyCode::KEY_LEFTMETA, KeyCode::KEY_RIGHTMETA]
                .iter()
                .any(|k| self.held.contains(k))
    }

    /// Forgets the current word, e.g. after a click moved the cursor.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.tracking = true;
        self.shown = None;
    }

    fn forward(&mut self, key: KeyCode, value: i32) {
        if is_modifier(key) {
            if value == 0 {
                self.held.remove(&key);
            } else {
                self.held.insert(key);
            }
        }
        self.sys.emit(key, value);
    }

    fn tap(&mut self, key: KeyCode) {
        self.sys.emit(key, 1);
        self.sys.emit(key, 0);
    }

    /// Types a key with the given shift state regardless of which shift keys are held.
    fn stroke(&mut self, (key, shift): Stroke) {
        let held: Vec<KeyCode> = [KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_RIGHTSHIFT]
            .into_iter()
            .filter(|k| self.held.contains(k))
            .collect();
        match (shift, held.is_empty()) {
            (true, true) => {
                self.sys.emit(KeyCode::KEY_LEFTSHIFT, 1);
                self.tap(key);
                self.sys.emit(KeyCode::KEY_LEFTSHIFT, 0);
            }
            (false, false) => {
                held.iter().for_each(|&k| self.sys.emit(k, 0));
                self.tap(key);
                held.iter().for_each(|&k| self.sys.emit(k, 1));
            }
            _ => self.tap(key),
        }
    }

    fn type_char(&mut self, lang: Lang, c: char) {
        if let Some(s) = layout::key_of(lang, c, self.caps) {
            self.stroke(s);
        } else if let Some(k) = control_key(c) {
            self.tap(k);
        } else {
            warn!("cannot type {c:?} in {}", lang.name());
        }
    }

    fn erase(&mut self, n: usize) {
        for _ in 0..n {
            self.tap(KeyCode::KEY_BACKSPACE);
        }
    }

    fn switch_to(&mut self, idx: usize) -> bool {
        if !self.sys.set_layout(idx) {
            warn!("failed to switch layout to #{idx}");
            return false;
        }
        self.note_layout(idx);
        self.sys.settle();
        true
    }

    fn note_layout(&mut self, idx: usize) {
        self.recent.retain(|&r| r != idx);
        self.recent.insert(0, idx);
    }

    fn lang_at(layouts: &[Option<Lang>], idx: usize) -> Option<Lang> {
        layouts.get(idx).copied().flatten()
    }

    fn next_layout(layouts: &[Option<Lang>], cur: usize) -> Option<usize> {
        (1..=layouts.len()).map(|i| (cur + i) % layouts.len()).find(|&i| layouts[i].is_some() && i != cur)
    }

    pub fn handle_key(&mut self, key: KeyCode, value: i32) {
        match value {
            0 => {
                if !self.swallowed.remove(&key) {
                    self.forward(key, 0);
                }
            }
            1 => self.press(key),
            _ => {
                if self.swallowed.contains(&key) {
                    return;
                }
                // The compositor repeats keys itself, so the buffer no longer matches the screen.
                if !is_modifier(key) {
                    self.buffer.clear();
                    self.tracking = false;
                    self.shown = None;
                }
                self.forward(key, value);
            }
        }
    }

    fn press(&mut self, key: KeyCode) {
        if is_modifier(key) {
            self.forward(key, 1);
            return;
        }
        if key == self.opts.hotkey {
            self.swallowed.insert(key);
            if self.ctrl() {
                self.convert_selection();
            } else if self.shift() {
                self.set_enabled(!self.enabled);
                self.sys.notify(if self.enabled { "Автопереключение включено" } else { "Автопереключение выключено" });
            } else {
                self.convert_last();
            }
            return;
        }
        if key == KeyCode::KEY_CAPSLOCK {
            self.caps = !self.caps;
            self.sys.caps_led(self.caps);
            self.forward(key, 1);
            return;
        }
        if self.chord() {
            self.reset();
            self.forward(key, 1);
            return;
        }
        if layout::is_word_key(key) {
            self.shown = None;
            if self.tracking {
                if self.buffer.len() < MAX_WORD {
                    self.buffer.push((key, self.shift()));
                } else {
                    self.buffer.clear();
                    self.tracking = false;
                }
            }
            self.forward(key, 1);
            return;
        }
        if key == KeyCode::KEY_BACKSPACE {
            if self.buffer.pop().is_none() {
                self.shown = None;
            }
            self.forward(key, 1);
            return;
        }
        let maybe_trigger = key == KeyCode::KEY_SPACE
            || (layout::char_of(Lang::En, key, false, false).is_some() && !self.buffer.is_empty());
        if maybe_trigger && self.tracking && !self.buffer.is_empty() {
            self.on_trigger(key);
            return;
        }
        self.reset();
        self.forward(key, 1);
    }

    fn on_trigger(&mut self, key: KeyCode) {
        let layouts = self.sys.layouts();
        let Some(cur) = self.sys.current_layout() else {
            self.reset();
            self.forward(key, 1);
            return;
        };
        if self.recent.first() != Some(&cur) {
            self.note_layout(cur);
        }
        let Some(cur_lang) = Self::lang_at(&layouts, cur) else {
            self.reset();
            self.forward(key, 1);
            return;
        };
        let shift = self.shift();
        let trigger = layout::char_of(cur_lang, key, shift, self.caps).unwrap_or(' ');
        if key != KeyCode::KEY_SPACE && !layout::is_trigger_punct(trigger) {
            self.reset();
            self.forward(key, 1);
            return;
        }
        let strokes = std::mem::take(&mut self.buffer);
        let typed = layout::decode(cur_lang, &strokes, self.caps).unwrap_or_default();
        let ignored = self.ignored.contains(&typed.to_lowercase());

        if self.enabled && self.opts.auto_switch && !ignored
            && let Some(sw) = self.detector.decide(&strokes, self.caps, cur, &layouts, &self.recent)
        {
            let target_lang = Self::lang_at(&layouts, sw.target).expect("decide returns known layouts");
            self.erase(strokes.len());
            if self.switch_to(sw.target) {
                info!("{typed:?} -> {:?} ({})", sw.word, target_lang.name());
                self.sys.corrected(&typed, &sw.word);
                for &s in &strokes[..sw.replay] {
                    self.stroke(s);
                }
                for &c in &sw.trailing {
                    self.type_char(target_lang, c);
                }
                self.emit_trigger(key, trigger, target_lang);
                self.shown = Some(Shown {
                    erase: sw.replay + sw.trailing.len() + 1,
                    strokes,
                    layout: cur,
                    trigger,
                    auto: true,
                    shown_layout: sw.target,
                });
            } else {
                // Put the original text back.
                for &s in &strokes {
                    self.stroke(s);
                }
                self.forward(key, 1);
            }
            return;
        }

        if self.enabled && self.opts.autocorrect && !ignored {
            let trail = typed.chars().rev().take_while(|c| !cur_lang.is_letter(*c)).count();
            let core: String = typed.chars().take(strokes.len() - trail).collect();
            if let Some(fixed) = self.detector.typo_fix(cur_lang, &core) {
                info!("typo {core:?} -> {fixed:?}");
                self.sys.corrected(&core, &fixed);
                self.erase(strokes.len());
                for c in fixed.chars() {
                    self.type_char(cur_lang, c);
                }
                for &s in &strokes[strokes.len() - trail..] {
                    self.stroke(s);
                }
                self.forward(key, 1);
                self.shown = Some(Shown {
                    erase: fixed.chars().count() + trail + 1,
                    strokes,
                    layout: cur,
                    trigger,
                    auto: true,
                    shown_layout: cur,
                });
                return;
            }
        }

        debug!("keep {typed:?}");
        self.forward(key, 1);
        self.shown = Some(Shown {
            erase: strokes.len() + 1,
            strokes,
            layout: cur,
            trigger,
            auto: false,
            shown_layout: cur,
        });
    }

    /// Re-emits the word-ending key; non-space triggers are retyped as the same character.
    fn emit_trigger(&mut self, key: KeyCode, trigger: char, lang: Lang) {
        if key == KeyCode::KEY_SPACE {
            self.forward(key, 1);
        } else {
            self.swallowed.insert(key);
            self.type_char(lang, trigger);
        }
    }

    /// Hotkey: undo the last automatic change, or convert the current / last word.
    fn convert_last(&mut self) {
        let layouts = self.sys.layouts();
        let Some(cur) = self.sys.current_layout() else { return };

        if !self.buffer.is_empty() {
            let Some(next) = Self::next_layout(&layouts, cur) else { return };
            let strokes = self.buffer.clone();
            self.erase(strokes.len());
            self.switch_to(next);
            for s in strokes {
                self.stroke(s);
            }
            return;
        }

        let Some(shown) = self.shown.take() else {
            // Nothing to convert: just switch layout, like the classic Pause behaviour.
            if let Some(next) = Self::next_layout(&layouts, cur) {
                self.switch_to(next);
            }
            return;
        };
        let target = if shown.auto {
            if let Some(lang) = Self::lang_at(&layouts, shown.layout) {
                let original = layout::decode(lang, &shown.strokes, self.caps).unwrap_or_default().to_lowercase();
                info!("undo, will not touch {original:?} again");
                self.sys.remember_ignored(&original);
                self.ignored.insert(original);
            }
            shown.layout
        } else {
            match Self::next_layout(&layouts, shown.shown_layout) {
                Some(n) => n,
                None => return,
            }
        };
        let Some(target_lang) = Self::lang_at(&layouts, target) else { return };
        self.erase(shown.erase);
        self.switch_to(target);
        for &s in &shown.strokes {
            self.stroke(s);
        }
        self.type_char(target_lang, shown.trigger);
        self.shown = Some(Shown {
            erase: shown.strokes.len() + 1,
            auto: false,
            shown_layout: target,
            ..shown
        });
    }

    /// Ctrl+hotkey: retype the selected text in the other script.
    fn convert_selection(&mut self) {
        self.reset();
        let Some(text) = self.sys.primary_selection().filter(|t| !t.is_empty()) else {
            self.sys.notify("Нет выделенного текста (нужен wl-clipboard)");
            return;
        };
        let layouts = self.sys.layouts();
        let Some(cur) = self.sys.current_layout() else { return };
        let latin = text.chars().filter(|c| c.is_ascii_alphabetic()).count();
        let cyrillic = text.chars().filter(|c| Lang::Ru.is_letter(*c) || Lang::Uk.is_letter(*c)).count();
        let to_cyrillic = latin >= cyrillic;
        let pick = |want_cyr: bool| -> Option<usize> {
            let ok = |i: usize| Self::lang_at(&layouts, i).is_some_and(|l| l.is_cyrillic() == want_cyr);
            if ok(cur) {
                return Some(cur);
            }
            self.recent.iter().copied().find(|&i| ok(i)).or_else(|| (0..layouts.len()).find(|&i| ok(i)))
        };
        let Some(target) = pick(to_cyrillic) else { return };
        let target_lang = Self::lang_at(&layouts, target).expect("picked a known layout");
        let source: Vec<Lang> = if to_cyrillic {
            vec![Lang::En]
        } else {
            let mut v = vec![Lang::Ru, Lang::Uk];
            if Self::lang_at(&layouts, cur) == Some(Lang::Uk) {
                v.reverse();
            }
            v
        };
        let converted: String = text
            .chars()
            .map(|c| {
                source
                    .iter()
                    .find_map(|&l| layout::key_of(l, c, false))
                    .and_then(|(k, s)| layout::char_of(target_lang, k, s, false))
                    .unwrap_or(c)
            })
            .collect();
        info!("selection {text:?} -> {converted:?}");
        self.switch_to(target);
        for c in converted.chars() {
            self.type_char(target_lang, c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::tests::DETECTOR;
    use crate::dict::Dicts;

    /// A text field that interprets virtual key events with the active layout.
    struct Field {
        text: String,
        layout: usize,
        layouts: Vec<Option<Lang>>,
        shift: u8,
        caps: bool,
        ignored: Vec<String>,
    }

    impl System for Field {
        fn emit(&mut self, key: KeyCode, value: i32) {
            match (key, value) {
                (KeyCode::KEY_LEFTSHIFT | KeyCode::KEY_RIGHTSHIFT, 1) => self.shift += 1,
                (KeyCode::KEY_LEFTSHIFT | KeyCode::KEY_RIGHTSHIFT, 0) => self.shift -= 1,
                (KeyCode::KEY_CAPSLOCK, 1) => self.caps = !self.caps,
                (KeyCode::KEY_BACKSPACE, 1) => {
                    self.text.pop();
                }
                (_, 1) if !is_modifier(key) => {
                    let lang = self.layouts[self.layout].unwrap();
                    if let Some(c) = layout::char_of(lang, key, self.shift > 0, self.caps) {
                        self.text.push(c);
                    }
                }
                _ => {}
            }
        }
        fn current_layout(&mut self) -> Option<usize> {
            Some(self.layout)
        }
        fn set_layout(&mut self, idx: usize) -> bool {
            self.layout = idx;
            true
        }
        fn layouts(&mut self) -> Vec<Option<Lang>> {
            self.layouts.clone()
        }
        fn remember_ignored(&mut self, word: &str) {
            self.ignored.push(word.to_string());
        }
    }

    fn engine(layout: usize) -> Engine<Field> {
        let field = Field {
            text: String::new(),
            layout,
            layouts: vec![Some(Lang::En), Some(Lang::Ru), Some(Lang::Uk)],
            shift: 0,
            caps: false,
            ignored: vec![],
        };
        let detector = Detector {
            dicts: Dicts::builtin(),
            lm_margin: DETECTOR.lm_margin,
            min_lm_len: DETECTOR.min_lm_len,
        };
        Engine::new(
            field,
            detector,
            Options { auto_switch: true, autocorrect: true, hotkey: KeyCode::KEY_PAUSE },
            HashSet::new(),
        )
    }

    fn press(e: &mut Engine<Field>, key: KeyCode) {
        e.handle_key(key, 1);
        e.handle_key(key, 0);
    }

    /// Types `text` with the physical keys that produce it in `lang`.
    fn type_as(e: &mut Engine<Field>, lang: Lang, text: &str) {
        for c in text.chars() {
            let (key, shift) = layout::key_of(lang, c, false).unwrap();
            if shift {
                e.handle_key(KeyCode::KEY_LEFTSHIFT, 1);
            }
            press(e, key);
            if shift {
                e.handle_key(KeyCode::KEY_LEFTSHIFT, 0);
            }
        }
    }

    #[test]
    fn end_to_end() {
        let mut e = engine(0);
        type_as(&mut e, Lang::En, "ghbdtn ");
        assert_eq!(e.sys.text, "привет ");
        assert_eq!(e.sys.layout, 1);
        // Now in ru: keep typing Russian.
        type_as(&mut e, Lang::Ru, "мир ");
        assert_eq!(e.sys.text, "привет мир ");
        // English typed in ru layout.
        type_as(&mut e, Lang::Ru, "руддщ ");
        assert_eq!(e.sys.text, "привет мир hello ");
        assert_eq!(e.sys.layout, 0);

        let mut e = engine(0);
        type_as(&mut e, Lang::En, "Ghbdtn, ");
        assert_eq!(e.sys.text, "Привет, ");
        let mut e = engine(0);
        type_as(&mut e, Lang::En, "ytn? ");
        assert_eq!(e.sys.text, "нет? ");

        // Ukrainian letters typed in ru layout.
        let mut e = engine(1);
        type_as(&mut e, Lang::Ru, "привыт ");
        assert_eq!(e.sys.text, "привіт ");
        assert_eq!(e.sys.layout, 2);

        // Non-space trigger: '?' typed with shift+7 in ru.
        let mut e = engine(1);
        type_as(&mut e, Lang::Ru, "руддщ?");
        assert_eq!(e.sys.text, "hello?");
    }

    #[test]
    fn undo_and_manual() {
        let mut e = engine(0);
        type_as(&mut e, Lang::En, "ghbdtn ");
        press(&mut e, KeyCode::KEY_PAUSE);
        assert_eq!(e.sys.text, "ghbdtn ");
        assert_eq!(e.sys.layout, 0);
        assert_eq!(e.sys.ignored, vec!["ghbdtn".to_string()]);
        type_as(&mut e, Lang::En, "ghbdtn ");
        assert_eq!(e.sys.text, "ghbdtn ghbdtn ");

        // Hotkey in the middle of a word converts what is typed so far.
        let mut e = engine(0);
        type_as(&mut e, Lang::En, "ghbd");
        press(&mut e, KeyCode::KEY_PAUSE);
        assert_eq!(e.sys.text, "прив");
        type_as(&mut e, Lang::Ru, "ет ");
        assert_eq!(e.sys.text, "привет ");

        // Hotkey after a kept word converts it.
        let mut e = engine(0);
        type_as(&mut e, Lang::En, "hello ");
        press(&mut e, KeyCode::KEY_PAUSE);
        assert_eq!(e.sys.text, "руддщ ");
    }

    #[test]
    fn chords_and_typos() {
        let mut e = engine(0);
        type_as(&mut e, Lang::En, "ghbdtn");
        e.handle_key(KeyCode::KEY_LEFTCTRL, 1);
        press(&mut e, KeyCode::KEY_X);
        e.handle_key(KeyCode::KEY_LEFTCTRL, 0);
        type_as(&mut e, Lang::En, " ");
        // The fake field types Ctrl+X as "x"; what matters is that the chord dropped the word.
        assert_eq!(e.sys.text, "ghbdtnx ");

        let mut e = engine(1);
        type_as(&mut e, Lang::Ru, "пртвет ");
        assert_eq!(e.sys.text, "привет ");
        press(&mut e, KeyCode::KEY_PAUSE);
        assert_eq!(e.sys.text, "пртвет ");
    }
}
