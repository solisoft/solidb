//! Built-in function modules for SDBQL.
//!
//! This module organizes built-in functions into categories:
//! - type_check: IS_ARRAY, IS_STRING, IS_NULL, etc.
//! - string: UPPER, LOWER, TRIM, SPLIT, etc.
//! - array: FIRST, LAST, SORTED, UNIQUE, etc.
//! - math: FLOOR, CEIL, ROUND, SIN, COS, etc.
//! - crypto: MD5, SHA256, BASE64, ARGON2, etc.
//! - datetime: NOW, DATE_*, TIME_BUCKET, etc.
//! - geo: DISTANCE, GEO_DISTANCE, etc.
//! - json: JSON_PARSE, JSON_STRINGIFY
//! - misc: UUID, TYPEOF, COALESCE, etc.

pub mod approx;
pub mod array;
pub mod crypto;
pub mod datetime;
pub mod geo;
pub mod json;
pub mod math;
pub mod misc;
pub mod string;
pub mod timeseries;
pub mod type_check;

use crate::error::DbResult;
use once_cell::sync::Lazy;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;

/// Upper-case a function name only when it is not already: the parser hands
/// the evaluator upper-case names, so the common case allocates nothing.
#[inline]
pub fn upper_name(name: &str) -> Cow<'_, str> {
    if name.bytes().any(|b| b.is_ascii_lowercase()) {
        Cow::Owned(name.to_ascii_uppercase())
    } else {
        Cow::Borrowed(name)
    }
}

/// Which implementation owns a function name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    TypeCheck,
    Datetime,
    Timeseries,
    MatchSeq,
    Semantic,
    Approx,
    Geo,
    Json,
    Crypto,
    Math,
    Str,
    Array,
    Misc,
    PhoneticId,
    PhoneticString,
    PhoneticAlgo,
    /// Needs the executor (collections, principal, context):
    /// `QueryExecutor::call_context_function` in `evaluate.rs`.
    Context,
}

// Name lists per module, in the precedence order of the historical dispatch
// cascade (phonetic slot first, then the builtins prefix routes, then
// string → array → math → misc). The first route listed for a name wins.
//
// The table is an accelerator, not the source of truth: a name missing here
// still resolves through the full cascade, and a routed module answering
// `Ok(None)` (e.g. string CONTAINS on an array) falls back to it too.

const PHONETIC_ID: &[&str] = &["UUID", "UUID_V4", "UUID_V7", "ULID", "NANOID"];
const PHONETIC_ALGO: &[&str] = &[
    "SOUNDEX",
    "SOUNDEX_FR",
    "SOUNDEX_ES",
    "SOUNDEX_IT",
    "SOUNDEX_PT",
    "SOUNDEX_NL",
    "SOUNDEX_EL",
    "SOUNDEX_JA",
    "METAPHONE",
    "DOUBLE_METAPHONE",
    "COLOGNE_PHONETIC",
    "COLOGNE",
    "CAVERPHONE",
    "NYSIIS",
];
const PHONETIC_STRING: &[&str] = &[
    "HIGHLIGHT",
    "SLUGIFY",
    "SANITIZE",
    "IS_EMAIL",
    "IS_URL",
    "IS_UUID",
    "IS_BLANK",
];
const DATETIME: &[&str] = &[
    "NOW",
    "DATE_NOW",
    "NOW_ISO",
    "DATE_NOW_ISO",
    "UUIDV4",
    "UUIDV7",
    "DATE_YEAR",
    "DATE_MONTH",
    "DATE_DAY",
    "DATE_HOUR",
    "DATE_MINUTE",
    "DATE_SECOND",
    "DATE_MILLISECOND",
    "DATE_DAYOFWEEK",
    "DATE_DAYOFYEAR",
    "DATE_WEEK",
    "DATE_ISOWEEK",
    "DATE_ISOWEEKYEAR",
    "DATE_QUARTER",
    "DATE_LEAPYEAR",
    "DATE_COMPARE",
    "DATE_ISO8601",
    "DATE_TIMESTAMP",
    "DATE_FORMAT",
    "DATE_TRUNC",
    "DATE_ROUND",
    "DATE_DAYS_IN_MONTH",
    "DATE_ADD",
    "DATE_SUBTRACT",
    "DATE_SUB",
    "DATE_DIFF",
    "TIME_BUCKET",
    "HUMAN_TIME",
];
const TYPE_CHECK: &[&str] = &[
    "IS_ARRAY",
    "IS_LIST",
    "IS_BOOL",
    "IS_BOOLEAN",
    "IS_NUMBER",
    "IS_NUMERIC",
    "IS_INTEGER",
    "IS_INT",
    "IS_STRING",
    "IS_NULL",
    "IS_OBJECT",
    "IS_DOCUMENT",
    "IS_EMPTY",
    "IS_DATE",
    "IS_KEY",
    "IS_SAME_COLLECTION",
];
const TIMESERIES: &[&str] = &["DELTA", "RATE", "FILL", "RESAMPLE"];
const APPROX: &[&str] = &[
    "APPROX_COUNT_DISTINCT",
    "APPROX_PERCENTILE",
    "APPROX_TOP_K",
    "SKETCH_MERGE",
    "MINHASH",
    "MINHASH_COUNT",
    "MINHASH_ERROR",
];
const GEO: &[&str] = &[
    "DISTANCE",
    "GEO_DISTANCE",
    "GEO_EQUALS",
    "GEO_WITHIN",
    "GEO_POINT",
    "GEO_LINESTRING",
    "GEO_POLYGON",
    "GEO_MULTIPOINT",
    "GEO_MULTILINESTRING",
    "GEO_MULTIPOLYGON",
    "GEO_CONTAINS",
    "GEO_INTERSECTS",
    "GEO_IN_RANGE",
    "GEO_AREA",
];
const JSON_FNS: &[&str] = &[
    "JSON_PARSE",
    "PARSE_JSON",
    "JSON_STRINGIFY",
    "TO_JSON",
    "JSON_STRINGIFY_PRETTY",
    "JSON_POINTER",
];
const CRYPTO: &[&str] = &[
    "MD5",
    "SHA256",
    "SHA512",
    "BASE64_ENCODE",
    "TO_BASE64",
    "BASE64_DECODE",
    "FROM_BASE64",
    "HEX_ENCODE",
    "TO_HEX",
    "HEX_DECODE",
    "FROM_HEX",
    "ARGON2_HASH",
    "ARGON2_VERIFY",
    "HMAC_SHA256",
];
const STRING: &[&str] = &[
    "NGRAM_SIMILARITY",
    "TOKENS",
    "PHRASE",
    "BOOST",
    "UPPER",
    "TO_UPPER",
    "TOUPPER",
    "LOWER",
    "TO_LOWER",
    "TOLOWER",
    "TRIM",
    "LTRIM",
    "RTRIM",
    "CONCAT",
    "CONCAT_WS",
    "CONCAT_SEPARATOR",
    "JOIN",
    "CONTAINS",
    "STARTS_WITH",
    "ENDS_WITH",
    "SPLIT",
    "SUBSTRING",
    "SUBSTR",
    "REPLACE",
    "SUBSTITUTE",
    "LEFT",
    "RIGHT",
    "CHAR_LENGTH",
    "CHARACTER_LENGTH",
    "BYTE_LENGTH",
    "REVERSE",
    "FIND_FIRST",
    "FIND",
    "FIND_LAST",
    "RFIND",
    "LIKE",
    "REGEX_TEST",
    "REGEX_REPLACE",
    "REGEX_MATCHES",
    "REGEX_SPLIT",
    "REPEAT",
    "PAD_LEFT",
    "LPAD",
    "PAD_RIGHT",
    "RPAD",
    "CAPITALIZE",
    "TITLE_CASE",
    "INITCAP",
    "WORD_COUNT",
    "TRUNCATE_TEXT",
    "MASK",
    "RANDOM_TOKEN",
    "ENCODE_URI",
    "URL_ENCODE",
    "ENCODE_URI_COMPONENT",
    "DECODE_URI",
    "URL_DECODE",
    "DECODE_URI_COMPONENT",
];
const ARRAY: &[&str] = &[
    "FIRST",
    "LAST",
    "REVERSE",
    "SORTED",
    "SORT",
    "SORTED_DESC",
    "UNIQUE",
    "FLATTEN",
    "PUSH",
    "POP",
    "SLICE",
    "POSITION",
    "INDEX_OF",
    "NTH",
    "CONTAINS",
    "CONTAINS_ARRAY",
    "TAKE",
    "DROP",
    "CHUNK",
    "ZIP",
    "ZIP_OBJECT",
    "COUNT",
    "OUTERSECTION",
    "SYMDIFF",
    "LENGTH",
    "APPEND",
    "SHIFT",
    "UNSHIFT",
    "UNION",
    "INTERSECTION",
    "MINUS",
    "DIFFERENCE",
];
const MATH: &[&str] = &[
    "FLOOR",
    "CEIL",
    "CEILING",
    "ROUND",
    "ABS",
    "SQRT",
    "POW",
    "POWER",
    "LOG",
    "LN",
    "LOG10",
    "LOG2",
    "EXP",
    "SIN",
    "COS",
    "TAN",
    "ASIN",
    "ACOS",
    "ATAN",
    "ATAN2",
    "DEGREES",
    "RADIANS",
    "PI",
    "E",
    "MOD",
    "BIT_AND",
    "BIT_OR",
    "BIT_XOR",
    "BIT_NEGATE",
    "BIT_NOT",
    "BIT_SHIFT_LEFT",
    "BIT_SHIFT_RIGHT",
    "CLAMP",
    "MIN",
    "MAX",
    "SUM",
    "AVG",
    "AVERAGE",
    "RAND",
    "RANDOM",
    "RANDOM_INT",
    "RAND_INT",
    "MEDIAN",
    "PERCENTILE",
    "QUANTILE",
    "VARIANCE",
    "VAR_POP",
    "VAR_SAMP",
    "STDDEV",
    "STDDEV_POP",
    "STDDEV_SAMP",
    "STDDEV_POPULATION",
    "COUNT_DISTINCT",
    "COUNT_UNIQUE",
    "UNIQUE_COUNT",
];
const MISC: &[&str] = &[
    "UUID",
    "UUID_V4",
    "UUID_V7",
    "TYPEOF",
    "TYPE_OF",
    "TYPENAME",
    "COALESCE",
    "NOT_NULL",
    "NULLIF",
    "ASSERT",
    "RANGE",
    "TO_NUMBER",
    "TO_NUM",
    "TO_STRING",
    "TO_STR",
    "TO_BOOL",
    "TO_BOOLEAN",
    "TO_ARRAY",
    "TO_LIST",
    "IF",
    "ATTRIBUTES",
    "KEYS",
    "VALUES",
    "KEEP",
    "UNSET",
    "REDACT",
    "PARSE_IDENTIFIER",
    "PARSE_COLLECTION",
    "PARSE_KEY",
    "UNSET_RECURSIVE",
    "KEEP_RECURSIVE",
    "GET",
    "DEEP_MERGE",
    "ENTRIES",
    "FROM_ENTRIES",
    "HAS",
];
/// Functions implemented on the executor (`evaluate.rs`). None of these
/// names is handled by a value-only module, so routing them first skips the
/// whole builtin cascade (DOCUMENT and MERGE used to miss every module).
pub const CONTEXT: &[&str] = &[
    "VECTOR_INDEX_STATS",
    "VECTOR_SIMILARITY",
    "VECTOR_NORMALIZE",
    "VECTOR_DISTANCE",
    "FULLTEXT",
    "SAMPLE",
    "DOCUMENT",
    "LEVENSHTEIN",
    "LEVENSHTEIN_DISTANCE",
    "LEVENSHTEIN_MATCH",
    "NGRAM_MATCH",
    "IN_RANGE",
    "EXISTS",
    "SIMILARITY",
    "FUZZY_MATCH",
    "BM25",
    "MERGE",
    "COLLECTION_COUNT",
    "HYBRID_SEARCH",
    "VECTOR_SEARCH",
    "NEIGHBORS",
    "GRAPH_RAG",
    "COMMUNITY_SEARCH",
    "PAGERANK",
    "DEGREE_CENTRALITY",
    "RERANK",
    "RAG_PIPELINE",
    "DOC_AS_OF",
    "DOC_HISTORY",
    "SNAPSHOT_DIFF",
    "CURRENT_USER",
    "CURRENT_ROLES",
    "CURRENT_DATABASE",
    "CAN",
    "CREATE_GRAPH",
    "DROP_GRAPH",
    "GRAPH_INFO",
    "CREATE_VIEW",
    "DROP_VIEW",
    "SEARCH_INDEX",
    "ROW_POLICY",
    "EMBED",
    "EXTRACT",
    "CITE",
    "GROUNDED",
    "SEARCH_SCORE",
    "APPLY",
    "CALL",
];

static ROUTES: Lazy<HashMap<&'static str, Route>> = Lazy::new(|| {
    let groups: &[(&[&str], Route)] = &[
        // phonetic slot (was tried before builtins)
        (DATETIME, Route::Datetime),
        (PHONETIC_ID, Route::PhoneticId),
        (PHONETIC_ALGO, Route::PhoneticAlgo),
        (PHONETIC_STRING, Route::PhoneticString),
        // builtins prefix routes
        (TYPE_CHECK, Route::TypeCheck),
        (TIMESERIES, Route::Timeseries),
        (&["MATCH_SEQ"], Route::MatchSeq),
        (&["SEMANTIC"], Route::Semantic),
        (APPROX, Route::Approx),
        (GEO, Route::Geo),
        (JSON_FNS, Route::Json),
        (CRYPTO, Route::Crypto),
        // builtins cascade
        (STRING, Route::Str),
        (ARRAY, Route::Array),
        (MATH, Route::Math),
        (MISC, Route::Misc),
        (CONTEXT, Route::Context),
    ];
    let mut map = HashMap::with_capacity(512);
    for (names, route) in groups {
        for name in *names {
            map.entry(*name).or_insert(*route);
        }
    }
    map
});

/// The implementation that owns `name` (already upper-case), if known.
#[inline]
pub fn route(name: &str) -> Option<Route> {
    ROUTES.get(name).copied()
}

/// Call the module a route points at. `Ok(None)` means that module declined
/// (the caller then falls back to [`evaluate_value_function`]).
pub fn call_route(route: Route, name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    use crate::sdbql::executor::phonetic;
    match route {
        Route::TypeCheck => type_check::evaluate(name, args),
        Route::Datetime => datetime::evaluate(name, args),
        Route::Timeseries => timeseries::evaluate(name, args),
        Route::MatchSeq => Ok(Some(match_seq(args)?)),
        Route::Semantic => Ok(Some(semantic(args)?)),
        Route::Approx => approx::evaluate(name, args),
        Route::Geo => geo::evaluate(name, args),
        Route::Json => json::evaluate(name, args),
        Route::Crypto => crypto::evaluate(name, args),
        Route::Math => math::evaluate(name, args),
        Route::Str => string::evaluate(name, args),
        Route::Array => array::evaluate(name, args),
        Route::Misc => misc::evaluate(name, args),
        Route::PhoneticId => phonetic::id::evaluate(name, args),
        Route::PhoneticString => phonetic::string::evaluate(name, args),
        Route::PhoneticAlgo => phonetic::evaluate(name, args),
        Route::Context => Ok(None),
    }
}

/// Every value-only function (phonetic slot, then builtins), by upper-case
/// name: the routing table first, then the full historical cascade.
pub fn evaluate_value_function(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    if let Some(r) = route(name) {
        if let Some(v) = call_route(r, name, args)? {
            return Ok(Some(v));
        }
        if r == Route::Context {
            return Ok(None);
        }
    }
    evaluate_unrouted(name, args)
}

/// The slow path behind the routing table: the phonetic slot, then the
/// builtins prefix routes and module cascade.
pub fn evaluate_unrouted(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    if let Some(v) = crate::sdbql::executor::phonetic::evaluate(name, args)? {
        return Ok(Some(v));
    }
    evaluate_cascade(name, args)
}

/// Try to evaluate a function using the built-in modules.
/// Returns Ok(Some(value)) if the function was handled,
/// Ok(None) if the function is not a built-in,
/// or Err if there was an error.
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    let name_upper = upper_name(name);
    let name: &str = &name_upper;
    if let Some(r) = route(name) {
        // Phonetic-slot routes belong to `phonetic::evaluate`, not here.
        if !matches!(
            r,
            Route::PhoneticId | Route::PhoneticString | Route::PhoneticAlgo | Route::Context
        ) {
            if let Some(v) = call_route(r, name, args)? {
                return Ok(Some(v));
            }
        }
    }
    evaluate_cascade(name, args)
}

/// The historical prefix-then-cascade dispatch, for names the table does not
/// know (or whose routed module declined).
fn evaluate_cascade(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    // One module per prefix so a string function does not scan DATE_*/GEO_*.
    if name.starts_with("IS_") {
        return type_check::evaluate(name, args);
    }
    if name.starts_with("DATE_")
        || name.starts_with("NOW")
        || name == "TIME_BUCKET"
        || name == "HUMAN_TIME"
        || name == "UUIDV4"
        || name == "UUIDV7"
    {
        return datetime::evaluate(name, args);
    }
    if matches!(
        name,
        "DELTA" | "RATE" | "FILL" | "RESAMPLE" | "MATCH_SEQ" | "SEMANTIC"
    ) {
        if name == "MATCH_SEQ" {
            return Ok(Some(match_seq(args)?));
        }
        if name == "SEMANTIC" {
            return Ok(Some(semantic(args)?));
        }
        return timeseries::evaluate(name, args);
    }
    if name.starts_with("APPROX_") || name == "SKETCH_MERGE" || name.starts_with("MINHASH") {
        return approx::evaluate(name, args);
    }
    if name.starts_with("GEO_") || name == "DISTANCE" {
        return geo::evaluate(name, args);
    }
    if name.starts_with("JSON_") || name == "PARSE_JSON" || name == "TO_JSON" {
        return json::evaluate(name, args);
    }
    if CRYPTO.contains(&name) {
        return crypto::evaluate(name, args);
    }
    if name.starts_with("BIT_") {
        return math::evaluate(name, args);
    }

    if let Some(v) = string::evaluate(name, args)? {
        return Ok(Some(v));
    }
    if let Some(v) = array::evaluate(name, args)? {
        return Ok(Some(v));
    }
    if let Some(v) = math::evaluate(name, args)? {
        return Ok(Some(v));
    }
    misc::evaluate(name, args)
}

fn semantic(args: &[Value]) -> DbResult<Value> {
    if args.len() < 2 {
        return Err(crate::error::DbError::ExecutionError(
            "SEMANTIC requires doc, query, [options]".to_string(),
        ));
    }
    let field = args
        .get(2)
        .and_then(|o| o.get("field"))
        .and_then(Value::as_str)
        .unwrap_or("body");
    let text = match &args[0] {
        Value::String(s) => s.clone(),
        Value::Object(o) => o
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    };
    let q = args[1].as_str().unwrap_or("");
    let score = crate::sdbql::executor::helpers::trigram_similarity(&text, q);
    Ok(serde_json::json!({
        "score": score,
        "match": score >= 0.45,
        "field": field
    }))
}

fn match_seq(args: &[Value]) -> DbResult<Value> {
    if args.len() != 3 {
        return Err(crate::error::DbError::ExecutionError(
            "MATCH_SEQ requires events, key_field, steps".to_string(),
        ));
    }
    let events = args[0].as_array().ok_or_else(|| {
        crate::error::DbError::ExecutionError("MATCH_SEQ: events must be an array".to_string())
    })?;
    let key_field = args[1].as_str().ok_or_else(|| {
        crate::error::DbError::ExecutionError("MATCH_SEQ: key_field must be a string".to_string())
    })?;
    let steps = args[2].as_array().ok_or_else(|| {
        crate::error::DbError::ExecutionError("MATCH_SEQ: steps must be an array".to_string())
    })?;
    if steps.is_empty() {
        return Ok(Value::Array(vec![]));
    }

    use std::collections::HashMap;
    let mut by_key: HashMap<String, Vec<&Value>> = HashMap::new();
    for ev in events {
        let k = ev
            .get(key_field)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".into());
        by_key.entry(k).or_default().push(ev);
    }

    let mut matches = Vec::new();
    for (key, mut evs) in by_key {
        evs.sort_by_key(|e| {
            e.get("ts")
                .or_else(|| e.get("t"))
                .or_else(|| e.get("time"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
        });
        if let Some(hit) = match_one_key(&evs, steps) {
            let mut obj = serde_json::Map::new();
            obj.insert("key".into(), Value::String(key));
            obj.insert("steps".into(), hit);
            matches.push(Value::Object(obj));
        }
    }
    Ok(Value::Array(matches))
}

fn event_ts(e: &Value) -> i64 {
    e.get("ts")
        .or_else(|| e.get("t"))
        .or_else(|| e.get("time"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

fn step_matches(ev: &Value, step: &Value) -> bool {
    if let Some(ty) = step.get("type").and_then(Value::as_str) {
        let ev_ty = ev
            .get("type")
            .or_else(|| ev.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if ev_ty != ty {
            return false;
        }
    }
    if let Some(field) = step.get("field").and_then(Value::as_str) {
        if let Some(eq) = step.get("equals") {
            if ev.get(field) != Some(eq) {
                return false;
            }
        }
    }
    true
}

fn match_one_key(evs: &[&Value], steps: &[Value]) -> Option<Value> {
    let mut found: Vec<Value> = Vec::new();
    let mut idx = 0usize;
    let mut last_ts: Option<i64> = None;
    for step in steps {
        let within = step
            .get("within")
            .and_then(Value::as_str)
            .and_then(|s| timeseries::parse_interval_ms(s).ok());
        let mut hit = None;
        while idx < evs.len() {
            let ev = evs[idx];
            idx += 1;
            if !step_matches(ev, step) {
                continue;
            }
            let ts = event_ts(ev);
            if let (Some(prev), Some(w)) = (last_ts, within) {
                if ts - prev > w {
                    return None;
                }
            }
            last_ts = Some(ts);
            let name = step
                .get("as")
                .and_then(Value::as_str)
                .unwrap_or("step")
                .to_string();
            hit = Some(serde_json::json!({ "as": name, "event": ev, "ts": ts }));
            break;
        }
        found.push(hit?);
    }
    Some(Value::Array(found))
}
