//! Frequency dictionaries and character trigram models per language.

use std::collections::{HashMap, HashSet};

use crate::layout::Lang;

const EN_WORDS: &str = include_str!("../data/en_50k.txt");
const RU_WORDS: &str = include_str!("../data/ru_50k.txt");
const UK_WORDS: &str = include_str!("../data/uk_50k.txt");

/// Rarer word forms (5+ letters) from the full lists, see scripts/extra_words.py.
const EN_EXTRA: &str = include_str!("../data/en_extra.txt");
const RU_EXTRA: &str = include_str!("../data/ru_extra.txt");
const UK_EXTRA: &str = include_str!("../data/uk_extra.txt");
/// Shorter entries of the full lists are mostly noise that would block real layout fixes.
const EXTRA_MIN_LEN: usize = 5;

/// One-letter words that are real words; the frequency lists are full of stray letters.
fn singles(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::En => &["a", "i"],
        Lang::Ru => &["а", "в", "и", "к", "о", "с", "у", "я"],
        Lang::Uk => &["а", "в", "з", "і", "й", "о", "у", "я"],
    }
}

/// Contraction suffixes: the English list is tokenized as "don" + "'t".
const EN_SUFFIXES: &[&str] = &["s", "t", "m", "re", "ll", "ve", "d"];

pub struct Dict {
    pub lang: Lang,
    ranks: HashMap<String, u32>,
    /// Known words beyond the frequency list: valid, but never suggested as typo fixes.
    extra: HashSet<&'static str>,
    lm: CharModel,
}

impl Dict {
    pub fn builtin(lang: Lang) -> Dict {
        let (text, extra) = match lang {
            Lang::En => (EN_WORDS, EN_EXTRA),
            Lang::Ru => (RU_WORDS, RU_EXTRA),
            Lang::Uk => (UK_WORDS, UK_EXTRA),
        };
        let mut dict = Dict::from_freq_list(lang, text, usize::MAX);
        dict.extra = extra.lines().filter(|w| w.chars().count() >= EXTRA_MIN_LEN).collect();
        dict
    }

    /// Parses "word count" lines, ordered by frequency, keeping the first `limit` of them.
    pub fn from_freq_list(lang: Lang, text: &str, limit: usize) -> Dict {
        let mut ranks = HashMap::new();
        let mut lm = CharModel::default();
        for (rank, line) in text.lines().take(limit).enumerate() {
            let mut parts = line.split_whitespace();
            let (Some(word), Some(count)) = (parts.next(), parts.next()) else {
                continue;
            };
            let word = word.to_lowercase();
            if !word.chars().all(|c| lang.is_letter(c)) {
                continue;
            }
            let count: f32 = count.parse().unwrap_or(1.0);
            lm.add(&word, count.ln().max(1.0));
            ranks.entry(word).or_insert(rank as u32);
        }
        Dict { lang, ranks, extra: HashSet::new(), lm }
    }

    /// Frequency rank of a lowercase word, if it counts as a known word.
    pub fn rank(&self, word: &str) -> Option<u32> {
        if let Some((head, tail)) = word.split_once(['\'', 'ʼ']) {
            return self.rank_with_apostrophe(head, tail);
        }
        let rank = *self.ranks.get(word)?;
        // Short entries are mostly noise unless frequent.
        let ok = match word.chars().count() {
            0 => false,
            1 => singles(self.lang).contains(&word),
            2 => rank < 3000,
            3 => rank < 30000,
            _ => true,
        };
        ok.then_some(rank)
    }

    fn rank_with_apostrophe(&self, head: &str, tail: &str) -> Option<u32> {
        match self.lang {
            Lang::En if EN_SUFFIXES.contains(&tail) => self.rank(head),
            // Ukrainian lists drop the apostrophe: "п'ять" is stored as "ять".
            Lang::Uk if (1..=3).contains(&head.chars().count()) && tail.chars().count() >= 2 => {
                let joined = format!("{head}{tail}");
                self.ranks.get(&joined).copied().or_else(|| self.rank(tail))
            }
            _ => None,
        }
    }

    /// The word is a real word: either in the frequency list or one of the extra word forms.
    pub fn knows(&self, word: &str) -> bool {
        self.rank(word).is_some() || self.is_extra(word)
    }

    /// Rare but valid word form; ranks after everything in the frequency list.
    pub fn is_extra(&self, word: &str) -> bool {
        self.extra.contains(word)
    }

    /// Average log-probability per character under the trigram model.
    pub fn score(&self, word: &str) -> f32 {
        self.lm.score(word)
    }

    /// Most frequent known word one edit (Damerau) away from `word`.
    pub fn suggest(&self, word: &str) -> Option<(String, u32)> {
        let chars: Vec<char> = word.chars().collect();
        let alphabet: Vec<char> = self.lang.alphabet().chars().collect();
        let mut best: Option<(String, u32)> = None;
        let mut consider = |cand: Vec<char>| {
            let cand: String = cand.into_iter().collect();
            if let Some(r) = self.rank(&cand)
                && best.as_ref().is_none_or(|(_, br)| r < *br)
            {
                best = Some((cand, r));
            }
        };
        for i in 0..chars.len() {
            let mut c = chars.clone();
            c.remove(i);
            consider(c);
            if i + 1 < chars.len() {
                let mut c = chars.clone();
                c.swap(i, i + 1);
                consider(c);
            }
            for &a in &alphabet {
                if a != chars[i] {
                    let mut c = chars.clone();
                    c[i] = a;
                    consider(c);
                }
            }
        }
        for i in 0..=chars.len() {
            for &a in &alphabet {
                let mut c = chars.clone();
                c.insert(i, a);
                consider(c);
            }
        }
        best
    }
}

/// Interpolated character trigram model with word boundary markers.
#[derive(Default)]
struct CharModel {
    tri: HashMap<[char; 3], f32>,
    bi: HashMap<[char; 2], f32>,
    uni: HashMap<char, f32>,
    ctx2: HashMap<[char; 2], f32>,
    ctx1: HashMap<char, f32>,
    total: f32,
}

impl CharModel {
    fn padded(word: &str) -> Vec<char> {
        let mut s = vec!['^', '^'];
        s.extend(word.chars());
        s.push('$');
        s
    }

    fn add(&mut self, word: &str, weight: f32) {
        let s = Self::padded(word);
        for w in s.windows(3) {
            *self.tri.entry([w[0], w[1], w[2]]).or_default() += weight;
            *self.ctx2.entry([w[0], w[1]]).or_default() += weight;
            *self.bi.entry([w[1], w[2]]).or_default() += weight;
            *self.ctx1.entry(w[1]).or_default() += weight;
            *self.uni.entry(w[2]).or_default() += weight;
            self.total += weight;
        }
    }

    fn score(&self, word: &str) -> f32 {
        let s = Self::padded(word);
        let windows = s.windows(3);
        let n = windows.len() as f32;
        let sum: f32 = windows
            .map(|w| {
                let ratio = |num: Option<&f32>, den: Option<&f32>| match den {
                    Some(d) if *d > 0.0 => num.copied().unwrap_or(0.0) / d,
                    _ => 0.0,
                };
                let p3 = ratio(self.tri.get(&[w[0], w[1], w[2]]), self.ctx2.get(&[w[0], w[1]]));
                let p2 = ratio(self.bi.get(&[w[1], w[2]]), self.ctx1.get(&w[1]));
                let p1 = (self.uni.get(&w[2]).copied().unwrap_or(0.0) + 0.5) / (self.total + 20.0);
                (0.6 * p3 + 0.3 * p2 + 0.1 * p1).ln()
            })
            .sum();
        sum / n
    }
}

pub struct Dicts {
    dicts: Vec<Dict>,
}

impl Dicts {
    pub fn builtin() -> Dicts {
        Dicts { dicts: Lang::ALL.iter().map(|&l| Dict::builtin(l)).collect() }
    }

    pub fn from_dicts(dicts: Vec<Dict>) -> Dicts {
        Dicts { dicts }
    }

    pub fn get(&self, lang: Lang) -> &Dict {
        self.dicts.iter().find(|d| d.lang == lang).expect("dictionary for every language")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        let en = Dict::builtin(Lang::En);
        let uk = Dict::builtin(Lang::Uk);
        assert!(en.knows("hello"));
        assert!(en.knows("don't"));
        assert!(!en.knows("b"));
        assert!(uk.knows("привіт"));
        assert!(uk.knows("п'ять"));
        assert!(en.score("hello") > en.score("ghbdtn"));
        assert_eq!(en.suggest("helllo").map(|s| s.0).as_deref(), Some("hello"));

        let ru = Dict::builtin(Lang::Ru);
        assert!(ru.rank("каталоге").is_none() && ru.knows("каталоге"));
        // Short entries of the full list stay unknown.
        assert!(!ru.knows("шт"));
    }
}
