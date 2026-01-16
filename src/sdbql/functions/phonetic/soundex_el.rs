/// Greek Soundex - optimized for Greek names
/// Handles Greek alphabet transliteration and phonetic rules:
/// - Greek letters (α, β, γ, δ, etc.) to Latin equivalents
/// - Digraphs: ου→U, αι→E, ει→I, οι→I, μπ→B, ντ→D, γκ→G
/// - Double consonants
pub fn soundex_el(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    // Transliterate Greek to Latin
    let mut transliterated = s.to_uppercase();

    // Handle Greek digraphs first (order matters)
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

    // Transliterate individual Greek letters
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
        .replace('Σ', "S") // Final sigma
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
    let mut processed = word;

    // Simplify common combinations
    processed = processed
        .replace("TH", "T")
        .replace("CH", "K")
        .replace("PS", "S")
        .replace("KS", "S");

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
