/// Dutch Soundex - optimized for Dutch names
/// Handles Dutch-specific phonetic rules:
/// - IJ digraph (treated as single letter)
/// - CH and SCH sounds
/// - Double vowels (aa, ee, oo, uu)
/// - W sounds like V
pub fn soundex_nl(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let s = s
        .to_uppercase()
        .replace("IJ", "Y") // IJ is a single letter in Dutch
        .replace('Ë', "E")
        .replace('Ï', "I")
        .replace('É', "E");

    let chars: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();

    if chars.is_empty() {
        return String::new();
    }

    let word: String = chars.iter().collect();
    let mut processed = word;

    // Dutch phonetic transformations
    processed = processed
        .replace("SCH", "S") // SCH at start often sounds like S
        .replace("CH", "G") // CH sounds like G
        .replace("PH", "F")
        .replace("QU", "KW")
        .replace("TH", "T")
        .replace("DT", "T")
        .replace("AA", "A") // Long vowels
        .replace("EE", "E")
        .replace("OO", "O")
        .replace("UU", "U");

    // W and V are often interchangeable in Dutch
    processed = processed.replace('W', "V");

    // Use first char from processed string (after transformations)
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
