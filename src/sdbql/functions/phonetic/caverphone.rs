/// Caverphone algorithm (version 2) - good for matching European surnames
/// Returns a 10-character code, particularly effective for English and European names
pub fn caverphone(s: &str) -> String {
    if s.is_empty() {
        return "1111111111".to_string();
    }

    let mut result = s.to_lowercase();

    // Remove anything not a letter
    result = result.chars().filter(|c| c.is_ascii_alphabetic()).collect();

    if result.is_empty() {
        return "1111111111".to_string();
    }

    // Apply transformation rules in order

    // Remove final 'e'
    if result.ends_with('e') {
        result.pop();
    }

    // Initial transformations
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

    // Common substitutions
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
        .replace("wh", "w2");

    // Handle 'w' followed by vowel
    result = result
        .replace("wa", "2a")
        .replace("we", "2e")
        .replace("wi", "2i")
        .replace("wo", "2o")
        .replace("wu", "2u");

    // Remove 'w' if not followed by vowel (remaining w's)
    result = result.replace('w', "2");

    // Handle 'h' (keep if between two vowels or at start before vowel)
    let chars: Vec<char> = result.chars().collect();
    let mut new_result = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == 'h' {
            let prev_vowel = i > 0
                && matches!(
                    chars[i - 1],
                    'a' | 'e' | 'i' | 'o' | 'u' | 'A' | 'E' | 'I' | 'O' | 'U'
                );
            let next_vowel = i + 1 < chars.len()
                && matches!(
                    chars[i + 1],
                    'a' | 'e' | 'i' | 'o' | 'u' | 'A' | 'E' | 'I' | 'O' | 'U'
                );
            if prev_vowel && next_vowel {
                new_result.push('2');
            }
            // else drop h
        } else {
            new_result.push(c);
        }
    }
    result = new_result;

    // Replace vowels
    result = result
        .replace('a', "A")
        .replace('e', "A")
        .replace('i', "A")
        .replace('o', "A")
        .replace('u', "A");

    // Remove duplicate adjacent letters
    let chars: Vec<char> = result.chars().collect();
    let mut deduped = String::new();
    let mut last_char: Option<char> = None;
    for c in chars {
        if last_char != Some(c) {
            deduped.push(c);
        }
        last_char = Some(c);
    }
    result = deduped;

    // Remove all '2's
    result = result.replace('2', "");

    // Pad with 1's or truncate to 10 characters
    while result.len() < 10 {
        result.push('1');
    }
    result.truncate(10);

    result.to_uppercase()
}
