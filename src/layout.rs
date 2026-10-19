//! Keyboard layout tables: evdev key + shift -> character for us / ru / ua.
//!
//! Tables mirror the default xkb variants: `us`, `ru` (winkeys), `ua` (unicode).

use evdev::KeyCode;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    En,
    Ru,
    Uk,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::En, Lang::Ru, Lang::Uk];

    /// Maps an xkb short layout name (as reported by KDE) to a supported language.
    pub fn from_xkb(short: &str) -> Option<Lang> {
        match short {
            "us" | "gb" | "en" => Some(Lang::En),
            "ru" => Some(Lang::Ru),
            "ua" | "uk" => Some(Lang::Uk),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ru => "ru",
            Lang::Uk => "uk",
        }
    }

    pub fn is_cyrillic(self) -> bool {
        self != Lang::En
    }

    fn col(self) -> usize {
        match self {
            Lang::En => 0,
            Lang::Ru => 2,
            Lang::Uk => 4,
        }
    }

    /// Lowercase alphabet of the language (apostrophe is handled separately).
    pub fn alphabet(self) -> &'static str {
        match self {
            Lang::En => "abcdefghijklmnopqrstuvwxyz",
            Lang::Ru => "абвгдеёжзийклмнопрстуфхцчшщъыьэюя",
            Lang::Uk => "абвгґдеєжзиіїйклмнопрстуфхцчшщьюя",
        }
    }

    pub fn is_letter(self, c: char) -> bool {
        let c = c.to_lowercase().next().unwrap_or(c);
        match self {
            Lang::En => c.is_ascii_lowercase(),
            Lang::Ru => ('а'..='я').contains(&c) || c == 'ё',
            Lang::Uk => {
                (('а'..='я').contains(&c) && !matches!(c, 'ы' | 'ъ' | 'э'))
                    || matches!(c, 'і' | 'ї' | 'є' | 'ґ')
            }
        }
    }
}

/// Key -> [en, EN, ru, RU, uk, UK].
const TABLE: &[(KeyCode, [char; 6])] = &[
    (KeyCode::KEY_GRAVE, ['`', '~', 'ё', 'Ё', '\'', 'ʼ']),
    (KeyCode::KEY_1, ['1', '!', '1', '!', '1', '!']),
    (KeyCode::KEY_2, ['2', '@', '2', '"', '2', '"']),
    (KeyCode::KEY_3, ['3', '#', '3', '№', '3', '№']),
    (KeyCode::KEY_4, ['4', '$', '4', ';', '4', ';']),
    (KeyCode::KEY_5, ['5', '%', '5', '%', '5', '%']),
    (KeyCode::KEY_6, ['6', '^', '6', ':', '6', ':']),
    (KeyCode::KEY_7, ['7', '&', '7', '?', '7', '?']),
    (KeyCode::KEY_8, ['8', '*', '8', '*', '8', '*']),
    (KeyCode::KEY_9, ['9', '(', '9', '(', '9', '(']),
    (KeyCode::KEY_0, ['0', ')', '0', ')', '0', ')']),
    (KeyCode::KEY_MINUS, ['-', '_', '-', '_', '-', '_']),
    (KeyCode::KEY_EQUAL, ['=', '+', '=', '+', '=', '+']),
    (KeyCode::KEY_Q, ['q', 'Q', 'й', 'Й', 'й', 'Й']),
    (KeyCode::KEY_W, ['w', 'W', 'ц', 'Ц', 'ц', 'Ц']),
    (KeyCode::KEY_E, ['e', 'E', 'у', 'У', 'у', 'У']),
    (KeyCode::KEY_R, ['r', 'R', 'к', 'К', 'к', 'К']),
    (KeyCode::KEY_T, ['t', 'T', 'е', 'Е', 'е', 'Е']),
    (KeyCode::KEY_Y, ['y', 'Y', 'н', 'Н', 'н', 'Н']),
    (KeyCode::KEY_U, ['u', 'U', 'г', 'Г', 'г', 'Г']),
    (KeyCode::KEY_I, ['i', 'I', 'ш', 'Ш', 'ш', 'Ш']),
    (KeyCode::KEY_O, ['o', 'O', 'щ', 'Щ', 'щ', 'Щ']),
    (KeyCode::KEY_P, ['p', 'P', 'з', 'З', 'з', 'З']),
    (KeyCode::KEY_LEFTBRACE, ['[', '{', 'х', 'Х', 'х', 'Х']),
    (KeyCode::KEY_RIGHTBRACE, [']', '}', 'ъ', 'Ъ', 'ї', 'Ї']),
    (KeyCode::KEY_A, ['a', 'A', 'ф', 'Ф', 'ф', 'Ф']),
    (KeyCode::KEY_S, ['s', 'S', 'ы', 'Ы', 'і', 'І']),
    (KeyCode::KEY_D, ['d', 'D', 'в', 'В', 'в', 'В']),
    (KeyCode::KEY_F, ['f', 'F', 'а', 'А', 'а', 'А']),
    (KeyCode::KEY_G, ['g', 'G', 'п', 'П', 'п', 'П']),
    (KeyCode::KEY_H, ['h', 'H', 'р', 'Р', 'р', 'Р']),
    (KeyCode::KEY_J, ['j', 'J', 'о', 'О', 'о', 'О']),
    (KeyCode::KEY_K, ['k', 'K', 'л', 'Л', 'л', 'Л']),
    (KeyCode::KEY_L, ['l', 'L', 'д', 'Д', 'д', 'Д']),
    (KeyCode::KEY_SEMICOLON, [';', ':', 'ж', 'Ж', 'ж', 'Ж']),
    (KeyCode::KEY_APOSTROPHE, ['\'', '"', 'э', 'Э', 'є', 'Є']),
    (KeyCode::KEY_BACKSLASH, ['\\', '|', '\\', '/', 'ґ', 'Ґ']),
    (KeyCode::KEY_Z, ['z', 'Z', 'я', 'Я', 'я', 'Я']),
    (KeyCode::KEY_X, ['x', 'X', 'ч', 'Ч', 'ч', 'Ч']),
    (KeyCode::KEY_C, ['c', 'C', 'с', 'С', 'с', 'С']),
    (KeyCode::KEY_V, ['v', 'V', 'м', 'М', 'м', 'М']),
    (KeyCode::KEY_B, ['b', 'B', 'и', 'И', 'и', 'И']),
    (KeyCode::KEY_N, ['n', 'N', 'т', 'Т', 'т', 'Т']),
    (KeyCode::KEY_M, ['m', 'M', 'ь', 'Ь', 'ь', 'Ь']),
    (KeyCode::KEY_COMMA, [',', '<', 'б', 'Б', 'б', 'Б']),
    (KeyCode::KEY_DOT, ['.', '>', 'ю', 'Ю', 'ю', 'Ю']),
    (KeyCode::KEY_SLASH, ['/', '?', '.', ',', '.', ',']),
    (KeyCode::KEY_SPACE, [' ', ' ', ' ', ' ', ' ', ' ']),
];

/// A typed key together with the shift state at the moment of pressing.
pub type Stroke = (KeyCode, bool);

/// Character produced by `key` in `lang`. Caps Lock affects letters only, like xkb does.
pub fn char_of(lang: Lang, key: KeyCode, shift: bool, caps: bool) -> Option<char> {
    let (_, chars) = TABLE.iter().find(|(k, _)| *k == key)?;
    let lower = chars[lang.col()];
    let upper = if lower.is_alphabetic() { shift ^ caps } else { shift };
    Some(chars[lang.col() + upper as usize])
}

/// Key and shift state that produce `c` in `lang`.
pub fn key_of(lang: Lang, c: char, caps: bool) -> Option<Stroke> {
    TABLE.iter().find_map(|(k, chars)| {
        let lower = chars[lang.col()];
        let alpha = lower.is_alphabetic();
        if lower == c {
            Some((*k, alpha && caps))
        } else if chars[lang.col() + 1] == c {
            Some((*k, !(alpha && caps)))
        } else {
            None
        }
    })
}

/// Keys that may be part of a word in at least one layout.
pub fn is_word_key(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::KEY_Q
            | KeyCode::KEY_W
            | KeyCode::KEY_E
            | KeyCode::KEY_R
            | KeyCode::KEY_T
            | KeyCode::KEY_Y
            | KeyCode::KEY_U
            | KeyCode::KEY_I
            | KeyCode::KEY_O
            | KeyCode::KEY_P
            | KeyCode::KEY_A
            | KeyCode::KEY_S
            | KeyCode::KEY_D
            | KeyCode::KEY_F
            | KeyCode::KEY_G
            | KeyCode::KEY_H
            | KeyCode::KEY_J
            | KeyCode::KEY_K
            | KeyCode::KEY_L
            | KeyCode::KEY_Z
            | KeyCode::KEY_X
            | KeyCode::KEY_C
            | KeyCode::KEY_V
            | KeyCode::KEY_B
            | KeyCode::KEY_N
            | KeyCode::KEY_M
            | KeyCode::KEY_GRAVE
            | KeyCode::KEY_LEFTBRACE
            | KeyCode::KEY_RIGHTBRACE
            | KeyCode::KEY_SEMICOLON
            | KeyCode::KEY_APOSTROPHE
            | KeyCode::KEY_BACKSLASH
            | KeyCode::KEY_COMMA
            | KeyCode::KEY_DOT
            | KeyCode::KEY_SLASH
    )
}

/// Punctuation that ends a word when produced by a non-word key (digit row, minus).
pub fn is_trigger_punct(c: char) -> bool {
    matches!(c, '!' | '"' | '?' | '(' | ')' | ':' | ';' | '%' | '*' | '-')
}

/// Decodes strokes into text as they appear in `lang`.
pub fn decode(lang: Lang, strokes: &[Stroke], caps: bool) -> Option<String> {
    strokes.iter().map(|&(k, s)| char_of(lang, k, s, caps)).collect()
}

/// Strokes that type `text` in `lang` (with Caps Lock off).
pub fn encode(lang: Lang, text: &str) -> Option<Vec<Stroke>> {
    text.chars().map(|c| key_of(lang, c, false)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_layouts() {
        for lang in Lang::ALL {
            for (key, chars) in TABLE {
                let col = lang.col();
                assert_eq!(char_of(lang, *key, false, false), Some(chars[col]));
                assert_eq!(char_of(lang, *key, true, false), Some(chars[col + 1]));
            }
        }
        assert_eq!(decode(Lang::Ru, &encode(Lang::En, "ghbdtn").unwrap(), false).unwrap(), "привет");
        assert_eq!(decode(Lang::Uk, &encode(Lang::En, "ghbdsn").unwrap(), false).unwrap(), "привіт");
    }

    #[test]
    fn caps_affects_letters_only() {
        assert_eq!(char_of(Lang::En, KeyCode::KEY_A, false, true), Some('A'));
        assert_eq!(char_of(Lang::En, KeyCode::KEY_A, true, true), Some('a'));
        assert_eq!(char_of(Lang::En, KeyCode::KEY_COMMA, false, true), Some(','));
        assert_eq!(char_of(Lang::Ru, KeyCode::KEY_COMMA, false, true), Some('Б'));
        assert_eq!(key_of(Lang::Ru, 'б', true), Some((KeyCode::KEY_COMMA, true)));
    }
}
