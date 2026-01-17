//! NYSIIS phonetic algorithm.
//!
//! NYSIIS (New York State Identification and Intelligence System) is a
//! phonetic algorithm developed for use in criminal justice systems.

/// NYSIIS (New York State Identification and Intelligence System) algorithm
pub fn nysiis(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let mut name: String = s
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect();

    if name.is_empty() {
        return String::new();
    }

    if name.starts_with("MAC") {
        name = format!("MCC{}", &name[3..]);
    } else if name.starts_with("KN") {
        name = format!("NN{}", &name[2..]);
    } else if name.starts_with('K') {
        name = format!("C{}", &name[1..]);
    } else if name.starts_with("PH") || name.starts_with("PF") {
        name = format!("FF{}", &name[2..]);
    } else if name.starts_with("SCH") {
        name = format!("SSS{}", &name[3..]);
    }

    if name.ends_with("EE") || name.ends_with("IE") {
        let len = name.len();
        name = format!("{}Y", &name[..len - 2]);
    } else if name.ends_with("DT")
        || name.ends_with("RT")
        || name.ends_with("RD")
        || name.ends_with("NT")
        || name.ends_with("ND")
    {
        let len = name.len();
        name = format!("{}D", &name[..len - 2]);
    }

    let first_char = name.chars().next().unwrap();
    let chars: Vec<char> = name.chars().collect();
    let mut result = String::from(first_char);
    let mut i = 1;

    while i < chars.len() {
        let c = chars[i];
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        let next = chars.get(i + 1).copied();

        let replacement = match c {
            'E' | 'I' | 'O' | 'U' => 'A',
            'Q' => 'G',
            'Z' => 'S',
            'M' => 'N',
            'K' => {
                if next == Some('N') {
                    'N'
                } else {
                    'C'
                }
            }
            'S' => {
                if next == Some('C') && chars.get(i + 2) == Some(&'H') {
                    result.push('S');
                    result.push('S');
                    i += 2;
                    'S'
                } else if next == Some('H') {
                    i += 1;
                    'S'
                } else {
                    'S'
                }
            }
            'P' => {
                if next == Some('H') {
                    i += 1;
                    'F'
                } else {
                    'P'
                }
            }
            'H' => {
                let prev_vowel = matches!(prev, Some('A') | Some('E') | Some('I') | Some('O') | Some('U'));
                let next_vowel = matches!(next, Some('A') | Some('E') | Some('I') | Some('O') | Some('U'));
                if !prev_vowel || !next_vowel {
                    if let Some(p) = prev {
                        p
                    } else {
                        i += 1;
                        continue;
                    }
                } else {
                    'H'
                }
            }
            'W' => {
                let prev_vowel = matches!(prev, Some('A') | Some('E') | Some('I') | Some('O') | Some('U'));
                if prev_vowel {
                    prev.unwrap_or('W')
                } else {
                    'W'
                }
            }
            _ => c,
        };

        if result.chars().last() != Some(replacement) {
            result.push(replacement);
        }

        i += 1;
    }

    if result.ends_with('S') && result.len() > 1 {
        result.pop();
    }
    if result.ends_with('A') && result.len() > 1 {
        result.pop();
    }
    if result.ends_with("AY") {
        let len = result.len();
        result = format!("{}Y", &result[..len - 2]);
    }

    result
}
