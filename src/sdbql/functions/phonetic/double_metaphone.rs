use super::metaphone::metaphone;

/// Double Metaphone - returns (primary, secondary) codes
/// Useful for names with ambiguous pronunciations (e.g., "Schmidt" in German vs English)
pub fn double_metaphone(s: &str) -> (String, String) {
    // For a full implementation, consider using the `rphonetic` crate
    // This simplified version returns the same code for both
    let primary = metaphone(s);
    let secondary = primary.clone();
    (primary, secondary)
}
