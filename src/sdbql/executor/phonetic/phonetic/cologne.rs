//! Cologne Phonetic algorithm (Kölner Phonetik).
//!
//! Optimized for German names and words. Produces numeric codes that group
//! phonetically similar German words.

/// Cologne Phonetic algorithm - optimized for German names
pub fn cologne_phonetic(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let s = s
        .to_uppercase()
        .replace('Ä', "A")
        .replace('Ö', "O")
        .replace('Ü', "U")
        .replace('ß', "SS");

    let chars: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();

    if chars.is_empty() {
        return String::new();
    }

    let mut result = String::new();
    let mut last_code: Option<char> = None;

    for (i, &c) in chars.iter().enumerate() {
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        let next = chars.get(i + 1).copied();

        let code = match c {
            'A' | 'E' | 'I' | 'J' | 'O' | 'U' | 'Y' => Some('0'),
            'B' => Some('1'),
            'P' => {
                if next == Some('H') {
                    Some('3')
                } else {
                    Some('1')
                }
            }
            'D' | 'T' => {
                if matches!(next, Some('C') | Some('S') | Some('Z')) {
                    Some('8')
                } else {
                    Some('2')
                }
            }
            'F' | 'V' | 'W' => Some('3'),
            'G' | 'K' | 'Q' => Some('4'),
            'C' => {
                if i == 0 {
                    if matches!(
                        next,
                        Some('A')
                            | Some('H')
                            | Some('K')
                            | Some('L')
                            | Some('O')
                            | Some('Q')
                            | Some('R')
                            | Some('U')
                            | Some('X')
                    ) {
                        Some('4')
                    } else {
                        Some('8')
                    }
                } else if matches!(prev, Some('S') | Some('Z')) {
                    Some('8')
                } else if matches!(
                    next,
                    Some('A') | Some('H') | Some('K') | Some('O') | Some('Q') | Some('U') | Some('X')
                ) {
                    Some('4')
                } else {
                    Some('8')
                }
            }
            'X' => {
                if matches!(prev, Some('C') | Some('K') | Some('Q')) {
                    Some('8')
                } else {
                    result.push('4');
                    Some('8')
                }
            }
            'L' => Some('5'),
            'M' | 'N' => Some('6'),
            'R' => Some('7'),
            'S' | 'Z' => Some('8'),
            'H' => None,
            _ => None,
        };

        if let Some(c) = code {
            if last_code != Some(c) {
                result.push(c);
            }
            last_code = Some(c);
        }
    }

    let trimmed: String = result.trim_start_matches('0').to_string();
    if trimmed.is_empty() && !result.is_empty() {
        "0".to_string()
    } else {
        trimmed
    }
}
