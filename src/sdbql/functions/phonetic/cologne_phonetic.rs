/// Cologne Phonetic algorithm - optimized for German names
/// Returns a numeric string code where similar-sounding German names produce the same code
/// Examples: Müller/Mueller/Miller all produce the same code
pub fn cologne_phonetic(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    // Normalize: uppercase and handle German umlauts
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
                    // Initial C
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
                    Some('A')
                        | Some('H')
                        | Some('K')
                        | Some('O')
                        | Some('Q')
                        | Some('U')
                        | Some('X')
                ) {
                    // C before A,H,K,O,Q,U,X (not after S,Z)
                    Some('4')
                } else {
                    Some('8')
                }
            }
            'X' => {
                if matches!(prev, Some('C') | Some('K') | Some('Q')) {
                    Some('8')
                } else {
                    // X = 48
                    result.push('4');
                    Some('8')
                }
            }
            'L' => Some('5'),
            'M' | 'N' => Some('6'),
            'R' => Some('7'),
            'S' | 'Z' => Some('8'),
            'H' => None, // H is ignored (produces no code)
            _ => None,
        };

        if let Some(c) = code {
            // Remove consecutive duplicates
            if last_code != Some(c) {
                result.push(c);
            }
            last_code = Some(c);
        }
    }

    // Remove leading zeros (except if that's all we have)
    let trimmed: String = result.trim_start_matches('0').to_string();
    if trimmed.is_empty() && !result.is_empty() {
        "0".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cologne_phonetic() {
        assert_eq!(cologne_phonetic("Müller"), "657");
        assert_eq!(cologne_phonetic("Mueller"), "657");
    }

    #[test]
    fn test_cologne_phonetic_empty() {
        assert_eq!(cologne_phonetic(""), "");
    }
}
