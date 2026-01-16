/// Spanish Soundex - optimized for Spanish/Castilian names
/// Handles Spanish-specific phonetic rules:
/// - Ñ (eñe) sound
/// - LL and Y equivalence
/// - H is always silent
/// - B/V equivalence
/// - C/Z/S equivalence in many dialects (seseo)
pub fn soundex_es(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    // Normalize: uppercase and handle Spanish characters
    let s = s
        .to_uppercase()
        .replace('Á', "A")
        .replace('É', "E")
        .replace('Í', "I")
        .replace('Ó', "O")
        .replace('Ú', "U")
        .replace('Ü', "U")
        .replace('Ñ', "NY"); // Ñ sounds like NY

    let chars: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();

    if chars.is_empty() {
        return String::new();
    }

    let word: String = chars.iter().collect();
    let mut processed = word;

    // Spanish phonetic transformations
    processed = processed
        .replace("CH", "X") // CH is a single sound
        .replace("LL", "Y") // LL and Y are equivalent
        .replace("RR", "R") // Double R is still R
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
        .replace("ZU", "SU");

    // C before other letters is K
    processed = processed.replace('C', "K");

    // B and V are equivalent in Spanish
    processed = processed.replace('B', "V");

    // H is always silent in Spanish
    processed = processed.replace('H', "");

    // Use first char from processed string (after transformations)
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
        'B' | 'V' | 'W' => Some('1'), // B and V sound the same in Spanish
        'K' | 'Q' => Some('2'),
        'D' | 'T' => Some('3'),
        'L' => Some('4'),
        'M' | 'N' => Some('5'),
        'R' => Some('6'),
        'G' | 'J' | 'X' => Some('7'), // J and G before e/i, X (from CH)
        'S' | 'Z' => Some('8'),
        'F' | 'P' => Some('9'),
        'Y' => Some('0'), // Y as consonant
        _ => None,
    }
}
