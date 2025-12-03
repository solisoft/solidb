use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

/// Type of index
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IndexType {
    /// Hash index - fast equality lookups (==)
    Hash,
    /// Persistent index - range queries and sorting (>, <, >=, <=)
    Persistent,
    /// Fulltext index - n-gram based text search with fuzzy matching
    Fulltext,
}

// ==================== N-gram Utilities ====================

/// Default n-gram size for fulltext indexing
pub const NGRAM_SIZE: usize = 3;

/// Generate n-grams from a string
pub fn generate_ngrams(text: &str, n: usize) -> Vec<String> {
    let normalized = normalize_text(text);
    if normalized.len() < n {
        return vec![normalized];
    }

    normalized
        .chars()
        .collect::<Vec<_>>()
        .windows(n)
        .map(|window| window.iter().collect())
        .collect()
}

/// Normalize text for indexing (lowercase, remove punctuation)
pub fn normalize_text(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Tokenize text into words
pub fn tokenize(text: &str) -> Vec<String> {
    normalize_text(text)
        .split_whitespace()
        .map(|s| s.to_string())
        .collect()
}

/// Calculate Levenshtein distance between two strings
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 { return b_len; }
    if b_len == 0 { return a_len; }

    let mut matrix = vec![vec![0usize; b_len + 1]; a_len + 1];

    for i in 0..=a_len { matrix[i][0] = i; }
    for j in 0..=b_len { matrix[0][j] = j; }

    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            matrix[i][j] = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + cost);
        }
    }

    matrix[a_len][b_len]
}

/// Calculate n-gram similarity (Jaccard coefficient)
pub fn ngram_similarity(ngrams1: &[String], ngrams2: &[String]) -> f64 {
    if ngrams1.is_empty() && ngrams2.is_empty() {
        return 1.0;
    }
    if ngrams1.is_empty() || ngrams2.is_empty() {
        return 0.0;
    }

    let set1: HashSet<_> = ngrams1.iter().collect();
    let set2: HashSet<_> = ngrams2.iter().collect();

    let intersection = set1.intersection(&set2).count();
    let union = set1.union(&set2).count();

    intersection as f64 / union as f64
}

/// Fulltext search result with scoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FulltextMatch {
    pub doc_key: String,
    pub score: f64,
    pub matched_terms: Vec<String>,
}

/// Index metadata stored in RocksDB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    /// Index name
    pub name: String,
    /// Field path being indexed (e.g., "age" or "address.city")
    pub field: String,
    /// Type of index
    pub index_type: IndexType,
    /// Whether index values must be unique
    pub unique: bool,
}

impl Index {
    /// Create a new index
    pub fn new(name: String, field: String, index_type: IndexType, unique: bool) -> Self {
        Self {
            name,
            field,
            index_type,
            unique,
        }
    }
}

/// Index statistics
#[derive(Debug, Clone, Serialize)]
pub struct IndexStats {
    pub name: String,
    pub field: String,
    pub index_type: IndexType,
    pub unique: bool,
    pub unique_values: usize,
    pub indexed_documents: usize,
}

/// Extract a field value from a document
pub fn extract_field_value(doc: &Value, field_path: &str) -> Value {
    let mut current = doc;

    for part in field_path.split('.') {
        match current.get(part) {
            Some(val) => current = val,
            None => return Value::Null,
        }
    }

    current.clone()
}
