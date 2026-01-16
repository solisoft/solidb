/// Portuguese Soundex - optimized for Portuguese names
/// Handles Portuguese-specific phonetic rules:
/// - Ç (cedilha)
/// - NH and LH digraphs
/// - Nasal vowels (ã, õ)
/// - X with multiple sounds
pub fn soundex_pt(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let s = s
        .to_uppercase()
        .replace('Á', "A")
        .replace('À', "A")
        .replace('Â', "A")
        .replace('Ã', "AN") // Nasal A
        .replace('É', "E")
        .replace('Ê', "E")
        .replace('Í', "I")
        .replace('Ó', "O")
        .replace('Ô', "O")
        .replace('Õ', "ON") // Nasal O
        .replace('Ú', "U")
        .replace('Ü', "U")
        .replace('Ç', "S");

    let chars: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();

    if chars.is_empty() {
        return String::new();
    }

    let word: String = chars.iter().collect();
    let mut processed = word;

    // Portuguese phonetic transformations
    processed = processed
        .replace("NH", "N") // NH is a palatal nasal
        .replace("LH", "L") // LH is a palatal lateral
        .replace("CH", "X") // CH sounds like SH
        .replace("RR", "R") // Double R
        .replace("SS", "S") // Double S
        .replace("QU", "K")
        .replace("GU", "G")
        .replace("CE", "SE")
        .replace("CI", "SI")
        .replace("GE", "JE")
        .replace("GI", "JI");

    // H is silent at start
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
