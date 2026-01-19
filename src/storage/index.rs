use serde::{Deserialize, Deserializer, Serialize};
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
    /// Bloom Filter index - probabilistic existence check
    Bloom,
    /// Cuckoo Filter index - probabilistic existence check with deletion support
    Cuckoo,
    /// Vector index - approximate nearest neighbor search using HNSW
    Vector,
}

/// Distance metric for vector similarity search
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum VectorMetric {
    /// Cosine similarity (normalized dot product, range 0-1 for normalized vectors)
    #[default]
    Cosine,
    /// Euclidean distance (L2 norm)
    Euclidean,
    /// Dot product (inner product)
    DotProduct,
}

/// Quantization method for vector compression
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum VectorQuantization {
    /// No quantization - full f32 precision (4 bytes/dim)
    #[default]
    None,
    /// Scalar Quantization - u8 per dimension (1 byte/dim), 4x compression
    Scalar,
}

/// Configuration for vector index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorIndexConfig {
    /// Index name
    pub name: String,
    /// Field path containing the vector (must be array of f32/f64)
    pub field: String,
    /// Dimension of vectors (must be consistent across all documents)
    pub dimension: usize,
    /// Distance metric to use
    #[serde(default)]
    pub metric: VectorMetric,
    /// HNSW M parameter - max connections per node (default: 16)
    #[serde(default = "default_hnsw_m")]
    pub m: usize,
    /// HNSW ef_construction - build quality parameter (default: 200)
    #[serde(default = "default_ef_construction")]
    pub ef_construction: usize,
    /// Quantization method for storage compression (default: None)
    #[serde(default)]
    pub quantization: VectorQuantization,
}

fn default_hnsw_m() -> usize {
    16
}

fn default_ef_construction() -> usize {
    200
}

impl VectorIndexConfig {
    /// Create a new vector index configuration with defaults
    pub fn new(name: String, field: String, dimension: usize) -> Self {
        Self {
            name,
            field,
            dimension,
            metric: VectorMetric::default(),
            m: default_hnsw_m(),
            ef_construction: default_ef_construction(),
            quantization: VectorQuantization::default(),
        }
    }

    /// Set the distance metric
    pub fn with_metric(mut self, metric: VectorMetric) -> Self {
        self.metric = metric;
        self
    }

    /// Set HNSW M parameter
    pub fn with_m(mut self, m: usize) -> Self {
        self.m = m;
        self
    }

    /// Set HNSW ef_construction parameter
    pub fn with_ef_construction(mut self, ef_construction: usize) -> Self {
        self.ef_construction = ef_construction;
        self
    }

    /// Set quantization method
    pub fn with_quantization(mut self, quantization: VectorQuantization) -> Self {
        self.quantization = quantization;
        self
    }
}

/// Vector index statistics
#[derive(Debug, Clone, Serialize)]
pub struct VectorIndexStats {
    pub name: String,
    pub field: String,
    pub dimension: usize,
    pub metric: VectorMetric,
    pub m: usize,
    pub ef_construction: usize,
    pub indexed_vectors: usize,
    /// Quantization method in use
    pub quantization: VectorQuantization,
    /// Estimated memory usage in bytes
    pub memory_bytes: usize,
    /// Compression ratio (1.0 = no compression, 4.0 = 4x compression)
    pub compression_ratio: f32,
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

    for (i, row) in matrix.iter_mut().enumerate().take(a_len + 1) {
        row[0] = i;
    }
    for (j, cell) in matrix[0].iter_mut().enumerate().take(b_len + 1) {
        *cell = j;
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

/// Custom deserializer for backward compatibility with single 'field'
pub fn deserialize_fields<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum FieldOrFields {
        Single(String),
        Multiple(Vec<String>),
    }

    match FieldOrFields::deserialize(deserializer)? {
        FieldOrFields::Single(s) => Ok(vec![s]),
        FieldOrFields::Multiple(v) => Ok(v),
    }
}

/// Index metadata stored in RocksDB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    /// Index name
    pub name: String,
    /// Field path(s) being indexed (e.g., "age" or ["name", "age"])
    #[serde(alias = "field", deserialize_with = "deserialize_fields")]
    pub fields: Vec<String>,
    /// Type of index
    pub index_type: IndexType,
    /// Whether index values must be unique
    pub unique: bool,
}

impl Index {
    /// Create a new index
    pub fn new(name: String, fields: Vec<String>, index_type: IndexType, unique: bool) -> Self {
        Self {
            name,
            fields,
            index_type,
            unique,
        }
    }
}

/// Index statistics
#[derive(Debug, Clone, Serialize)]
pub struct IndexStats {
    pub name: String,
    pub fields: Vec<String>,
    /// Primary field (for backward compatibility)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_generate_ngrams() {
        let ngrams = generate_ngrams("hello", 3);
        assert!(ngrams.contains(&"hel".to_string()));
        assert!(ngrams.contains(&"ell".to_string()));
        assert!(ngrams.contains(&"llo".to_string()));
    }

    #[test]
    fn test_generate_ngrams_short_string() {
        // When string is shorter than n, returns vec with the normalized string
        let ngrams = generate_ngrams("ab", 3);
        assert_eq!(ngrams.len(), 1);
        assert_eq!(ngrams[0], "ab");
    }

    #[test]
    fn test_normalize_text() {
        assert_eq!(normalize_text("Hello World!"), "hello world");
        assert_eq!(normalize_text("Test123"), "test123");
        assert_eq!(normalize_text("  spaced  "), "spaced");
    }

    #[test]
    fn test_tokenize() {
        let tokens = tokenize("Hello World  Test");
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], "hello");
        assert_eq!(tokens[1], "world");
        assert_eq!(tokens[2], "test");
    }

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
        assert_eq!(levenshtein_distance("abc", "ab"), 1);
        assert_eq!(levenshtein_distance("abc", "adc"), 1);
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn test_ngram_similarity() {
        let ngrams1 = generate_ngrams("hello", 3);
        let ngrams2 = generate_ngrams("hello", 3);
        assert!((ngram_similarity(&ngrams1, &ngrams2) - 1.0).abs() < 1e-10);

        let ngrams3 = generate_ngrams("world", 3);
        let sim = ngram_similarity(&ngrams1, &ngrams3);
        assert!(sim < 1.0);
    }

    #[test]
    fn test_ngram_similarity_empty() {
        // Both empty means identical (returns 1.0)
        let sim = ngram_similarity(&[], &[]);
        assert!((sim - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_calculate_idf() {
        // If term appears in all docs, IDF should be low/negative
        let idf_all = calculate_idf(100, 100);
        let idf_few = calculate_idf(100, 5);

        // Term appearing in fewer docs should have higher IDF
        assert!(idf_few > idf_all);
    }

    #[test]
    fn test_index_type() {
        assert_ne!(IndexType::Hash, IndexType::Persistent);
        assert_ne!(IndexType::Fulltext, IndexType::TTL);
    }

    #[test]
    fn test_index_new() {
        let index = Index::new(
            "idx_name".to_string(),
            vec!["field1".to_string(), "field2".to_string()],
            IndexType::Persistent,
            true,
        );

        assert_eq!(index.name, "idx_name");
        assert_eq!(index.fields.len(), 2);
        assert_eq!(index.index_type, IndexType::Persistent);
        assert!(index.unique);
    }

    #[test]
    fn test_ttl_index_new() {
        let ttl_idx = TtlIndex::new("ttl_idx".to_string(), "expires_at".to_string(), 3600);

        assert_eq!(ttl_idx.name, "ttl_idx");
        assert_eq!(ttl_idx.field, "expires_at");
        assert_eq!(ttl_idx.expire_after_seconds, 3600);
    }

    #[test]
    fn test_extract_field_value_simple() {
        let doc = json!({"name": "Alice", "age": 30});

        assert_eq!(extract_field_value(&doc, "name"), json!("Alice"));
        assert_eq!(extract_field_value(&doc, "age"), json!(30));
        assert_eq!(extract_field_value(&doc, "missing"), Value::Null);
    }

    #[test]
    fn test_extract_field_value_nested() {
        let doc = json!({
            "user": {
                "profile": {
                    "name": "Bob"
                }
            }
        });

        assert_eq!(extract_field_value(&doc, "user.profile.name"), json!("Bob"));
        assert_eq!(extract_field_value(&doc, "user.missing"), Value::Null);
    }

    #[test]
    fn test_fulltext_match() {
        let match_result = FulltextMatch {
            doc_key: "doc1".to_string(),
            score: 2.5,
            matched_terms: vec!["hello".to_string()],
        };

        assert_eq!(match_result.doc_key, "doc1");
        assert!((match_result.score - 2.5).abs() < 1e-10);
    }

    #[test]
    fn test_index_serialization() {
        let index = Index::new(
            "test_idx".to_string(),
            vec!["field".to_string()],
            IndexType::Hash,
            false,
        );

        let json = serde_json::to_string(&index).unwrap();
        assert!(json.contains("test_idx"));
        assert!(json.contains("Hash"));

        let deserialized: Index = serde_json::from_str(&json).unwrap();
        assert_eq!(index.name, deserialized.name);
    }

    #[test]
    fn test_index_stats() {
        let stats = IndexStats {
            name: "idx".to_string(),
            index_type: IndexType::Persistent,
            fields: vec!["field1".to_string()],
            field: "field1".to_string(),
            unique: true,
            unique_values: 500,
            indexed_documents: 1000,
        };

        assert_eq!(stats.indexed_documents, 1000);
        assert_eq!(stats.unique_values, 500);
    }

    #[test]
    fn test_bm25_score_empty() {
        let query_terms: Vec<String> = vec![];
        let doc_terms: Vec<String> = vec![];
        let term_doc_freq = std::collections::HashMap::new();

        let score = bm25_score(&query_terms, &doc_terms, 0, 1.0, 1, &term_doc_freq);
        assert!((score - 0.0).abs() < 1e-10);
    }
}
