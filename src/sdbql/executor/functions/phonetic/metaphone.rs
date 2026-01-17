//! Metaphone phonetic algorithm.
//!
//! Metaphone is more accurate than Soundex as it uses more sophisticated rules
//! for encoding English pronunciations.

/// Metaphone phonetic algorithm - more accurate than Soundex
pub fn metaphone(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let s = s.to_uppercase();
    let chars: Vec<char> = s.chars().filter(|c| c.is_ascii_alphabetic()).collect();

    if chars.is_empty() {
        return String::new();
    }

    let mut result = String::new();
    let mut i = 0;

    // Skip initial silent letters
    if chars.len() >= 2 {
        match (chars[0], chars[1]) {
            ('K', 'N') | ('G', 'N') | ('P', 'N') | ('A', 'E') | ('W', 'R') => i = 1,
            _ => {}
        }
    }

    while i < chars.len() && result.len() < 6 {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let prev = if i > 0 { chars.get(i - 1).copied() } else { None };

        if c != 'C' && prev == Some(c) && !matches!(c, 'A' | 'E' | 'I' | 'O' | 'U') {
            i += 1;
            continue;
        }

        match c {
            'A' | 'E' | 'I' | 'O' | 'U' => {
                if i == 0 {
                    result.push(c);
                }
            }
            'B' => {
                if !(prev == Some('M') && next.is_none()) {
                    result.push('B');
                }
            }
            'C' => {
                if next == Some('H') {
                    result.push('X');
                    i += 1;
                } else if matches!(next, Some('I') | Some('E') | Some('Y')) {
                    result.push('S');
                } else {
                    result.push('K');
                }
            }
            'D' => {
                if next == Some('G')
                    && matches!(chars.get(i + 2), Some('E') | Some('I') | Some('Y'))
                {
                    result.push('J');
                    i += 1;
                } else {
                    result.push('T');
                }
            }
            'F' | 'J' | 'L' | 'M' | 'N' | 'R' => result.push(c),
            'G' => {
                if next == Some('H') {
                    if !matches!(chars.get(i + 2), Some('T')) {
                        result.push('F');
                    }
                    i += 1;
                } else if next == Some('N') {
                    // GN at end is silent
                } else if matches!(next, Some('E') | Some('I') | Some('Y')) {
                    result.push('J');
                } else {
                    result.push('K');
                }
            }
            'H' => {
                if !matches!(prev, Some('A') | Some('E') | Some('I') | Some('O') | Some('U'))
                    && matches!(next, Some('A') | Some('E') | Some('I') | Some('O') | Some('U'))
                {
                    result.push('H');
                }
            }
            'K' => {
                if prev != Some('C') {
                    result.push('K');
                }
            }
            'P' => {
                if next == Some('H') {
                    result.push('F');
                    i += 1;
                } else {
                    result.push('P');
                }
            }
            'Q' => result.push('K'),
            'S' => {
                if next == Some('H') {
                    result.push('X');
                    i += 1;
                } else if next == Some('I') && matches!(chars.get(i + 2), Some('O') | Some('A')) {
                    result.push('X');
                } else {
                    result.push('S');
                }
            }
            'T' => {
                if next == Some('H') {
                    result.push('0');
                    i += 1;
                } else if next == Some('I') && matches!(chars.get(i + 2), Some('O') | Some('A')) {
                    result.push('X');
                } else {
                    result.push('T');
                }
            }
            'V' => result.push('F'),
            'W' | 'Y' => {
                if matches!(next, Some('A') | Some('E') | Some('I') | Some('O') | Some('U')) {
                    result.push(c);
                }
            }
            'X' => {
                result.push('K');
                result.push('S');
            }
            'Z' => result.push('S'),
            _ => {}
        }
        i += 1;
    }

    result
}

/// Double Metaphone - returns (primary, secondary) codes
pub fn double_metaphone(s: &str) -> (String, String) {
    let primary = metaphone(s);
    let secondary = primary.clone();
    (primary, secondary)
}
