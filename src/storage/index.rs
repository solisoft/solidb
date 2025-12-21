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
    /// TTL index - automatic document expiration based on timestamp field
    TTL,
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

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut matrix = vec![vec![0usize; b_len + 1]; a_len + 1];

    for i in 0..=a_len {
        matrix[i][0] = i;
    }
    for j in 0..=b_len {
        matrix[0][j] = j;
    }

    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
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

// ==================== BM25 Scoring ====================

/// BM25 parameters
pub const BM25_K1: f64 = 1.5; // Term frequency saturation parameter
pub const BM25_B: f64 = 0.75; // Length normalization parameter

/// Calculate BM25 score for a document given query terms
///
/// # Arguments
/// * `query_terms` - Tokenized query terms
/// * `doc_terms` - Tokenized document terms
/// * `doc_length` - Length of the document (number of terms)
/// * `avg_doc_length` - Average document length in the collection
/// * `total_docs` - Total number of documents in the collection
/// * `term_doc_freq` - Map of term -> number of documents containing that term
pub fn bm25_score(
    query_terms: &[String],
    doc_terms: &[String],
    doc_length: usize,
    avg_doc_length: f64,
    total_docs: usize,
    term_doc_freq: &std::collections::HashMap<String, usize>,
) -> f64 {
    if query_terms.is_empty() || doc_terms.is_empty() || total_docs == 0 {
        return 0.0;
    }

    // Count term frequencies in document
    let mut doc_term_freq: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for term in doc_terms {
        *doc_term_freq.entry(term.clone()).or_insert(0) += 1;
    }

    let mut score = 0.0;

    for query_term in query_terms {
        // Get term frequency in document
        let tf = *doc_term_freq.get(query_term).unwrap_or(&0) as f64;

        if tf == 0.0 {
            continue; // Term not in document
        }

        // Calculate IDF
        let df = *term_doc_freq.get(query_term).unwrap_or(&0) as f64;
        let idf = calculate_idf(total_docs, df as usize);

        // Calculate BM25 component for this term
        let numerator = tf * (BM25_K1 + 1.0);
        let denominator =
            tf + BM25_K1 * (1.0 - BM25_B + BM25_B * (doc_length as f64 / avg_doc_length));

        score += idf * (numerator / denominator);
    }

    score
}

/// Calculate Inverse Document Frequency (IDF)
///
/// # Arguments
/// * `total_docs` - Total number of documents in the collection
/// * `doc_freq` - Number of documents containing the term
///
/// # Returns
/// IDF score using the formula: log((N - df + 0.5) / (df + 0.5))
pub fn calculate_idf(total_docs: usize, doc_freq: usize) -> f64 {
    if total_docs == 0 || doc_freq == 0 {
        return 0.0;
    }

    let n = total_docs as f64;
    let df = doc_freq as f64;

    // BM25 IDF formula
    ((n - df + 0.5) / (df + 0.5)).ln()
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

/// TTL index metadata stored in RocksDB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtlIndex {
    /// Index name
    pub name: String,
    /// Timestamp field path (must contain Unix timestamp in seconds)
    pub field: String,
    /// Documents expire after this many seconds from the timestamp value
    pub expire_after_seconds: u64,
}

impl TtlIndex {
    /// Create a new TTL index
    pub fn new(name: String, field: String, expire_after_seconds: u64) -> Self {
        Self {
            name,
            field,
            expire_after_seconds,
        }
    }
}

/// TTL index statistics
#[derive(Debug, Clone, Serialize)]
pub struct TtlIndexStats {
    pub name: String,
    pub field: String,
    pub expire_after_seconds: u64,
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
