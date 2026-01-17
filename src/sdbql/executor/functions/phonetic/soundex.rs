//! American Soundex phonetic algorithm.
//!
//! The Soundex algorithm encodes names by sound, as pronounced in English.
//! It produces a 4-character code where the first character is the first letter
//! of the name and the remaining three are digits.

/// American Soundex algorithm - returns 4-character phonetic code
/// Example: "Smith" and "Smyth" both return "S530"
pub fn soundex(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let s = s.to_uppercase();
    let chars: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();

    if chars.is_empty() {
        return String::new();
    }

    let first = chars[0];
    let mut code = String::from(first);
    let mut last_digit = soundex_digit(first);

    for &ch in &chars[1..] {
        let digit = soundex_digit(ch);
        if let Some(d) = digit {
            if Some(d) != last_digit {
                code.push(d);
                if code.len() == 4 {
                    break;
                }
            }
            last_digit = Some(d);
        } else {
            last_digit = None;
        }
    }

    while code.len() < 4 {
        code.push('0');
    }

    code
}

fn soundex_digit(c: char) -> Option<char> {
    match c {
        'B' | 'F' | 'P' | 'V' => Some('1'),
        'C' | 'G' | 'J' | 'K' | 'Q' | 'S' | 'X' | 'Z' => Some('2'),
        'D' | 'T' => Some('3'),
        'L' => Some('4'),
        'M' | 'N' => Some('5'),
        'R' => Some('6'),
        _ => None,
    }
}
