//! Tests for SDBQL Phonetic Matching Functions
//! SOUNDEX, METAPHONE, and DOUBLE_METAPHONE

use serde_json::json;
use solidb::parse;
use solidb::sdbql::QueryExecutor;
use solidb::storage::StorageEngine;
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine =
        StorageEngine::new(tmp_dir.path().to_str().unwrap()).expect("Failed to create storage");
    (engine, tmp_dir)
}

fn execute_query(engine: &StorageEngine, query_str: &str) -> Vec<serde_json::Value> {
    let query = parse(query_str).expect("Failed to parse query");
    let executor = QueryExecutor::new(engine);
    executor.execute(&query).expect("Failed to execute query")
}

// =============================================================================
// SOUNDEX Tests
// =============================================================================

#[test]
fn test_soundex_basic() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Robert")"#);
    assert_eq!(result, vec![json!("R163")]);
}

#[test]
fn test_soundex_matching_names() {
    let (engine, _tmp) = create_test_engine();

    // Smith and Smyth should have same Soundex
    let smith = execute_query(&engine, r#"RETURN SOUNDEX("Smith")"#);
    let smyth = execute_query(&engine, r#"RETURN SOUNDEX("Smyth")"#);
    assert_eq!(smith, smyth);
    assert_eq!(smith, vec![json!("S530")]);
}

#[test]
fn test_soundex_robert_rupert() {
    let (engine, _tmp) = create_test_engine();

    // Robert and Rupert should have same Soundex
    let robert = execute_query(&engine, r#"RETURN SOUNDEX("Robert")"#);
    let rupert = execute_query(&engine, r#"RETURN SOUNDEX("Rupert")"#);
    assert_eq!(robert, rupert);
    assert_eq!(robert, vec![json!("R163")]);
}

#[test]
fn test_soundex_empty_string() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX("")"#);
    assert_eq!(result, vec![json!("")]);
}

#[test]
fn test_soundex_single_letter() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX("A")"#);
    assert_eq!(result, vec![json!("A000")]);
}

#[test]
fn test_soundex_null() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX(null)"#);
    assert_eq!(result, vec![json!(null)]);
}

#[test]
fn test_soundex_case_insensitive() {
    let (engine, _tmp) = create_test_engine();
    let upper = execute_query(&engine, r#"RETURN SOUNDEX("SMITH")"#);
    let lower = execute_query(&engine, r#"RETURN SOUNDEX("smith")"#);
    let mixed = execute_query(&engine, r#"RETURN SOUNDEX("SmItH")"#);
    assert_eq!(upper, lower);
    assert_eq!(lower, mixed);
}

#[test]
fn test_soundex_comparison() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Johnson") == SOUNDEX("Jonson")"#);
    assert_eq!(result, vec![json!(true)]);
}

#[test]
fn test_soundex_different_names() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Smith") == SOUNDEX("Jones")"#);
    assert_eq!(result, vec![json!(false)]);
}

// =============================================================================
// METAPHONE Tests
// =============================================================================

#[test]
fn test_metaphone_basic() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN METAPHONE("Smith")"#);
    assert_eq!(result, vec![json!("SM0")]);
}

#[test]
fn test_metaphone_katherine_catherine() {
    let (engine, _tmp) = create_test_engine();

    // Katherine and Catherine should match
    let katherine = execute_query(&engine, r#"RETURN METAPHONE("Katherine")"#);
    let catherine = execute_query(&engine, r#"RETURN METAPHONE("Catherine")"#);
    assert_eq!(katherine, catherine);
}

#[test]
fn test_metaphone_wright_right() {
    let (engine, _tmp) = create_test_engine();

    // Wright and Right should match (silent W)
    let wright = execute_query(&engine, r#"RETURN METAPHONE("Wright")"#);
    let right = execute_query(&engine, r#"RETURN METAPHONE("Right")"#);
    assert_eq!(wright, right);
}

#[test]
fn test_metaphone_phone_fone() {
    let (engine, _tmp) = create_test_engine();

    // Phone and Fone should match (PH -> F)
    let phone = execute_query(&engine, r#"RETURN METAPHONE("Phone")"#);
    let fone = execute_query(&engine, r#"RETURN METAPHONE("Fone")"#);
    assert_eq!(phone, fone);
}

#[test]
fn test_metaphone_knife() {
    let (engine, _tmp) = create_test_engine();

    // Knife - silent K
    let result = execute_query(&engine, r#"RETURN METAPHONE("Knife")"#);
    assert_eq!(result, vec![json!("NF")]);
}

#[test]
fn test_metaphone_empty_string() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN METAPHONE("")"#);
    assert_eq!(result, vec![json!("")]);
}

#[test]
fn test_metaphone_null() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN METAPHONE(null)"#);
    assert_eq!(result, vec![json!(null)]);
}

#[test]
fn test_metaphone_philip_phillip() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(
        &engine,
        r#"RETURN METAPHONE("Philip") == METAPHONE("Phillip")"#,
    );
    assert_eq!(result, vec![json!(true)]);
}

// =============================================================================
// DOUBLE_METAPHONE Tests
// =============================================================================

#[test]
fn test_double_metaphone_returns_array() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN DOUBLE_METAPHONE("Smith")"#);

    // Should return array with two elements
    assert_eq!(result.len(), 1);
    if let serde_json::Value::Array(arr) = &result[0] {
        assert_eq!(arr.len(), 2);
    } else {
        panic!("Expected array");
    }
}

#[test]
fn test_double_metaphone_null() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN DOUBLE_METAPHONE(null)"#);
    assert_eq!(result, vec![json!(null)]);
}

#[test]
fn test_double_metaphone_primary() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN DOUBLE_METAPHONE("Smith")[0]"#);
    // Primary code should match METAPHONE
    let metaphone = execute_query(&engine, r#"RETURN METAPHONE("Smith")"#);
    assert_eq!(result, metaphone);
}

// =============================================================================
// Integration Tests with Collections
// =============================================================================

#[test]
fn test_soundex_filter_collection() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("names".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine
        .get_collection("names")
        .expect("Collection not found");

    coll.insert(json!({"_key": "1", "name": "Smith"})).unwrap();
    coll.insert(json!({"_key": "2", "name": "Smyth"})).unwrap();
    coll.insert(json!({"_key": "3", "name": "Smythe"})).unwrap();
    coll.insert(json!({"_key": "4", "name": "Jones"})).unwrap();
    coll.insert(json!({"_key": "5", "name": "Schmidt"}))
        .unwrap();

    let result = execute_query(
        &engine,
        r#"
        FOR doc IN names
          FILTER SOUNDEX(doc.name) == SOUNDEX("Smith")
          SORT doc.name
          RETURN doc.name
    "#,
    );

    // Smith, Smyth, Smythe, and Schmidt all have same Soundex (S530)
    // Schmidt matches because S→2 and C→2 are adjacent duplicates, so C is dropped
    assert_eq!(
        result,
        vec![
            json!("Schmidt"),
            json!("Smith"),
            json!("Smyth"),
            json!("Smythe")
        ]
    );
}

#[test]
fn test_metaphone_filter_collection() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("contacts".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine
        .get_collection("contacts")
        .expect("Collection not found");

    coll.insert(json!({"_key": "1", "name": "Katherine"}))
        .unwrap();
    coll.insert(json!({"_key": "2", "name": "Catherine"}))
        .unwrap();
    coll.insert(json!({"_key": "3", "name": "Kathryn"}))
        .unwrap();
    coll.insert(json!({"_key": "4", "name": "John"})).unwrap();

    let result = execute_query(
        &engine,
        r#"
        FOR doc IN contacts
          FILTER METAPHONE(doc.name) == METAPHONE("Katherine")
          SORT doc.name
          RETURN doc.name
    "#,
    );

    assert!(result.len() >= 2); // At least Katherine and Catherine
}

#[test]
fn test_soundex_with_levenshtein_combo() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("users".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine
        .get_collection("users")
        .expect("Collection not found");

    coll.insert(json!({"_key": "1", "name": "Johnson"}))
        .unwrap();
    coll.insert(json!({"_key": "2", "name": "Jonson"})).unwrap();
    coll.insert(json!({"_key": "3", "name": "Johnston"}))
        .unwrap();
    coll.insert(json!({"_key": "4", "name": "Smith"})).unwrap();

    // Combine SOUNDEX with LEVENSHTEIN for comprehensive fuzzy matching
    let result = execute_query(
        &engine,
        r#"
        FOR doc IN users
          LET soundexMatch = SOUNDEX(doc.name) == SOUNDEX("Johnson")
          LET editDist = LEVENSHTEIN(doc.name, "Johnson")
          FILTER soundexMatch OR editDist <= 3
          SORT soundexMatch DESC, editDist ASC
          RETURN { name: doc.name, soundexMatch: soundexMatch, editDist: editDist }
    "#,
    );

    assert!(result.len() >= 2);
}

#[test]
fn test_soundex_in_return_object() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("people".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine
        .get_collection("people")
        .expect("Collection not found");

    coll.insert(json!({"_key": "1", "name": "Alice"})).unwrap();

    let result = execute_query(
        &engine,
        r#"
        FOR doc IN people
          RETURN {
            name: doc.name,
            soundex: SOUNDEX(doc.name),
            metaphone: METAPHONE(doc.name)
          }
    "#,
    );

    assert_eq!(result.len(), 1);
    let obj = &result[0];
    assert_eq!(obj["name"], json!("Alice"));
    assert!(obj["soundex"].is_string());
    assert!(obj["metaphone"].is_string());
}

// =============================================================================
// COLOGNE (German Phonetic) Tests
// =============================================================================

#[test]
fn test_cologne_basic() {
    let (engine, _tmp) = create_test_engine();
    // Müller should produce a code
    let result = execute_query(&engine, r#"RETURN COLOGNE("Müller")"#);
    assert_eq!(result.len(), 1);
    assert!(result[0].is_string());
}

#[test]
fn test_cologne_german_names_match() {
    let (engine, _tmp) = create_test_engine();

    // Mueller and Miller should match (both variations of Müller)
    let mueller = execute_query(&engine, r#"RETURN COLOGNE("Mueller")"#);
    let miller = execute_query(&engine, r#"RETURN COLOGNE("Miller")"#);
    assert_eq!(mueller, miller);
}

#[test]
fn test_cologne_meyer_variants() {
    let (engine, _tmp) = create_test_engine();

    // Meyer, Meier, Maier should produce same code
    let meyer = execute_query(&engine, r#"RETURN COLOGNE("Meyer")"#);
    let meier = execute_query(&engine, r#"RETURN COLOGNE("Meier")"#);
    let maier = execute_query(&engine, r#"RETURN COLOGNE("Maier")"#);
    assert_eq!(meyer, meier);
    assert_eq!(meier, maier);
}

#[test]
fn test_cologne_empty_string() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN COLOGNE("")"#);
    assert_eq!(result, vec![json!("")]);
}

#[test]
fn test_cologne_null() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN COLOGNE(null)"#);
    assert_eq!(result, vec![json!(null)]);
}

#[test]
fn test_cologne_umlaut_handling() {
    let (engine, _tmp) = create_test_engine();

    // ä, ö, ü should be treated like a, o, u
    let result = execute_query(
        &engine,
        r#"RETURN COLOGNE("Schröder") == COLOGNE("Schroder")"#,
    );
    assert_eq!(result, vec![json!(true)]);
}

#[test]
fn test_cologne_eszett_handling() {
    let (engine, _tmp) = create_test_engine();

    // ß should be treated like ss
    let result = execute_query(&engine, r#"RETURN COLOGNE("Groß") == COLOGNE("Gross")"#);
    assert_eq!(result, vec![json!(true)]);
}

// =============================================================================
// CAVERPHONE Tests
// =============================================================================

#[test]
fn test_caverphone_basic() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN CAVERPHONE("Smith")"#);
    assert_eq!(result.len(), 1);
    // Caverphone returns 10-character codes
    if let serde_json::Value::String(s) = &result[0] {
        assert_eq!(s.len(), 10);
    } else {
        panic!("Expected string");
    }
}

#[test]
fn test_caverphone_empty_string() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN CAVERPHONE("")"#);
    assert_eq!(result, vec![json!("1111111111")]);
}

#[test]
fn test_caverphone_null() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN CAVERPHONE(null)"#);
    assert_eq!(result, vec![json!(null)]);
}

#[test]
fn test_caverphone_lee_leigh() {
    let (engine, _tmp) = create_test_engine();

    // Lee and Leigh should match
    let lee = execute_query(&engine, r#"RETURN CAVERPHONE("Lee")"#);
    let leigh = execute_query(&engine, r#"RETURN CAVERPHONE("Leigh")"#);
    assert_eq!(lee, leigh);
}

#[test]
fn test_caverphone_stevenson_stephenson() {
    let (engine, _tmp) = create_test_engine();

    // Stevenson and Stephenson should match (ph -> f)
    let stevenson = execute_query(&engine, r#"RETURN CAVERPHONE("Stevenson")"#);
    let stephenson = execute_query(&engine, r#"RETURN CAVERPHONE("Stephenson")"#);
    assert_eq!(stevenson, stephenson);
}

// =============================================================================
// NYSIIS Tests
// =============================================================================

#[test]
fn test_nysiis_basic() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN NYSIIS("Johnson")"#);
    assert_eq!(result.len(), 1);
    assert!(result[0].is_string());
}

#[test]
fn test_nysiis_empty_string() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN NYSIIS("")"#);
    assert_eq!(result, vec![json!("")]);
}

#[test]
fn test_nysiis_null() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN NYSIIS(null)"#);
    assert_eq!(result, vec![json!(null)]);
}

#[test]
fn test_nysiis_mac_prefix() {
    let (engine, _tmp) = create_test_engine();

    // MAC -> MCC transformation
    let macdonald = execute_query(&engine, r#"RETURN NYSIIS("MacDonald")"#);
    let mcdonald = execute_query(&engine, r#"RETURN NYSIIS("McDonald")"#);
    // Both should start with MCC
    assert_eq!(macdonald, mcdonald);
}

#[test]
fn test_nysiis_kn_prefix() {
    let (engine, _tmp) = create_test_engine();

    // KN -> NN transformation
    let knight = execute_query(&engine, r#"RETURN NYSIIS("Knight")"#);
    let night = execute_query(&engine, r#"RETURN NYSIIS("Night")"#);
    assert_eq!(knight, night);
}

#[test]
fn test_nysiis_ph_prefix() {
    let (engine, _tmp) = create_test_engine();

    // PH -> FF transformation
    let phelps = execute_query(&engine, r#"RETURN NYSIIS("Phelps")"#);
    let felps = execute_query(&engine, r#"RETURN NYSIIS("Felps")"#);
    assert_eq!(phelps, felps);
}

// =============================================================================
// Integration Tests with Collections (European)
// =============================================================================

#[test]
fn test_cologne_filter_collection() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("german_names".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine
        .get_collection("german_names")
        .expect("Collection not found");

    coll.insert(json!({"_key": "1", "name": "Müller"})).unwrap();
    coll.insert(json!({"_key": "2", "name": "Mueller"}))
        .unwrap();
    coll.insert(json!({"_key": "3", "name": "Miller"})).unwrap();
    coll.insert(json!({"_key": "4", "name": "Schmidt"}))
        .unwrap();

    let result = execute_query(
        &engine,
        r#"
        FOR doc IN german_names
          FILTER COLOGNE(doc.name) == COLOGNE("Müller")
          SORT doc.name
          RETURN doc.name
    "#,
    );

    // Müller, Mueller, Miller should all match
    assert!(result.len() >= 2);
    assert!(result.contains(&json!("Mueller")));
    assert!(result.contains(&json!("Miller")));
}

#[test]
fn test_nysiis_filter_collection() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("surnames".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine
        .get_collection("surnames")
        .expect("Collection not found");

    coll.insert(json!({"_key": "1", "name": "MacDonald"}))
        .unwrap();
    coll.insert(json!({"_key": "2", "name": "McDonald"}))
        .unwrap();
    coll.insert(json!({"_key": "3", "name": "Smith"})).unwrap();

    let result = execute_query(
        &engine,
        r#"
        FOR doc IN surnames
          FILTER NYSIIS(doc.name) == NYSIIS("MacDonald")
          SORT doc.name
          RETURN doc.name
    "#,
    );

    // MacDonald and McDonald should match
    assert_eq!(result.len(), 2);
}

#[test]
fn test_european_phonetic_in_return() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("test_names".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine
        .get_collection("test_names")
        .expect("Collection not found");

    coll.insert(json!({"_key": "1", "name": "Schröder"}))
        .unwrap();

    let result = execute_query(
        &engine,
        r#"
        FOR doc IN test_names
          RETURN {
            name: doc.name,
            cologne: COLOGNE(doc.name),
            caverphone: CAVERPHONE(doc.name),
            nysiis: NYSIIS(doc.name)
          }
    "#,
    );

    assert_eq!(result.len(), 1);
    let obj = &result[0];
    assert_eq!(obj["name"], json!("Schröder"));
    assert!(obj["cologne"].is_string());
    assert!(obj["caverphone"].is_string());
    assert!(obj["nysiis"].is_string());
}

// =============================================================================
// SOUNDEX with Locale Parameter Tests
// =============================================================================

#[test]
fn test_soundex_locale_default() {
    let (engine, _tmp) = create_test_engine();

    // Without locale parameter, should use English (American Soundex)
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Smith")"#);
    assert_eq!(result, vec![json!("S530")]);
}

#[test]
fn test_soundex_locale_english() {
    let (engine, _tmp) = create_test_engine();

    // Explicit "en" locale should behave same as default
    let default_result = execute_query(&engine, r#"RETURN SOUNDEX("Smith")"#);
    let en_result = execute_query(&engine, r#"RETURN SOUNDEX("Smith", "en")"#);
    assert_eq!(default_result, en_result);
}

#[test]
fn test_soundex_locale_german() {
    let (engine, _tmp) = create_test_engine();

    // German locale should use Cologne Phonetic
    let result = execute_query(
        &engine,
        r#"RETURN SOUNDEX("Müller", "de") == SOUNDEX("Mueller", "de")"#,
    );
    assert_eq!(result, vec![json!(true)]);
}

#[test]
fn test_soundex_locale_german_matches_cologne() {
    let (engine, _tmp) = create_test_engine();

    // SOUNDEX with "de" should produce same result as COLOGNE
    let soundex_de = execute_query(&engine, r#"RETURN SOUNDEX("Meyer", "de")"#);
    let cologne = execute_query(&engine, r#"RETURN COLOGNE("Meyer")"#);
    assert_eq!(soundex_de, cologne);
}

#[test]
fn test_soundex_locale_french() {
    let (engine, _tmp) = create_test_engine();

    // French locale - basic test
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Dupont", "fr")"#);
    assert_eq!(result.len(), 1);
    assert!(result[0].is_string());
}

#[test]
fn test_soundex_locale_french_silent_endings() {
    let (engine, _tmp) = create_test_engine();

    // French names with silent endings should match
    let result = execute_query(
        &engine,
        r#"RETURN SOUNDEX("Dupont", "fr") == SOUNDEX("Dupon", "fr")"#,
    );
    assert_eq!(result, vec![json!(true)]);
}

#[test]
fn test_soundex_locale_french_accents() {
    let (engine, _tmp) = create_test_engine();

    // French names with accents should match non-accented versions
    let result = execute_query(
        &engine,
        r#"RETURN SOUNDEX("Lefèvre", "fr") == SOUNDEX("Lefevre", "fr")"#,
    );
    assert_eq!(result, vec![json!(true)]);
}

#[test]
fn test_soundex_locale_french_eau() {
    let (engine, _tmp) = create_test_engine();

    // French "eau" sound
    let result = execute_query(
        &engine,
        r#"RETURN SOUNDEX("Beaumont", "fr") == SOUNDEX("Bomont", "fr")"#,
    );
    assert_eq!(result, vec![json!(true)]);
}

#[test]
fn test_soundex_locale_french_null() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX(null, "fr")"#);
    assert_eq!(result, vec![json!(null)]);
}

#[test]
fn test_soundex_locale_french_empty() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX("", "fr")"#);
    assert_eq!(result, vec![json!("")]);
}

#[test]
fn test_soundex_locale_filter_collection() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("french_names".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine
        .get_collection("french_names")
        .expect("Collection not found");

    coll.insert(json!({"_key": "1", "name": "Dupont"})).unwrap();
    coll.insert(json!({"_key": "2", "name": "Dupon"})).unwrap();
    coll.insert(json!({"_key": "3", "name": "Martin"})).unwrap();

    let result = execute_query(
        &engine,
        r#"
        FOR doc IN french_names
          FILTER SOUNDEX(doc.name, "fr") == SOUNDEX("Dupont", "fr")
          SORT doc.name
          RETURN doc.name
    "#,
    );

    // Dupont and Dupon should match (silent 't')
    assert!(result.len() >= 2);
    assert!(result.contains(&json!("Dupont")));
    assert!(result.contains(&json!("Dupon")));
}

// =============================================================================
// SOUNDEX Spanish (es) Locale Tests
// =============================================================================

#[test]
fn test_soundex_locale_spanish_basic() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX("García", "es")"#);
    assert_eq!(result.len(), 1);
    assert!(result[0].is_string());
}

#[test]
fn test_soundex_locale_spanish_bv() {
    let (engine, _tmp) = create_test_engine();
    // B and V are equivalent in Spanish
    let result = execute_query(
        &engine,
        r#"RETURN SOUNDEX("Baca", "es") == SOUNDEX("Vaca", "es")"#,
    );
    assert_eq!(result, vec![json!(true)]);
}

#[test]
fn test_soundex_locale_spanish_h_silent() {
    let (engine, _tmp) = create_test_engine();
    // H is always silent in Spanish
    let result = execute_query(
        &engine,
        r#"RETURN SOUNDEX("Hernández", "es") == SOUNDEX("Ernandez", "es")"#,
    );
    assert_eq!(result, vec![json!(true)]);
}

// =============================================================================
// SOUNDEX Italian (it) Locale Tests
// =============================================================================

#[test]
fn test_soundex_locale_italian_basic() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Rossi", "it")"#);
    assert_eq!(result.len(), 1);
    assert!(result[0].is_string());
}

#[test]
fn test_soundex_locale_italian_double_consonants() {
    let (engine, _tmp) = create_test_engine();
    // Double consonants should be reduced in Italian
    let result = execute_query(
        &engine,
        r#"RETURN SOUNDEX("Rossi", "it") == SOUNDEX("Rosi", "it")"#,
    );
    assert_eq!(result, vec![json!(true)]);
}

#[test]
fn test_soundex_locale_italian_gn() {
    let (engine, _tmp) = create_test_engine();
    // GN digraph (like in "gnocchi")
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Agnelli", "it")"#);
    assert!(result[0].is_string());
}

// =============================================================================
// SOUNDEX Portuguese (pt) Locale Tests
// =============================================================================

#[test]
fn test_soundex_locale_portuguese_basic() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Silva", "pt")"#);
    assert_eq!(result.len(), 1);
    assert!(result[0].is_string());
}

#[test]
fn test_soundex_locale_portuguese_cedilha() {
    let (engine, _tmp) = create_test_engine();
    // Ç should be equivalent to S
    let result = execute_query(
        &engine,
        r#"RETURN SOUNDEX("Gonçalves", "pt") == SOUNDEX("Gonsalves", "pt")"#,
    );
    assert_eq!(result, vec![json!(true)]);
}

#[test]
fn test_soundex_locale_portuguese_nh() {
    let (engine, _tmp) = create_test_engine();
    // NH digraph
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Carvalho", "pt")"#);
    assert!(result[0].is_string());
}

// =============================================================================
// SOUNDEX Dutch (nl) Locale Tests
// =============================================================================

#[test]
fn test_soundex_locale_dutch_basic() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Jansen", "nl")"#);
    assert_eq!(result.len(), 1);
    assert!(result[0].is_string());
}

#[test]
fn test_soundex_locale_dutch_ij() {
    let (engine, _tmp) = create_test_engine();
    // IJ is a single letter in Dutch
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Dijkstra", "nl")"#);
    assert!(result[0].is_string());
}

#[test]
fn test_soundex_locale_dutch_wv() {
    let (engine, _tmp) = create_test_engine();
    // W and V are often interchangeable in Dutch
    let result = execute_query(
        &engine,
        r#"RETURN SOUNDEX("Willemsen", "nl") == SOUNDEX("Villemsen", "nl")"#,
    );
    assert_eq!(result, vec![json!(true)]);
}

// =============================================================================
// SOUNDEX Greek (el) Locale Tests
// =============================================================================

#[test]
fn test_soundex_locale_greek_basic() {
    let (engine, _tmp) = create_test_engine();
    // Greek name in Greek alphabet
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Παπαδόπουλος", "el")"#);
    assert_eq!(result.len(), 1);
    assert!(result[0].is_string());
}

#[test]
fn test_soundex_locale_greek_latin() {
    let (engine, _tmp) = create_test_engine();
    // Greek name already in Latin letters should also work
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Papadopoulos", "el")"#);
    assert_eq!(result.len(), 1);
    assert!(result[0].is_string());
}

#[test]
fn test_soundex_locale_greek_mp_b() {
    let (engine, _tmp) = create_test_engine();
    // ΜΠ sounds like B in Greek
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Μπάρμπας", "el")"#);
    assert!(result[0].is_string());
}

#[test]
fn test_soundex_locale_greek_nt_d() {
    let (engine, _tmp) = create_test_engine();
    // ΝΤ sounds like D in Greek
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Ντίνος", "el")"#);
    assert!(result[0].is_string());
}

#[test]
fn test_soundex_locale_greek_common_names() {
    let (engine, _tmp) = create_test_engine();
    // Test common Greek surnames
    let result1 = execute_query(&engine, r#"RETURN SOUNDEX("Νικολάου", "el")"#);
    let result2 = execute_query(&engine, r#"RETURN SOUNDEX("Γεωργίου", "el")"#);
    assert!(result1[0].is_string());
    assert!(result2[0].is_string());
}

#[test]
fn test_soundex_locale_greek_empty() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX("", "el")"#);
    assert_eq!(result, vec![json!("")]);
}

#[test]
fn test_soundex_locale_greek_null() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX(null, "el")"#);
    assert_eq!(result, vec![json!(null)]);
}

// =============================================================================
// SOUNDEX Japanese (ja) Locale Tests
// =============================================================================

#[test]
fn test_soundex_locale_japanese_hiragana() {
    let (engine, _tmp) = create_test_engine();
    // Yamada in hiragana
    let result = execute_query(&engine, r#"RETURN SOUNDEX("やまだ", "ja")"#);
    assert_eq!(result.len(), 1);
    assert!(result[0].is_string());
}

#[test]
fn test_soundex_locale_japanese_katakana() {
    let (engine, _tmp) = create_test_engine();
    // Yamada in katakana
    let result = execute_query(&engine, r#"RETURN SOUNDEX("ヤマダ", "ja")"#);
    assert_eq!(result.len(), 1);
    assert!(result[0].is_string());
}

#[test]
fn test_soundex_locale_japanese_hiragana_katakana_match() {
    let (engine, _tmp) = create_test_engine();
    // Hiragana and Katakana versions should match
    let result = execute_query(
        &engine,
        r#"RETURN SOUNDEX("すずき", "ja") == SOUNDEX("スズキ", "ja")"#,
    );
    assert_eq!(result, vec![json!(true)]);
}

#[test]
fn test_soundex_locale_japanese_romaji() {
    let (engine, _tmp) = create_test_engine();
    // Romaji input should also work
    let result = execute_query(&engine, r#"RETURN SOUNDEX("Tanaka", "ja")"#);
    assert_eq!(result.len(), 1);
    assert!(result[0].is_string());
}

#[test]
fn test_soundex_locale_japanese_common_names() {
    let (engine, _tmp) = create_test_engine();
    // Test common Japanese surnames
    let result1 = execute_query(&engine, r#"RETURN SOUNDEX("たなか", "ja")"#);
    let result2 = execute_query(&engine, r#"RETURN SOUNDEX("さとう", "ja")"#);
    assert!(result1[0].is_string());
    assert!(result2[0].is_string());
}

#[test]
fn test_soundex_locale_japanese_voiced() {
    let (engine, _tmp) = create_test_engine();
    // Test voiced consonants (dakuten)
    let result = execute_query(&engine, r#"RETURN SOUNDEX("ごとう", "ja")"#); // Gotou
    assert!(result[0].is_string());
}

#[test]
fn test_soundex_locale_japanese_empty() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX("", "ja")"#);
    assert_eq!(result, vec![json!("")]);
}

#[test]
fn test_soundex_locale_japanese_null() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN SOUNDEX(null, "ja")"#);
    assert_eq!(result, vec![json!(null)]);
}
