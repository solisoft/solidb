/// French Soundex (Soundex Français) - optimized for French names
/// Handles French-specific phonetic rules:
/// - Silent endings (t, d, s, x, z, p at end of words)
/// - Nasal vowels (an, en, in, on, un → numeric codes)
/// - Special combinations (eau, au, ou, ch, gn, ph, etc.)
/// - Accented characters (é, è, ê, à, etc.)
pub fn soundex_fr(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    // Normalize: uppercase and remove accents
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

    // Build the string for processing
    let word: String = chars.iter().collect();

    // Apply French phonetic transformations
    let mut processed = word.clone();

    // Remove silent endings
    if processed.len() > 2 {
        let last_char = processed.chars().last().unwrap();
        if matches!(last_char, 'T' | 'D' | 'S' | 'X' | 'Z' | 'P') {
            processed.pop();
        }
    }

    // Handle common French letter combinations
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
        .replace("GY", "JI");

    // C before E, I, Y becomes S
    processed = processed
        .replace("CE", "SE")
        .replace("CI", "SI")
        .replace("CY", "SY");

    // Remaining C becomes K
    processed = processed.replace('C', "K");

    // Handle nasal vowels (simplified)
    processed = processed
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

    // Get first character (save original letter)
    let first_char = chars[0];
    let mut result = String::from(first_char);

    // Process remaining characters with French Soundex mapping
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
            // Vowels and silent letters reset the last code
            last_code = None;
        }
    }

    // Pad with zeros to length 4
    while result.len() < 4 {
        result.push('0');
    }

    result
}

/// French Soundex digit mapping
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
        '1' | '2' | '3' | '4' => Some(c), // Keep nasal vowel codes
        _ => None,                        // Vowels A, E, I, O, U, H, W, Y
    }
}
