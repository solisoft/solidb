/// Japanese Soundex - optimized for Japanese names
/// Converts Hiragana and Katakana to Romaji, then applies phonetic matching
/// Note: Kanji characters are skipped (require dictionary for reading)
pub fn soundex_ja(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    // Convert to Romaji
    let mut romaji = String::new();

    for c in s.chars() {
        let converted = match c {
            // Hiragana vowels
            'あ' | 'ア' => "A",
            'い' | 'イ' => "I",
            'う' | 'ウ' => "U",
            'え' | 'エ' => "E",
            'お' | 'オ' => "O",

            // Hiragana K-row + voiced G
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

            // Hiragana S-row + voiced Z
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

            // Hiragana T-row + voiced D
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

            // Hiragana N-row
            'な' | 'ナ' => "NA",
            'に' | 'ニ' => "NI",
            'ぬ' | 'ヌ' => "NU",
            'ね' | 'ネ' => "NE",
            'の' | 'ノ' => "NO",

            // Hiragana H-row + voiced B + semi-voiced P
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

            // Hiragana M-row
            'ま' | 'マ' => "MA",
            'み' | 'ミ' => "MI",
            'む' | 'ム' => "MU",
            'め' | 'メ' => "ME",
            'も' | 'モ' => "MO",

            // Hiragana Y-row
            'や' | 'ヤ' => "YA",
            'ゆ' | 'ユ' => "YU",
            'よ' | 'ヨ' => "YO",

            // Hiragana R-row
            'ら' | 'ラ' => "RA",
            'り' | 'リ' => "RI",
            'る' | 'ル' => "RU",
            'れ' | 'レ' => "RE",
            'ろ' | 'ロ' => "RO",

            // Hiragana W-row and N
            'わ' | 'ワ' => "WA",
            'を' | 'ヲ' => "O",
            'ん' | 'ン' => "N",

            // Small kana (for combinations)
            'っ' | 'ッ' => "", // Double consonant marker - handled separately
            'ゃ' | 'ャ' => "YA",
            'ゅ' | 'ュ' => "YU",
            'ょ' | 'ョ' => "YO",
            'ぁ' | 'ァ' => "A",
            'ぃ' | 'ィ' => "I",
            'ぅ' | 'ゥ' => "U",
            'ぇ' | 'ェ' => "E",
            'ぉ' | 'ォ' => "O",

            // Extended Katakana
            'ヴ' => "VU",

            // Latin letters (pass through uppercase)
            c if c.is_ascii_alphabetic() => {
                romaji.push(c.to_ascii_uppercase());
                continue;
            }

            // Skip other characters (including Kanji)
            _ => continue,
        };
        romaji.push_str(converted);
    }

    if romaji.is_empty() {
        return String::new();
    }

    // Apply Japanese phonetic simplifications
    let mut processed = romaji;

    // Simplify common combinations
    processed = processed
        .replace("SH", "S")
        .replace("CH", "T")
        .replace("TS", "T")
        .replace("DZ", "Z");

    // Remove double vowels (long vowels)
    processed = processed
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
        _ => None, // Vowels A, E, I, O, U
    }
}
