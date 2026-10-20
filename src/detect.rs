//! Decides whether a finished word was typed in the wrong layout, or with a typo.

use crate::dict::Dicts;
use crate::layout::{self, Lang, Stroke};

pub struct Detector {
    pub dicts: Dicts,
    /// Required advantage in average log-probability per char for a switch without a dictionary hit.
    pub lm_margin: f32,
    /// Minimum word length for the language-model fallback.
    pub min_lm_len: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Switch {
    /// Index of the layout to switch to.
    pub target: usize,
    /// How many leading strokes to replay as-is in the target layout.
    pub replay: usize,
    /// Trailing punctuation to retype as characters after the replayed strokes.
    pub trailing: Vec<char>,
    /// The word as it will appear (for logs).
    pub word: String,
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// Letters only, apostrophe allowed inside (don't, п'ять).
fn word_shaped(lang: Lang, word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    let apostrophe = |c: char| c == '\'' || c == 'ʼ';
    !chars.is_empty()
        && chars.iter().enumerate().all(|(i, &c)| {
            lang.is_letter(c) || (apostrophe(c) && lang != Lang::Ru && i > 0 && i + 1 < chars.len())
        })
}

impl Detector {
    /// Checks the strokes of a finished word typed while layout `cur` was active.
    /// `layouts` are the configured layouts in KDE order; `recent` lists layout indices,
    /// most recently used first, to break ties between ru and uk.
    pub fn decide(
        &self,
        strokes: &[Stroke],
        caps: bool,
        cur: usize,
        layouts: &[Option<Lang>],
        recent: &[usize],
    ) -> Option<Switch> {
        let cur_lang = (*layouts.get(cur)?)?;
        let cur_chars: Vec<char> = layout::decode(cur_lang, strokes, caps)?.chars().collect();
        let trail = cur_chars.iter().rev().take_while(|c| !cur_lang.is_letter(**c)).count();
        let core = strokes.len() - trail;
        if core == 0 {
            return None;
        }
        let cur_core = lower(&cur_chars[..core].iter().collect::<String>());
        let cur_dict = self.dicts.get(cur_lang);
        if cur_dict.knows(&cur_core) || cur_dict.knows(&lower(&cur_chars.iter().collect::<String>())) {
            return None;
        }
        let cur_score = cur_dict.score(&cur_core);

        let mut options = vec![(strokes.len(), Vec::new())];
        if trail > 0 {
            options.push((core, cur_chars[core..].to_vec()));
        }

        let mut best: Option<(f32, usize, Switch)> = None;
        let mut seen = Vec::new();
        for (idx, lang) in layouts.iter().enumerate() {
            let Some(lang) = *lang else { continue };
            if lang == cur_lang || seen.contains(&lang) {
                continue;
            }
            seen.push(lang);
            let dict = self.dicts.get(lang);
            for (replay, trailing) in &options {
                let alt = lower(&layout::decode(lang, &strokes[..*replay], caps)?);
                if alt == cur_core || !word_shaped(lang, &alt) {
                    continue;
                }
                let score = if let Some(rank) = dict.rank(&alt) {
                    // Any dictionary hit beats any model guess; prefer frequent words.
                    100.0 - (rank as f32 + 1.0).ln()
                } else if dict.is_extra(&alt) {
                    // Rare form, jargon or user word: still a dictionary hit, below every ranked one (ln 50k ≈ 10.8).
                    89.0
                } else if alt.chars().count() >= self.min_lm_len {
                    let gain = dict.score(&alt) - cur_score;
                    if gain <= self.lm_margin {
                        continue;
                    }
                    gain
                } else {
                    continue;
                };
                // Unused layouts rank after recent ones, in KDE order.
                let recency = recent.iter().position(|&r| r == idx).unwrap_or(recent.len() + idx);
                let mut word = layout::decode(lang, &strokes[..*replay], caps)?;
                word.extend(trailing.iter());
                let better = match &best {
                    None => true,
                    // Same text in ru and uk: take the layout used most recently.
                    Some((b, r, sw)) if sw.word == word => recency < *r || (recency == *r && score > *b),
                    Some((b, _, _)) => score > *b,
                };
                if better {
                    best = Some((score, recency, Switch { target: idx, replay: *replay, trailing: trailing.clone(), word }));
                }
            }
        }
        best.map(|(_, _, s)| s)
    }

    /// A dictionary correction for a word typed in `lang`, preserving its capitalization.
    pub fn typo_fix(&self, lang: Lang, word: &str) -> Option<String> {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() < 5 || !word_shaped(lang, word) || chars.iter().all(|c| c.is_uppercase()) {
            return None;
        }
        let dict = self.dicts.get(lang);
        let low = lower(word);
        if dict.knows(&low) {
            return None;
        }
        let (fixed, rank) = dict.suggest(&low)?;
        if rank > 20000 {
            return None;
        }
        let mut out: String = fixed.clone();
        if chars[0].is_uppercase() {
            let mut it = fixed.chars();
            out = it.next().map(|c| c.to_uppercase().chain(it).collect()).unwrap_or_default();
        }
        Some(out)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::LazyLock;

    pub static DETECTOR: LazyLock<Detector> =
        LazyLock::new(|| Detector { dicts: Dicts::builtin(), lm_margin: 1.0, min_lm_len: 4 });

    const LAYOUTS: [Option<Lang>; 3] = [Some(Lang::En), Some(Lang::Ru), Some(Lang::Uk)];

    fn check(typed_in: Lang, text: &str) -> Option<String> {
        let cur = LAYOUTS.iter().position(|l| *l == Some(typed_in)).unwrap();
        let strokes = layout::encode(typed_in, text).unwrap();
        DETECTOR.decide(&strokes, false, cur, &LAYOUTS, &[cur]).map(|s| s.word)
    }

    #[test]
    fn switches_wrong_layout() {
        assert_eq!(check(Lang::En, "ghbdtn").as_deref(), Some("привет"));
        assert_eq!(check(Lang::Ru, "руддщ").as_deref(), Some("hello"));
        assert_eq!(check(Lang::En, "ghbdsn").as_deref(), Some("привіт"));
        assert_eq!(check(Lang::Ru, "привыт").as_deref(), Some("привіт"));
        assert_eq!(check(Lang::En, "ytn?").as_deref(), Some("нет?"));
        assert_eq!(check(Lang::En, "ghbdtn,").as_deref(), Some("привет,"));
        assert_eq!(check(Lang::Ru, "вщтэе").as_deref(), Some("don't"));
        // Rare word form, known only from the extra list.
        assert_eq!(check(Lang::En, "rfnfkjut").as_deref(), Some("каталоге"));
        // Jargon.
        assert_eq!(check(Lang::En, "rjvvbn").as_deref(), Some("коммит"));
    }

    #[test]
    fn keeps_correct_words() {
        for (lang, w) in [
            (Lang::En, "hello"),
            (Lang::En, "don't"),
            (Lang::En, "nginx"),
            (Lang::En, "kubectl"),
            (Lang::Ru, "привет"),
            (Lang::Uk, "привіт"),
            (Lang::En, "a"),
            (Lang::En, "ok"),
        ] {
            assert_eq!(check(lang, w), None, "{w}");
        }
    }

    #[test]
    fn typo() {
        assert_eq!(DETECTOR.typo_fix(Lang::Ru, "Пртвет").as_deref(), Some("Привет"));
        assert_eq!(DETECTOR.typo_fix(Lang::Ru, "привет"), None);
        // Inflected forms missing from the 50k list are not typos.
        for w in ["каталоге", "тестами", "указывается", "консультанту", "Сербия"] {
            assert_eq!(DETECTOR.typo_fix(Lang::Ru, w), None, "{w}");
        }
        assert_eq!(DETECTOR.typo_fix(Lang::Ru, "коммит"), None);
    }
}
