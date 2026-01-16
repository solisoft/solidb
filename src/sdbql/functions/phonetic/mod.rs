//! Phonetic matching functions for fuzzy name matching.
//!
//! This module provides various phonetic algorithms for matching names that sound similar
//! but may be spelled differently. These algorithms are particularly useful for:
//! - Searching for names across different spellings
//! - Deduplicating records with similar-sounding names
//! - Cross-cultural name matching
//!
//! # Available Algorithms
//!
//! - **soundex**: American Soundex (default English)
//! - **metaphone**: Metaphone algorithm (better than Soundex for English)
//! - **double_metaphone**: Double Metaphone (handles ambiguous pronunciations)
//! - **cologne_phonetic**: Cologne phonetic (optimized for German)
//! - **caverphone**: Caverphone (European surnames)
//! - **nysiis**: NYSIIS (various ethnic origins)
//! - **soundex_fr**: French Soundex
//! - **soundex_es**: Spanish Soundex
//! - **soundex_it**: Italian Soundex
//! - **soundex_pt**: Portuguese Soundex
//! - **soundex_nl**: Dutch Soundex
//! - **soundex_el**: Greek Soundex
//! - **soundex_ja**: Japanese Soundex (Romaji-based)

pub mod soundex;
pub mod metaphone;
pub mod double_metaphone;
pub mod cologne_phonetic;
pub mod caverphone;
pub mod nysiis;
pub mod soundex_fr;
pub mod soundex_es;
pub mod soundex_it;
pub mod soundex_pt;
pub mod soundex_nl;
pub mod soundex_el;
pub mod soundex_ja;

pub use soundex::soundex;
pub use metaphone::metaphone;
pub use double_metaphone::double_metaphone;
pub use cologne_phonetic::cologne_phonetic;
pub use caverphone::caverphone;
pub use nysiis::nysiis;
pub use soundex_fr::soundex_fr;
pub use soundex_es::soundex_es;
pub use soundex_it::soundex_it;
pub use soundex_pt::soundex_pt;
pub use soundex_nl::soundex_nl;
pub use soundex_el::soundex_el;
pub use soundex_ja::soundex_ja;
