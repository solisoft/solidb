//! Caverphone phonetic algorithm.
//!
//! Caverphone (version 2) is optimized for matching European surnames,
//! particularly those of New Zealand European settlers.

/// Caverphone algorithm (version 2) - good for matching European surnames
pub fn caverphone(s: &str) -> String {
    if s.is_empty() {
        return "1111111111".to_string();
    }

    let mut result: String = s
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect();

    if result.is_empty() {
        return "1111111111".to_string();
    }

    if result.ends_with('e') {
        result.pop();
    }

    if result.starts_with("cough") {
        result = format!("cof2f{}", &result[5..]);
    }
    if result.starts_with("rough") {
        result = format!("rof2f{}", &result[5..]);
    }
    if result.starts_with("tough") {
        result = format!("tof2f{}", &result[5..]);
    }
    if result.starts_with("enough") {
        result = format!("enof2f{}", &result[6..]);
    }
    if result.starts_with("gn") {
        result = format!("2n{}", &result[2..]);
    }
    if result.ends_with("mb") {
        let len = result.len();
        result = format!("{}m2", &result[..len - 2]);
    }

    result = result
        .replace("cq", "2q")
        .replace("ci", "si")
        .replace("ce", "se")
        .replace("cy", "sy")
        .replace("tch", "2ch")
        .replace("c", "k")
        .replace("q", "k")
        .replace("x", "k")
        .replace("v", "f")
        .replace("dg", "2g")
        .replace("tio", "sio")
        .replace("tia", "sia")
        .replace("d", "t")
        .replace("ph", "fh")
        .replace("b", "p")
        .replace("sh", "s2")
        .replace("z", "s")
        .replace("gh", "22")
        .replace("gn", "2n")
        .replace('g', "k")
        .replace("kh", "k2")
        .replace("wh", "w2")
        .replace("wa", "2a")
        .replace("we", "2e")
        .replace("wi", "2i")
        .replace("wo", "2o")
        .replace("wu", "2u")
        .replace('w', "2");

    let chars: Vec<char> = result.chars().collect();
    let mut new_result = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == 'h' {
            let prev_vowel = i > 0 && matches!(chars[i - 1], 'a' | 'e' | 'i' | 'o' | 'u');
            let next_vowel =
                i + 1 < chars.len() && matches!(chars[i + 1], 'a' | 'e' | 'i' | 'o' | 'u');
            if prev_vowel && next_vowel {
                new_result.push('2');
            }
        } else {
            new_result.push(c);
        }
    }
    result = new_result;

    result = result
        .replace('a', "A")
        .replace('e', "A")
        .replace('i', "A")
        .replace('o', "A")
        .replace('u', "A");

    let chars: Vec<char> = result.chars().collect();
    let mut deduped = String::new();
    let mut last_char: Option<char> = None;
    for c in chars {
        if last_char != Some(c) {
            deduped.push(c);
        }
        last_char = Some(c);
    }
    result = deduped.replace('2', "");

    while result.len() < 10 {
        result.push('1');
    }
    result.truncate(10);

    result.to_uppercase()
}
