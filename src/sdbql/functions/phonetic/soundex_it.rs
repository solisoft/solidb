/// Italian Soundex - optimized for Italian names
/// Handles Italian-specific phonetic rules:
/// - GL (like "gli") and GN (like "gnocchi")
/// - SC before e/i
/// - Double consonants
/// - H is silent
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
    let mut processed = word;

    // Italian phonetic transformations
    processed = processed
        .replace("GLI", "LI") // GL before I
        .replace("GN", "N") // GN is a single nasal sound
        .replace("SCE", "SE") // SC before E
        .replace("SCI", "SI") // SC before I
        .replace("CHE", "KE") // CH before E
        .replace("CHI", "KI") // CH before I
        .replace("GHE", "GE") // GH before E
        .replace("GHI", "GI") // GH before I
        .replace("CE", "CE") // C before E = CH sound
        .replace("CI", "CI") // C before I = CH sound
        .replace("GE", "JE") // G before E = J sound
        .replace("GI", "JI") // G before I = J sound
        .replace("QU", "KU");

    // Remove H (always silent)
    processed = processed.replace('H', "");

    // Remove double consonants
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
