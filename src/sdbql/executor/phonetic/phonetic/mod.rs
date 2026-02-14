//! Phonetic matching algorithms for SDBQL.
//!
//! This module provides phonetic matching functions for fuzzy name matching:
//! - American Soundex
//! - Metaphone / Double Metaphone
//! - Cologne Phonetic (German)
//! - Caverphone (European surnames)
//! - NYSIIS (New York State)
//! - Language-specific: French, Spanish, Italian, Portuguese, Dutch, Greek, Japanese

mod caverphone;
mod cologne;
mod localized;
mod metaphone;
mod nysiis;
mod soundex;

pub use caverphone::caverphone;
pub use cologne::cologne_phonetic;
pub use localized::{
    soundex_el, soundex_es, soundex_fr, soundex_it, soundex_ja, soundex_nl, soundex_pt,
};
pub use metaphone::{double_metaphone, metaphone};
pub use nysiis::nysiis;
pub use soundex::soundex;

use crate::error::DbResult;
use serde_json::Value;

/// Evaluate phonetic functions by name
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "SOUNDEX" => {
            let s = args.first().and_then(|v| v.as_str());
            if let Some(s) = s {
                let locale = args.get(1).and_then(|v| v.as_str()).unwrap_or("en");
                let result = match locale {
                    "de" => cologne_phonetic(s),
                    "fr" => soundex_fr(s),
                    "es" => soundex_es(s),
                    "it" => soundex_it(s),
                    "pt" => soundex_pt(s),
                    "nl" => soundex_nl(s),
                    "el" => soundex_el(s),
                    "ja" => soundex_ja(s),
                    _ => soundex(s),
                };
                Ok(Some(Value::String(result)))
            } else {
                Ok(Some(Value::Null))
            }
        }
        "METAPHONE" => {
            if let Some(Value::String(s)) = args.first() {
                Ok(Some(Value::String(metaphone(s))))
            } else {
                Ok(Some(Value::Null))
            }
        }
        "DOUBLE_METAPHONE" => {
            if let Some(Value::String(s)) = args.first() {
                let (primary, secondary) = double_metaphone(s);
                Ok(Some(Value::Array(vec![
                    Value::String(primary),
                    Value::String(secondary),
                ])))
            } else {
                Ok(Some(Value::Null))
            }
        }
        "COLOGNE_PHONETIC" => {
            if let Some(Value::String(s)) = args.first() {
                Ok(Some(Value::String(cologne_phonetic(s))))
            } else {
                Ok(Some(Value::Null))
            }
        }
        "CAVERPHONE" => {
            if let Some(Value::String(s)) = args.first() {
                Ok(Some(Value::String(caverphone(s))))
            } else {
                Ok(Some(Value::Null))
            }
        }
        "NYSIIS" => {
            if let Some(Value::String(s)) = args.first() {
                Ok(Some(Value::String(nysiis(s))))
            } else {
                Ok(Some(Value::Null))
            }
        }
        "SOUNDEX_FR" => {
            if let Some(Value::String(s)) = args.first() {
                Ok(Some(Value::String(soundex_fr(s))))
            } else {
                Ok(Some(Value::Null))
            }
        }
        "SOUNDEX_ES" => {
            if let Some(Value::String(s)) = args.first() {
                Ok(Some(Value::String(soundex_es(s))))
            } else {
                Ok(Some(Value::Null))
            }
        }
        "SOUNDEX_IT" => {
            if let Some(Value::String(s)) = args.first() {
                Ok(Some(Value::String(soundex_it(s))))
            } else {
                Ok(Some(Value::Null))
            }
        }
        "SOUNDEX_PT" => {
            if let Some(Value::String(s)) = args.first() {
                Ok(Some(Value::String(soundex_pt(s))))
            } else {
                Ok(Some(Value::Null))
            }
        }
        "SOUNDEX_NL" => {
            if let Some(Value::String(s)) = args.first() {
                Ok(Some(Value::String(soundex_nl(s))))
            } else {
                Ok(Some(Value::Null))
            }
        }
        "SOUNDEX_EL" => {
            if let Some(Value::String(s)) = args.first() {
                Ok(Some(Value::String(soundex_el(s))))
            } else {
                Ok(Some(Value::Null))
            }
        }
        "SOUNDEX_JA" => {
            if let Some(Value::String(s)) = args.first() {
                Ok(Some(Value::String(soundex_ja(s))))
            } else {
                Ok(Some(Value::Null))
            }
        }
        _ => Ok(None),
    }
}
