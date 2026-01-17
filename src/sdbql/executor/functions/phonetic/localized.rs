//! Language-specific Soundex variants.
//!
//! These phonetic algorithms are optimized for specific languages:
//! - French (soundex_fr)
//! - Spanish (soundex_es)
//! - Italian (soundex_it)
//! - Portuguese (soundex_pt)
//! - Dutch (soundex_nl)
//! - Greek (soundex_el)
//! - Japanese (soundex_ja)

// =============================================================================
// French Soundex
// =============================================================================

/// French Soundex - optimized for French names
pub fn soundex_fr(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let s = s
        .to_uppercase()
        .replace('É', "E")
        .replace('È', "E")
        .replace('Ê', "E")
        .replace('Ë', "E")
        .replace('À', "A")
        .replace('Â', "A")
        .replace('Ä', "A")
        .replace('Î', "I")
        .replace('Ï', "I")
        .replace('Ô', "O")
        .replace('Ö', "O")
        .replace('Ù', "U")
        .replace('Û', "U")
        .replace('Ü', "U")
        .replace('Ç', "S")
        .replace('Œ', "OE")
        .replace('Æ', "AE");

    let chars: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if chars.is_empty() {
        return String::new();
    }

    let word: String = chars.iter().collect();
    let mut processed = word.clone();

    if processed.len() > 2 {
        let last_char = processed.chars().last().unwrap();
        if matches!(last_char, 'T' | 'D' | 'S' | 'X' | 'Z' | 'P') {
            processed.pop();
        }
    }

    processed = processed
        .replace("EAU", "O")
        .replace("AU", "O")
        .replace("OU", "U")
        .replace("OI", "WA")
        .replace("CH", "S")
        .replace("SCH", "S")
        .replace("SH", "S")
        .replace("GN", "N")
        .replace("PH", "F")
        .replace("QU", "K")
        .replace("CK", "K")
        .replace("CC", "K")
        .replace("GU", "G")
        .replace("GA", "KA")
        .replace("GO", "KO")
        .replace("GY", "JI")
        .replace("CE", "SE")
        .replace("CI", "SI")
        .replace("CY", "SY")
        .replace('C', "K")
        .replace("AN", "1")
        .replace("AM", "1")
        .replace("EN", "1")
        .replace("EM", "1")
        .replace("IN", "2")
        .replace("IM", "2")
        .replace("AIN", "2")
        .replace("EIN", "2")
        .replace("ON", "3")
        .replace("OM", "3")
        .replace("UN", "4")
        .replace("UM", "4");

    let first_char = chars[0];
    let mut result = String::from(first_char);
    let processed_chars: Vec<char> = processed.chars().collect();
    let mut last_code: Option<char> = soundex_fr_digit(first_char);

    for &c in processed_chars.iter().skip(1) {
        if result.len() >= 4 {
            break;
        }
        let code = soundex_fr_digit(c);
        if let Some(d) = code {
            if Some(d) != last_code {
                result.push(d);
            }
            last_code = Some(d);
        } else {
            last_code = None;
        }
    }

    while result.len() < 4 {
        result.push('0');
    }
    result
}

fn soundex_fr_digit(c: char) -> Option<char> {
    match c {
        'B' | 'P' => Some('1'),
        'C' | 'K' | 'Q' => Some('2'),
        'D' | 'T' => Some('3'),
        'L' => Some('4'),
        'M' | 'N' => Some('5'),
        'R' => Some('6'),
        'G' | 'J' => Some('7'),
        'S' | 'X' | 'Z' => Some('8'),
        'F' | 'V' => Some('9'),
        '1' | '2' | '3' | '4' => Some(c),
        _ => None,
    }
}

// =============================================================================
// Spanish Soundex
// =============================================================================

/// Spanish Soundex - optimized for Spanish/Castilian names
pub fn soundex_es(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let s = s
        .to_uppercase()
        .replace('Á', "A")
        .replace('É', "E")
        .replace('Í', "I")
        .replace('Ó', "O")
        .replace('Ú', "U")
        .replace('Ü', "U")
        .replace('Ñ', "NY");

    let chars: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if chars.is_empty() {
        return String::new();
    }

    let word: String = chars.iter().collect();
    let processed = word
        .replace("CH", "X")
        .replace("LL", "Y")
        .replace("RR", "R")
        .replace("QU", "K")
        .replace("GU", "G")
        .replace("GÜ", "GW")
        .replace("CE", "SE")
        .replace("CI", "SI")
        .replace("CY", "SY")
        .replace("ZA", "SA")
        .replace("ZE", "SE")
        .replace("ZI", "SI")
        .replace("ZO", "SO")
        .replace("ZU", "SU")
        .replace('C', "K")
        .replace('B', "V")
        .replace('H', "");

    let processed_chars: Vec<char> = processed.chars().collect();
    if processed_chars.is_empty() {
        return String::new();
    }

    let first_char = processed_chars[0];
    let mut result = String::from(first_char);
    let mut last_code: Option<char> = soundex_es_digit(first_char);

    for &c in processed_chars.iter().skip(1) {
        if result.len() >= 4 {
            break;
        }
        let code = soundex_es_digit(c);
        if let Some(d) = code {
            if Some(d) != last_code {
                result.push(d);
            }
            last_code = Some(d);
        } else {
            last_code = None;
        }
    }

    while result.len() < 4 {
        result.push('0');
    }
    result
}

fn soundex_es_digit(c: char) -> Option<char> {
    match c {
        'B' | 'V' | 'W' => Some('1'),
        'K' | 'Q' => Some('2'),
        'D' | 'T' => Some('3'),
        'L' => Some('4'),
        'M' | 'N' => Some('5'),
        'R' => Some('6'),
        'G' | 'J' | 'X' => Some('7'),
        'S' | 'Z' => Some('8'),
        'F' | 'P' => Some('9'),
        'Y' => Some('0'),
        _ => None,
    }
}

// =============================================================================
// Italian Soundex
// =============================================================================

/// Italian Soundex - optimized for Italian names
pub fn soundex_it(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let s = s
        .to_uppercase()
        .replace('À', "A")
        .replace('È', "E")
        .replace('É', "E")
        .replace('Ì', "I")
        .replace('Ò', "O")
        .replace('Ù', "U");

    let chars: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if chars.is_empty() {
        return String::new();
    }

    let word: String = chars.iter().collect();
    let mut processed = word
        .replace("GLI", "LI")
        .replace("GN", "N")
        .replace("SCE", "SE")
        .replace("SCI", "SI")
        .replace("CHE", "KE")
        .replace("CHI", "KI")
        .replace("GHE", "GE")
        .replace("GHI", "GI")
        .replace("GE", "JE")
        .replace("GI", "JI")
        .replace("QU", "KU")
        .replace('H', "");

    let mut prev_char = ' ';
    let deduped: String = processed
        .chars()
        .filter(|&c| {
            let keep = c != prev_char || "AEIOU".contains(c);
            prev_char = c;
            keep
        })
        .collect();
    processed = deduped;

    let first_char = chars[0];
    let mut result = String::from(first_char);
    let processed_chars: Vec<char> = processed.chars().collect();
    let mut last_code: Option<char> = soundex_it_digit(first_char);

    for &c in processed_chars.iter().skip(1) {
        if result.len() >= 4 {
            break;
        }
        let code = soundex_it_digit(c);
        if let Some(d) = code {
            if Some(d) != last_code {
                result.push(d);
            }
            last_code = Some(d);
        } else {
            last_code = None;
        }
    }

    while result.len() < 4 {
        result.push('0');
    }
    result
}

fn soundex_it_digit(c: char) -> Option<char> {
    match c {
        'B' | 'P' => Some('1'),
        'C' | 'K' | 'Q' => Some('2'),
        'D' | 'T' => Some('3'),
        'L' => Some('4'),
        'M' | 'N' => Some('5'),
        'R' => Some('6'),
        'G' | 'J' => Some('7'),
        'S' | 'X' | 'Z' => Some('8'),
        'F' | 'V' => Some('9'),
        _ => None,
    }
}

// =============================================================================
// Portuguese Soundex
// =============================================================================

/// Portuguese Soundex - optimized for Portuguese names
pub fn soundex_pt(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let s = s
        .to_uppercase()
        .replace('Á', "A")
        .replace('À', "A")
        .replace('Â', "A")
        .replace('Ã', "AN")
        .replace('É', "E")
        .replace('Ê', "E")
        .replace('Í', "I")
        .replace('Ó', "O")
        .replace('Ô', "O")
        .replace('Õ', "ON")
        .replace('Ú', "U")
        .replace('Ü', "U")
        .replace('Ç', "S");

    let chars: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if chars.is_empty() {
        return String::new();
    }

    let word: String = chars.iter().collect();
    let mut processed = word
        .replace("NH", "N")
        .replace("LH", "L")
        .replace("CH", "X")
        .replace("RR", "R")
        .replace("SS", "S")
        .replace("QU", "K")
        .replace("GU", "G")
        .replace("CE", "SE")
        .replace("CI", "SI")
        .replace("GE", "JE")
        .replace("GI", "JI");

    if processed.starts_with('H') {
        processed = processed[1..].to_string();
    }

    let first_char = chars[0];
    let mut result = String::from(first_char);
    let processed_chars: Vec<char> = processed.chars().collect();
    let mut last_code: Option<char> = soundex_pt_digit(first_char);

    for &c in processed_chars.iter().skip(1) {
        if result.len() >= 4 {
            break;
        }
        let code = soundex_pt_digit(c);
        if let Some(d) = code {
            if Some(d) != last_code {
                result.push(d);
            }
            last_code = Some(d);
        } else {
            last_code = None;
        }
    }

    while result.len() < 4 {
        result.push('0');
    }
    result
}

fn soundex_pt_digit(c: char) -> Option<char> {
    match c {
        'B' | 'P' => Some('1'),
        'C' | 'K' | 'Q' => Some('2'),
        'D' | 'T' => Some('3'),
        'L' => Some('4'),
        'M' | 'N' => Some('5'),
        'R' => Some('6'),
        'G' | 'J' | 'X' => Some('7'),
        'S' | 'Z' => Some('8'),
        'F' | 'V' => Some('9'),
        _ => None,
    }
}

// =============================================================================
// Dutch Soundex
// =============================================================================

/// Dutch Soundex - optimized for Dutch names
pub fn soundex_nl(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let s = s
        .to_uppercase()
        .replace("IJ", "Y")
        .replace('Ë', "E")
        .replace('Ï', "I")
        .replace('É', "E");

    let chars: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if chars.is_empty() {
        return String::new();
    }

    let word: String = chars.iter().collect();
    let processed = word
        .replace("SCH", "S")
        .replace("CH", "G")
        .replace("PH", "F")
        .replace("QU", "KW")
        .replace("TH", "T")
        .replace("DT", "T")
        .replace("AA", "A")
        .replace("EE", "E")
        .replace("OO", "O")
        .replace("UU", "U")
        .replace('W', "V");

    let processed_chars: Vec<char> = processed.chars().collect();
    if processed_chars.is_empty() {
        return String::new();
    }

    let first_char = processed_chars[0];
    let mut result = String::from(first_char);
    let mut last_code: Option<char> = soundex_nl_digit(first_char);

    for &c in processed_chars.iter().skip(1) {
        if result.len() >= 4 {
            break;
        }
        let code = soundex_nl_digit(c);
        if let Some(d) = code {
            if Some(d) != last_code {
                result.push(d);
            }
            last_code = Some(d);
        } else {
            last_code = None;
        }
    }

    while result.len() < 4 {
        result.push('0');
    }
    result
}

fn soundex_nl_digit(c: char) -> Option<char> {
    match c {
        'B' | 'P' => Some('1'),
        'C' | 'K' | 'Q' => Some('2'),
        'D' | 'T' => Some('3'),
        'L' => Some('4'),
        'M' | 'N' => Some('5'),
        'R' => Some('6'),
        'G' | 'J' => Some('7'),
        'S' | 'X' | 'Z' => Some('8'),
        'F' | 'V' => Some('9'),
        _ => None,
    }
}

// =============================================================================
// Greek Soundex
// =============================================================================

/// Greek Soundex - optimized for Greek names (with transliteration)
pub fn soundex_el(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let mut transliterated = s.to_uppercase();

    transliterated = transliterated
        .replace("ΟΥ", "U")
        .replace("ΑΥ", "AV")
        .replace("ΕΥ", "EV")
        .replace("ΑΙ", "E")
        .replace("ΕΙ", "I")
        .replace("ΟΙ", "I")
        .replace("ΥΙ", "I")
        .replace("ΜΠ", "B")
        .replace("ΝΤ", "D")
        .replace("ΓΚ", "G")
        .replace("ΓΓ", "NG")
        .replace("ΤΣ", "TS")
        .replace("ΤΖ", "DZ");

    transliterated = transliterated
        .replace('Α', "A")
        .replace('Ά', "A")
        .replace('Β', "V")
        .replace('Γ', "G")
        .replace('Δ', "D")
        .replace('Ε', "E")
        .replace('Έ', "E")
        .replace('Ζ', "Z")
        .replace('Η', "I")
        .replace('Ή', "I")
        .replace('Θ', "TH")
        .replace('Ι', "I")
        .replace('Ί', "I")
        .replace('Ϊ', "I")
        .replace('Κ', "K")
        .replace('Λ', "L")
        .replace('Μ', "M")
        .replace('Ν', "N")
        .replace('Ξ', "KS")
        .replace('Ο', "O")
        .replace('Ό', "O")
        .replace('Π', "P")
        .replace('Ρ', "R")
        .replace('Σ', "S")
        .replace('Τ', "T")
        .replace('Υ', "I")
        .replace('Ύ', "I")
        .replace('Ϋ', "I")
        .replace('Φ', "F")
        .replace('Χ', "CH")
        .replace('Ψ', "PS")
        .replace('Ω', "O")
        .replace('Ώ', "O");

    let chars: Vec<char> = transliterated
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect();
    if chars.is_empty() {
        return String::new();
    }

    let word: String = chars.iter().collect();
    let mut processed = word
        .replace("TH", "T")
        .replace("CH", "K")
        .replace("PS", "S")
        .replace("KS", "S");

    let mut prev_char = ' ';
    let deduped: String = processed
        .chars()
        .filter(|&c| {
            let keep = c != prev_char || "AEIOU".contains(c);
            prev_char = c;
            keep
        })
        .collect();
    processed = deduped;

    let processed_chars: Vec<char> = processed.chars().collect();
    if processed_chars.is_empty() {
        return String::new();
    }

    let first_char = processed_chars[0];
    let mut result = String::from(first_char);
    let mut last_code: Option<char> = soundex_el_digit(first_char);

    for &c in processed_chars.iter().skip(1) {
        if result.len() >= 4 {
            break;
        }
        let code = soundex_el_digit(c);
        if let Some(d) = code {
            if Some(d) != last_code {
                result.push(d);
            }
            last_code = Some(d);
        } else {
            last_code = None;
        }
    }

    while result.len() < 4 {
        result.push('0');
    }
    result
}

fn soundex_el_digit(c: char) -> Option<char> {
    match c {
        'B' | 'P' => Some('1'),
        'G' | 'K' => Some('2'),
        'D' | 'T' => Some('3'),
        'L' => Some('4'),
        'M' | 'N' => Some('5'),
        'R' => Some('6'),
        'S' | 'Z' => Some('7'),
        'F' | 'V' => Some('8'),
        _ => None,
    }
}

// =============================================================================
// Japanese Soundex
// =============================================================================

/// Japanese Soundex - converts Hiragana/Katakana to Romaji, then applies phonetic matching
pub fn soundex_ja(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let mut romaji = String::new();

    for c in s.chars() {
        let converted = match c {
            'あ' | 'ア' => "A",
            'い' | 'イ' => "I",
            'う' | 'ウ' => "U",
            'え' | 'エ' => "E",
            'お' | 'オ' => "O",
            'か' | 'カ' => "KA",
            'き' | 'キ' => "KI",
            'く' | 'ク' => "KU",
            'け' | 'ケ' => "KE",
            'こ' | 'コ' => "KO",
            'が' | 'ガ' => "GA",
            'ぎ' | 'ギ' => "GI",
            'ぐ' | 'グ' => "GU",
            'げ' | 'ゲ' => "GE",
            'ご' | 'ゴ' => "GO",
            'さ' | 'サ' => "SA",
            'し' | 'シ' => "SI",
            'す' | 'ス' => "SU",
            'せ' | 'セ' => "SE",
            'そ' | 'ソ' => "SO",
            'ざ' | 'ザ' => "ZA",
            'じ' | 'ジ' => "ZI",
            'ず' | 'ズ' => "ZU",
            'ぜ' | 'ゼ' => "ZE",
            'ぞ' | 'ゾ' => "ZO",
            'た' | 'タ' => "TA",
            'ち' | 'チ' => "TI",
            'つ' | 'ツ' => "TU",
            'て' | 'テ' => "TE",
            'と' | 'ト' => "TO",
            'だ' | 'ダ' => "DA",
            'ぢ' | 'ヂ' => "DI",
            'づ' | 'ヅ' => "DU",
            'で' | 'デ' => "DE",
            'ど' | 'ド' => "DO",
            'な' | 'ナ' => "NA",
            'に' | 'ニ' => "NI",
            'ぬ' | 'ヌ' => "NU",
            'ね' | 'ネ' => "NE",
            'の' | 'ノ' => "NO",
            'は' | 'ハ' => "HA",
            'ひ' | 'ヒ' => "HI",
            'ふ' | 'フ' => "HU",
            'へ' | 'ヘ' => "HE",
            'ほ' | 'ホ' => "HO",
            'ば' | 'バ' => "BA",
            'び' | 'ビ' => "BI",
            'ぶ' | 'ブ' => "BU",
            'べ' | 'ベ' => "BE",
            'ぼ' | 'ボ' => "BO",
            'ぱ' | 'パ' => "PA",
            'ぴ' | 'ピ' => "PI",
            'ぷ' | 'プ' => "PU",
            'ぺ' | 'ペ' => "PE",
            'ぽ' | 'ポ' => "PO",
            'ま' | 'マ' => "MA",
            'み' | 'ミ' => "MI",
            'む' | 'ム' => "MU",
            'め' | 'メ' => "ME",
            'も' | 'モ' => "MO",
            'や' | 'ヤ' => "YA",
            'ゆ' | 'ユ' => "YU",
            'よ' | 'ヨ' => "YO",
            'ら' | 'ラ' => "RA",
            'り' | 'リ' => "RI",
            'る' | 'ル' => "RU",
            'れ' | 'レ' => "RE",
            'ろ' | 'ロ' => "RO",
            'わ' | 'ワ' => "WA",
            'を' | 'ヲ' => "O",
            'ん' | 'ン' => "N",
            'っ' | 'ッ' => "",
            'ゃ' | 'ャ' => "YA",
            'ゅ' | 'ュ' => "YU",
            'ょ' | 'ョ' => "YO",
            'ぁ' | 'ァ' => "A",
            'ぃ' | 'ィ' => "I",
            'ぅ' | 'ゥ' => "U",
            'ぇ' | 'ェ' => "E",
            'ぉ' | 'ォ' => "O",
            'ヴ' => "VU",
            c if c.is_ascii_alphabetic() => {
                romaji.push(c.to_ascii_uppercase());
                continue;
            }
            _ => continue,
        };
        romaji.push_str(converted);
    }

    if romaji.is_empty() {
        return String::new();
    }

    let processed = romaji
        .replace("SH", "S")
        .replace("CH", "T")
        .replace("TS", "T")
        .replace("DZ", "Z")
        .replace("AA", "A")
        .replace("II", "I")
        .replace("UU", "U")
        .replace("EE", "E")
        .replace("OO", "O")
        .replace("OU", "O");

    let processed_chars: Vec<char> = processed.chars().collect();
    if processed_chars.is_empty() {
        return String::new();
    }

    let first_char = processed_chars[0];
    let mut result = String::from(first_char);
    let mut last_code: Option<char> = soundex_ja_digit(first_char);

    for &c in processed_chars.iter().skip(1) {
        if result.len() >= 4 {
            break;
        }
        let code = soundex_ja_digit(c);
        if let Some(d) = code {
            if Some(d) != last_code {
                result.push(d);
            }
            last_code = Some(d);
        } else {
            last_code = None;
        }
    }

    while result.len() < 4 {
        result.push('0');
    }
    result
}

fn soundex_ja_digit(c: char) -> Option<char> {
    match c {
        'B' | 'P' => Some('1'),
        'G' | 'K' => Some('2'),
        'D' | 'T' => Some('3'),
        'M' | 'N' => Some('4'),
        'R' => Some('5'),
        'S' | 'Z' => Some('6'),
        'H' | 'F' | 'V' => Some('7'),
        'W' | 'Y' => Some('8'),
        _ => None,
    }
}
