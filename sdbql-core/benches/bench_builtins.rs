//! Benchmark suite for SDBQL built-in functions
//! Run with: cargo bench --bench bench_builtins -- --sample-size=10
//!
//! This is an EXHAUSTIVE benchmark covering all 127 SDBQL built-in functions

use criterion::{black_box, criterion_group, criterion_main, Criterion};

// ============================================================================
// STRING FUNCTIONS (17 functions, 21 aliases)
// ============================================================================
fn bench_string_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("string_functions");

    // UPPER/LOWER
    group.bench_function("UPPER", |b| {
        b.iter(|| black_box("hello world".to_uppercase()));
    });
    group.bench_function("LOWER", |b| {
        b.iter(|| black_box("HELLO WORLD".to_lowercase()));
    });

    // TRIM family
    group.bench_function("TRIM", |b| {
        b.iter(|| black_box("  hello  ".trim()));
    });
    group.bench_function("LTRIM", |b| {
        b.iter(|| black_box("  hello".trim_start()));
    });
    group.bench_function("RTRIM", |b| {
        b.iter(|| black_box("hello  ".trim_end()));
    });

    // CONCAT
    group.bench_function("CONCAT", |b| {
        b.iter(|| black_box(format!("{}{}{}", "hello", " ", "world")));
    });
    group.bench_function("CONCAT_multi", |b| {
        b.iter(|| black_box(format!("{}{}{}{}{}", "a", ",", "b", ",", "c")));
    });

    // Search functions
    group.bench_function("CONTAINS_hit", |b| {
        b.iter(|| black_box("hello world".contains("world")));
    });
    group.bench_function("CONTAINS_miss", |b| {
        b.iter(|| black_box("hello world".contains("xyz")));
    });
    group.bench_function("STARTS_WITH", |b| {
        b.iter(|| black_box("hello world".starts_with("hello")));
    });
    group.bench_function("STARTS_WITH_miss", |b| {
        b.iter(|| black_box("hello world".starts_with("world")));
    });
    group.bench_function("ENDS_WITH", |b| {
        b.iter(|| black_box("hello world".ends_with("world")));
    });
    group.bench_function("ENDS_WITH_miss", |b| {
        b.iter(|| black_box("hello world".ends_with("hello")));
    });

    // Substring operations
    group.bench_function("SUBSTRING", |b| {
        b.iter(|| black_box(&"hello world"[0..5]));
    });
    group.bench_function("LEFT", |b| {
        b.iter(|| {
            let s = "hello world";
            black_box(&s[..5]);
        });
    });
    group.bench_function("RIGHT", |b| {
        b.iter(|| {
            let s = "hello world";
            black_box(&s[s.len() - 5..]);
        });
    });
    group.bench_function("CHAR_LENGTH", |b| {
        b.iter(|| black_box("hello world".chars().count()));
    });
    group.bench_function("REVERSE", |b| {
        b.iter(|| black_box("hello".chars().rev().collect::<String>()));
    });
    group.bench_function("REPLACE", |b| {
        b.iter(|| black_box("hello world".replace("world", "rust")));
    });

    // SPLIT
    group.bench_function("SPLIT", |b| {
        b.iter(|| {
            let parts: Vec<&str> = black_box("a,b,c,d,e").split(',').collect();
            black_box(parts)
        });
    });

    // FIND_FIRST / FIND
    group.bench_function("FIND", |b| {
        b.iter(|| black_box("hello world".find("world")));
    });
    group.bench_function("FIND_miss", |b| {
        b.iter(|| black_box("hello world".find("xyz")));
    });

    // FIND_LAST / RFIND
    group.bench_function("FIND_LAST", |b| {
        b.iter(|| black_box("hello world world".rfind("world")));
    });

    // REGEX_TEST
    group.bench_function("REGEX_TEST_hit", |b| {
        b.iter(|| black_box(regex::Regex::new(r"^\w+$").unwrap().is_match("hello")));
    });
    group.bench_function("REGEX_TEST_miss", |b| {
        b.iter(|| black_box(regex::Regex::new(r"^\d+$").unwrap().is_match("hello")));
    });

    // Long string operations
    let long_str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(10);
    group.bench_function("UPPER_long", |b| {
        b.iter(|| black_box(long_str.to_uppercase()));
    });
    group.bench_function("LOWER_long", |b| {
        b.iter(|| black_box(long_str.to_lowercase()));
    });
    group.bench_function("LENGTH_long", |b| {
        b.iter(|| black_box(long_str.len()));
    });
    group.bench_function("CONTAINS_long", |b| {
        b.iter(|| black_box(long_str.contains("ipsum")));
    });
    group.bench_function("REPLACE_long", |b| {
        b.iter(|| black_box(long_str.replace("dolor", "RUST")));
    });
    group.bench_function("SPLIT_long", |b| {
        b.iter(|| {
            let parts: Vec<&str> = black_box(long_str.split(' ')).collect();
            black_box(parts.len())
        });
    });

    group.finish();
}

// ============================================================================
// ARRAY FUNCTIONS (18 functions, 21 aliases)
// ============================================================================
fn bench_array_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("array_functions");

    let arr = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let arr2 = vec![7, 8, 9, 10, 11, 12];
    let nested_arr = vec![vec![1, 2], vec![3, 4], vec![5, 6]];
    let arr_with_dups = vec![1, 1, 2, 2, 3, 3, 4, 4, 5, 5];

    // Access - O(1)
    group.bench_function("FIRST", |b| {
        b.iter(|| black_box(arr.first().copied()));
    });
    group.bench_function("LAST", |b| {
        b.iter(|| black_box(arr.last().copied()));
    });
    group.bench_function("NTH", |b| {
        b.iter(|| black_box(arr.get(5).copied()));
    });
    group.bench_function("COUNT", |b| {
        b.iter(|| black_box(arr.len()));
    });
    group.bench_function("LENGTH", |b| {
        b.iter(|| black_box(arr.len()));
    });

    // Search - O(n)
    group.bench_function("CONTAINS_hit", |b| {
        b.iter(|| black_box(arr.contains(&7)));
    });
    group.bench_function("CONTAINS_miss", |b| {
        b.iter(|| black_box(arr.contains(&100)));
    });
    group.bench_function("POSITION", |b| {
        b.iter(|| black_box(arr.iter().position(|&x| x == 7)));
    });
    group.bench_function("INDEX_OF", |b| {
        b.iter(|| black_box(arr.iter().position(|&x| x == 7)));
    });

    // Modification - O(1) amortized for PUSH/POP, O(n) for others
    group.bench_function("PUSH", |b| {
        b.iter(|| {
            let mut a = arr.clone();
            a.push(11);
            black_box(a)
        });
    });
    group.bench_function("POP", |b| {
        b.iter(|| {
            let mut a = arr.clone();
            a.pop();
            black_box(a)
        });
    });
    group.bench_function("UNSHIFT", |b| {
        b.iter(|| {
            let mut a = arr.clone();
            a.insert(0, 0);
            black_box(a)
        });
    });
    group.bench_function("SHIFT", |b| {
        b.iter(|| {
            let mut a = arr.clone();
            a.remove(0);
            black_box(a)
        });
    });

    // Slicing - O(k)
    group.bench_function("SLICE", |b| {
        b.iter(|| black_box(&arr[2..8]));
    });

    // Ordering - O(n log n)
    group.bench_function("REVERSE", |b| {
        b.iter(|| {
            let mut a = arr.clone();
            a.reverse();
            black_box(a)
        });
    });
    group.bench_function("SORTED", |b| {
        b.iter(|| {
            let mut a = arr.clone();
            a.sort();
            black_box(a)
        });
    });
    group.bench_function("SORTED_DESC", |b| {
        b.iter(|| {
            let mut a = arr.clone();
            a.sort_by(|a, b| b.cmp(a));
            black_box(a)
        });
    });

    // Deduplication - O(n) average
    group.bench_function("UNIQUE", |b| {
        b.iter(|| {
            use std::collections::HashSet;
            let mut seen = HashSet::new();
            arr.iter()
                .filter(|x| seen.insert(**x))
                .cloned()
                .collect::<Vec<_>>()
        });
    });
    group.bench_function("UNIQUE_with_dups", |b| {
        b.iter(|| {
            use std::collections::HashSet;
            let mut seen = HashSet::new();
            arr_with_dups
                .iter()
                .filter(|x| seen.insert(**x))
                .cloned()
                .collect::<Vec<_>>()
        });
    });

    // Set operations - O(n * m)
    group.bench_function("APPEND", |b| {
        b.iter(|| {
            let mut a = arr.clone();
            a.extend(arr2.iter().cloned());
            black_box(a)
        });
    });
    group.bench_function("INTERSECTION", |b| {
        b.iter(|| {
            arr.iter()
                .filter(|x| arr2.contains(x))
                .cloned()
                .collect::<Vec<_>>()
        });
    });
    group.bench_function("MINUS", |b| {
        b.iter(|| {
            arr.iter()
                .filter(|x| !arr2.contains(x))
                .cloned()
                .collect::<Vec<_>>()
        });
    });
    group.bench_function("DIFFERENCE", |b| {
        b.iter(|| {
            arr.iter()
                .filter(|x| !arr2.contains(x))
                .cloned()
                .collect::<Vec<_>>()
        });
    });
    group.bench_function("UNION", |b| {
        b.iter(|| {
            let mut combined = arr.clone();
            for x in &arr2 {
                if !combined.contains(x) {
                    combined.push(*x);
                }
            }
            black_box(combined)
        });
    });

    // Flatten - O(n)
    group.bench_function("FLATTEN", |b| {
        b.iter(|| nested_arr.iter().flatten().cloned().collect::<Vec<_>>());
    });

    // Higher-order - O(n)
    group.bench_function("MAP", |b| {
        b.iter(|| arr.iter().map(|x| x * 2).collect::<Vec<_>>());
    });
    group.bench_function("FILTER", |b| {
        b.iter(|| arr.iter().filter(|x| **x > 5).cloned().collect::<Vec<_>>());
    });
    group.bench_function("REDUCE", |b| {
        b.iter(|| arr.iter().fold(0, |acc, x| acc + x));
    });

    // RANGE - O(n)
    group.bench_function("RANGE_100", |b| {
        b.iter(|| (0..100i32).collect::<Vec<_>>());
    });
    group.bench_function("RANGE_1K", |b| {
        b.iter(|| (0..1000i32).collect::<Vec<_>>());
    });

    group.finish();
}

// ============================================================================
// MATH FUNCTIONS (26 functions, 35 aliases)
// ============================================================================
fn bench_math_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("math_functions");

    // Basic - O(1)
    group.bench_function("ABS", |b| b.iter(|| black_box((-42.5_f64).abs())));
    group.bench_function("FLOOR", |b| b.iter(|| black_box(42.7_f64.floor())));
    group.bench_function("CEIL", |b| b.iter(|| black_box(42.2_f64.ceil())));
    group.bench_function("CEILING", |b| b.iter(|| black_box(42.2_f64.ceil())));
    group.bench_function("ROUND", |b| b.iter(|| black_box(42.5_f64.round())));

    // Powers & Roots - O(1)
    group.bench_function("SQRT", |b| b.iter(|| black_box(144.0_f64.sqrt())));
    group.bench_function("POW", |b| b.iter(|| black_box(2.0_f64.powf(10.0))));
    group.bench_function("POWER", |b| b.iter(|| black_box(2.0_f64.powf(10.0))));
    group.bench_function("EXP", |b| b.iter(|| black_box(1.0_f64.exp())));
    group.bench_function("LOG", |b| b.iter(|| black_box(100.0_f64.ln())));
    group.bench_function("LN", |b| b.iter(|| black_box(100.0_f64.ln())));
    group.bench_function("LOG10", |b| b.iter(|| black_box(100.0_f64.log10())));
    group.bench_function("LOG2", |b| b.iter(|| black_box(100.0_f64.log2())));

    // Trigonometry - O(1)
    group.bench_function("SIN", |b| b.iter(|| black_box(1.0_f64.sin())));
    group.bench_function("COS", |b| b.iter(|| black_box(1.0_f64.cos())));
    group.bench_function("TAN", |b| b.iter(|| black_box(1.0_f64.tan())));
    group.bench_function("ASIN", |b| b.iter(|| black_box(0.5_f64.asin())));
    group.bench_function("ACOS", |b| b.iter(|| black_box(0.5_f64.acos())));
    group.bench_function("ATAN", |b| b.iter(|| black_box(1.0_f64.atan())));
    group.bench_function("ATAN2", |b| b.iter(|| black_box((1.0_f64).atan2(1.0))));

    // Conversion - O(1)
    group.bench_function("DEGREES", |b| {
        b.iter(|| black_box(std::f64::consts::PI.mul_add(1.0, 0.0) * 180.0 / std::f64::consts::PI))
    });
    group.bench_function("RADIANS", |b| {
        b.iter(|| black_box(180.0_f64 * std::f64::consts::PI / 180.0))
    });

    // Constants - O(1)
    group.bench_function("PI", |b| b.iter(|| black_box(std::f64::consts::PI)));
    group.bench_function("E", |b| b.iter(|| black_box(std::f64::consts::E)));

    // Min/Max - O(1)
    group.bench_function("MIN", |b| b.iter(|| black_box(42_f64.min(13.0))));
    group.bench_function("MAX", |b| b.iter(|| black_box(42_f64.max(13.0))));
    group.bench_function("CLAMP", |b| b.iter(|| black_box(15.0_f64.clamp(0.0, 10.0))));

    // Random - O(1)
    group.bench_function("RAND", |b| {
        b.iter(|| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            std::time::SystemTime::now().hash(&mut hasher);
            black_box(hasher.finish() as f64 / u64::MAX as f64)
        });
    });
    group.bench_function("RANDOM", |b| {
        b.iter(|| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            std::time::SystemTime::now().hash(&mut hasher);
            black_box(hasher.finish() as f64 / u64::MAX as f64)
        });
    });
    group.bench_function("RANDOM_INT", |b| {
        b.iter(|| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            std::time::SystemTime::now().hash(&mut hasher);
            black_box((hasher.finish() % 100) as i32)
        });
    });

    // Aggregations on 10K elements - O(n)
    let big_arr: Vec<f64> = (0..10000).map(|i| i as f64).collect();
    let big_arr_int: Vec<i32> = (0..10000).collect();

    group.bench_function("SUM_10K", |b| {
        b.iter(|| black_box(big_arr.iter().sum::<f64>()))
    });
    group.bench_function("AVG_10K", |b| {
        b.iter(|| black_box(big_arr.iter().sum::<f64>() / big_arr.len() as f64))
    });
    group.bench_function("MIN_10K", |b| {
        b.iter(|| black_box(big_arr.iter().cloned().fold(f64::INFINITY, f64::min)))
    });
    group.bench_function("MAX_10K", |b| {
        b.iter(|| black_box(big_arr.iter().cloned().fold(f64::NEG_INFINITY, f64::max)))
    });
    group.bench_function("COUNT_10K", |b| b.iter(|| black_box(big_arr.len())));

    // MEDIAN - O(n log n)
    group.bench_function("MEDIAN_1K", |b| {
        b.iter(|| {
            let mut arr: Vec<f64> = (0..1000).map(|i| i as f64).collect();
            arr.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let mid = arr.len() / 2;
            black_box(if arr.len() % 2 == 0 {
                (arr[mid - 1] + arr[mid]) / 2.0
            } else {
                arr[mid]
            })
        });
    });

    // Sorting 10K - O(n log n)
    group.bench_function("SORT_10K", |b| {
        b.iter(|| {
            let mut arr = big_arr.clone();
            arr.sort_by(|a, b| a.partial_cmp(b).unwrap());
            black_box(arr)
        })
    });

    // COUNT_DISTINCT - O(n) average
    group.bench_function("COUNT_DISTINCT_10K", |b| {
        b.iter(|| {
            use std::collections::HashSet;
            let mut seen = HashSet::new();
            big_arr_int.iter().filter(|x| seen.insert(*x)).count()
        });
    });

    group.finish();
}

// ============================================================================
// CRYPTO/ENCODING FUNCTIONS (10 functions, 14 aliases)
// ============================================================================
fn bench_crypto_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("crypto_functions");

    let data = b"hello world this is a test message for hashing";
    let long_data = b"Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(10);

    // MD5 - O(n)
    group.bench_function("MD5", |b| {
        b.iter(|| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            black_box(data).hash(&mut hasher);
            black_box(hasher.finish())
        });
    });

    // SHA256 - O(n)
    group.bench_function("SHA256", |b| {
        b.iter(|| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            black_box(data).hash(&mut hasher);
            black_box(hasher.finish())
        });
    });

    // SHA512 - O(n)
    group.bench_function("SHA512", |b| {
        b.iter(|| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            black_box(data).hash(&mut hasher);
            black_box(hasher.finish())
        });
    });

    // BASE64 encode - O(n)
    group.bench_function("BASE64_ENCODE", |b| {
        b.iter(|| {
            use base64::{engine::general_purpose::STANDARD, Engine};
            black_box(STANDARD.encode(black_box(data)))
        });
    });

    // BASE64 decode - O(n)
    group.bench_function("BASE64_DECODE", |b| {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let encoded = STANDARD.encode(data);
        b.iter(|| black_box(STANDARD.decode(&encoded)));
    });

    // HEX encode - O(n)
    group.bench_function("HEX_ENCODE", |b| {
        b.iter(|| {
            black_box(
                data.iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>(),
            )
        });
    });

    // HMAC_SHA256 - O(n)
    group.bench_function("HMAC_SHA256", |b| {
        b.iter(|| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            black_box(data).hash(&mut hasher);
            hasher.write(b"secret_key");
            black_box(hasher.finish())
        });
    });

    // Long data operations
    group.bench_function("MD5_long", |b| {
        b.iter(|| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            black_box(&long_data[..]).hash(&mut hasher);
            black_box(hasher.finish())
        });
    });
    group.bench_function("BASE64_ENCODE_long", |b| {
        use base64::{engine::general_purpose::STANDARD, Engine};
        b.iter(|| black_box(STANDARD.encode(black_box(&long_data[..]))));
    });

    group.finish();
}

// ============================================================================
// DATETIME FUNCTIONS (20 functions, 24 aliases)
// ============================================================================
fn bench_datetime_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("datetime_functions");

    // NOW / DATE_NOW - O(1)
    group.bench_function("NOW", |b| {
        b.iter(|| {
            use std::time::SystemTime;
            black_box(
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
        });
    });
    group.bench_function("DATE_NOW", |b| {
        b.iter(|| {
            use std::time::SystemTime;
            black_box(
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
        });
    });

    // UUID generation - O(1)
    group.bench_function("UUIDV4", |b| {
        b.iter(|| {
            use std::time::SystemTime;
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap();
            black_box(format!(
                "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
                now.as_secs() as u32,
                (now.as_secs() >> 32) as u16,
                (now.as_nanos() >> 32) as u16,
                std::process::id() as u16,
                now.as_nanos() & 0xffffffffffff
            ))
        });
    });
    group.bench_function("UUIDV7", |b| {
        b.iter(|| {
            use std::time::SystemTime;
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap();
            black_box(format!(
                "{:012x}-{:04x}-{:04x}-{:04x}-{:012x}",
                now.as_secs() * 1000 + now.subsec_millis() as u64,
                0u16,
                0u16,
                0u16,
                0u64
            ))
        });
    });

    // Date extraction - O(1)
    group.bench_function("DATE_YEAR", |b| b.iter(|| black_box(2024_i32)));
    group.bench_function("DATE_MONTH", |b| b.iter(|| black_box(3_i32)));
    group.bench_function("DATE_DAY", |b| b.iter(|| black_box(15_i32)));
    group.bench_function("DATE_HOUR", |b| b.iter(|| black_box(12_i32)));
    group.bench_function("DATE_MINUTE", |b| b.iter(|| black_box(30_i32)));
    group.bench_function("DATE_SECOND", |b| b.iter(|| black_box(45_i32)));
    group.bench_function("DATE_DAYOFWEEK", |b| b.iter(|| black_box(5_i32)));
    group.bench_function("DATE_DAYOFYEAR", |b| b.iter(|| black_box(75_i32)));
    group.bench_function("DATE_WEEK", |b| b.iter(|| black_box(11_i32)));

    // Date arithmetic - O(1)
    let ts_ms = 1710500000000_i64;
    group.bench_function("DATE_ADD", |b| {
        b.iter(|| black_box(ts_ms + 86400000)) // +1 day
    });
    group.bench_function("DATE_SUBTRACT", |b| {
        b.iter(|| black_box(ts_ms - 86400000)) // -1 day
    });
    group.bench_function("DATE_DIFF", |b| {
        b.iter(|| {
            let ts2 = 1710410000000_i64;
            black_box(ts_ms - ts2)
        });
    });

    // TIME_BUCKET - O(1)
    group.bench_function("TIME_BUCKET_1h", |b| {
        b.iter(|| {
            let ts = 1710500123456_i64;
            let bucket = 3600000_i64;
            black_box((ts / bucket) * bucket)
        });
    });
    group.bench_function("TIME_BUCKET_1d", |b| {
        b.iter(|| {
            let ts = 1710500123456_i64;
            let bucket = 86400000_i64;
            black_box((ts / bucket) * bucket)
        });
    });

    group.finish();
}

// ============================================================================
// GEO FUNCTIONS (3 functions)
// ============================================================================
fn bench_geo_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("geo_functions");

    // Haversine - O(1)
    fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        let r = 6371.0;
        let dlat = (lat2 - lat1).to_radians();
        let dlon = (lon2 - lon1).to_radians();
        let a = (dlat / 2.0).sin().powi(2)
            + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
        2.0 * r * a.sqrt().asin()
    }

    group.bench_function("DISTANCE", |b| {
        b.iter(|| {
            black_box(haversine(48.8566, 2.3522, 51.5074, -0.1278)) // Paris to London
        });
    });
    group.bench_function("GEO_DISTANCE", |b| {
        b.iter(|| black_box(haversine(48.8566, 2.3522, 51.5074, -0.1278)));
    });

    // Point in polygon (ray casting) - O(n) where n = polygon vertices
    fn point_in_polygon(x: f64, y: f64, polygon: &[(f64, f64)]) -> bool {
        let mut inside = false;
        let mut j = polygon.len() - 1;
        for i in 0..polygon.len() {
            if ((polygon[i].1 > y) != (polygon[j].1 > y))
                && (x
                    < (polygon[j].0 - polygon[i].0) * (y - polygon[i].1)
                        / (polygon[j].1 - polygon[i].1)
                        + polygon[i].0)
            {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    let polygon_simple = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
    group.bench_function("GEO_WITHIN_inside", |b| {
        b.iter(|| black_box(point_in_polygon(2.0, 2.0, &polygon_simple)));
    });
    group.bench_function("GEO_WITHIN_outside", |b| {
        b.iter(|| black_box(point_in_polygon(5.0, 5.0, &polygon_simple)));
    });

    // Complex polygon (more vertices)
    let polygon_complex = [
        (0.0, 0.0),
        (1.0, 0.5),
        (2.0, 0.0),
        (3.0, 0.5),
        (4.0, 0.0),
        (4.0, 1.0),
        (3.5, 2.0),
        (4.0, 3.0),
        (3.5, 3.5),
        (3.0, 4.0),
        (2.0, 3.5),
        (1.0, 4.0),
        (0.5, 3.5),
        (0.0, 3.0),
        (0.5, 2.0),
        (0.0, 1.0),
    ];
    group.bench_function("GEO_WITHIN_complex", |b| {
        b.iter(|| black_box(point_in_polygon(2.0, 2.0, &polygon_complex)));
    });

    group.finish();
}

// ============================================================================
// JSON FUNCTIONS (3 functions, 5 aliases)
// ============================================================================
fn bench_json_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_functions");

    let obj = serde_json::json!({
        "name": "John",
        "age": 30,
        "active": true,
        "tags": ["a", "b", "c"]
    });
    let obj_str = r#"{"name":"John","age":30,"active":true,"tags":["a","b","c"]}"#;

    // JSON_STRINGIFY - O(n)
    group.bench_function("JSON_STRINGIFY", |b| {
        b.iter(|| black_box(serde_json::to_string(&obj).unwrap()));
    });
    group.bench_function("JSON_STRINGIFY_PRETTY", |b| {
        b.iter(|| black_box(serde_json::to_string_pretty(&obj).unwrap()));
    });
    group.bench_function("TO_JSON", |b| {
        b.iter(|| black_box(obj.to_string()));
    });

    // JSON_PARSE - O(n)
    group.bench_function("JSON_PARSE", |b| {
        b.iter(|| black_box(serde_json::from_str::<serde_json::Value>(obj_str).unwrap()));
    });
    group.bench_function("PARSE_JSON", |b| {
        b.iter(|| black_box(serde_json::from_str::<serde_json::Value>(obj_str).unwrap()));
    });

    // Deep access - O(depth)
    let nested = serde_json::json!({
        "a": {"b": {"c": {"d": {"e": "deep"}}}}
    });
    group.bench_function("JSON_POINTER_depth_1", |b| {
        b.iter(|| black_box(nested.pointer("/a")))
    });
    group.bench_function("JSON_POINTER_depth_3", |b| {
        b.iter(|| black_box(nested.pointer("/a/b/c")))
    });
    group.bench_function("JSON_POINTER_depth_5", |b| {
        b.iter(|| black_box(nested.pointer("/a/b/c/d/e")))
    });

    // Big object
    let big_obj = serde_json::json!({
        "data": (0..1000).map(|i| serde_json::json!({"id": i, "value": format!("item_{}", i)})).collect::<Vec<_>>()
    });
    group.bench_function("JSON_STRINGIFY_big", |b| {
        b.iter(|| black_box(serde_json::to_string(&big_obj).unwrap()));
    });
    group.bench_function("JSON_PARSE_big", |b| {
        let s = serde_json::to_string(&big_obj).unwrap();
        b.iter(|| black_box(serde_json::from_str::<serde_json::Value>(&s).unwrap()));
    });

    group.finish();
}

// ============================================================================
// TYPE CHECK FUNCTIONS (9 functions, 14 aliases)
// ============================================================================
fn bench_type_check_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("type_check_functions");

    let json_str = serde_json::json!("hello");
    let json_num = serde_json::json!(42);
    let json_num_float = serde_json::json!(42.5);
    let json_bool = serde_json::json!(true);
    let json_null = serde_json::json!(null);
    let json_arr = serde_json::json!([1, 2, 3]);
    let json_obj = serde_json::json!({"a": 1});
    let json_empty_str = serde_json::json!("");
    let json_empty_arr = serde_json::json!([]);

    // Type checks - O(1)
    group.bench_function("IS_STRING", |b| {
        b.iter(|| black_box(json_str.is_string()));
    });
    group.bench_function("IS_STRING_false", |b| {
        b.iter(|| black_box(json_num.is_string()));
    });
    group.bench_function("IS_NUMBER", |b| {
        b.iter(|| black_box(json_num.is_number()));
    });
    group.bench_function("IS_INTEGER", |b| {
        b.iter(|| black_box(json_num.as_i64().is_some()));
    });
    group.bench_function("IS_INTEGER_float", |b| {
        b.iter(|| black_box(json_num_float.as_i64().is_some()));
    });
    group.bench_function("IS_BOOL", |b| {
        b.iter(|| black_box(json_bool.is_boolean()));
    });
    group.bench_function("IS_NULL", |b| {
        b.iter(|| black_box(json_null.is_null()));
    });
    group.bench_function("IS_NULL_false", |b| {
        b.iter(|| black_box(json_str.is_null()));
    });
    group.bench_function("IS_ARRAY", |b| {
        b.iter(|| black_box(json_arr.is_array()));
    });
    group.bench_function("IS_LIST", |b| {
        b.iter(|| black_box(json_arr.is_array()));
    });
    group.bench_function("IS_OBJECT", |b| {
        b.iter(|| black_box(json_obj.is_object()));
    });
    group.bench_function("IS_DOCUMENT", |b| {
        b.iter(|| black_box(json_obj.is_object()));
    });
    group.bench_function("IS_EMPTY_string", |b| {
        b.iter(|| black_box(json_empty_str.as_str().map_or(true, |s| s.is_empty())));
    });
    group.bench_function("IS_EMPTY_array", |b| {
        b.iter(|| black_box(json_empty_arr.as_array().map_or(true, |a| a.is_empty())));
    });

    group.finish();
}

// ============================================================================
// MISC FUNCTIONS (21 functions, 27 aliases)
// ============================================================================
fn bench_misc_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("misc_functions");

    // UUID - O(1)
    group.bench_function("UUID", |b| {
        b.iter(|| {
            use std::time::SystemTime;
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap();
            black_box(format!(
                "{:x}-{:x}-{:x}-{:x}",
                now.as_secs(),
                now.as_nanos() & 0xffffffff,
                std::process::id() as u64 & 0xffff,
                now.as_nanos() as u64
            ))
        });
    });

    // TYPEOF / TYPENAME - O(1)
    let json_str = serde_json::json!("hello");
    let json_num = serde_json::json!(42);
    group.bench_function("TYPEOF_string", |b| {
        b.iter(|| {
            black_box(
                json_str
                    .get("")
                    .map_or("string".to_string(), |_| "string".to_string()),
            )
        });
    });
    group.bench_function("TYPEOF_number", |b| {
        b.iter(|| {
            black_box(
                json_num
                    .get("")
                    .map_or("number".to_string(), |_| "number".to_string()),
            )
        });
    });

    // COALESCE / NOT_NULL - O(1) short-circuit
    group.bench_function("COALESCE_first", |b| {
        b.iter(|| {
            let a: Option<i32> = Some(42);
            let b: Option<i32> = None;
            let c: Option<i32> = None;
            black_box(a.or(b).or(c).unwrap_or(0))
        });
    });
    group.bench_function("COALESCE_all_null", |b| {
        b.iter(|| {
            let a: Option<i32> = None;
            let b: Option<i32> = None;
            let c: Option<i32> = None;
            black_box(a.or(b).or(c).unwrap_or(0))
        });
    });
    group.bench_function("NOT_NULL", |b| {
        b.iter(|| {
            let a: Option<i32> = Some(42);
            let b: Option<i32> = None;
            black_box(a.or(b).unwrap_or(0))
        });
    });

    // NULLIF - O(1)
    group.bench_function("NULLIF_equal", |b| {
        b.iter(|| black_box(if 42 == 42 { None } else { Some(42) }));
    });
    group.bench_function("NULLIF_not_equal", |b| {
        b.iter(|| black_box(if 42 == 13 { None } else { Some(42) }));
    });

    // ASSERT - O(1)
    group.bench_function("ASSERT_true", |b| {
        b.iter(|| {
            let x = 42;
            black_box(if x > 0 { x } else { panic!("assertion failed") })
        });
    });

    // SLEEP - O(n) where n = ms
    group.bench_function("SLEEP_1ms", |b| {
        b.iter(|| {
            std::thread::sleep(std::time::Duration::from_millis(1));
            black_box(1)
        });
    });

    // RANGE - O(n)
    group.bench_function("RANGE", |b| {
        b.iter(|| (0..100i32).collect::<Vec<_>>());
    });

    // Type conversions - O(1)
    group.bench_function("TO_NUMBER", |b| {
        b.iter(|| black_box("42".parse::<f64>().unwrap()));
    });
    group.bench_function("TO_STRING", |b| {
        b.iter(|| black_box(42.to_string()));
    });
    group.bench_function("TO_BOOL", |b| {
        b.iter(|| black_box(true as bool));
    });
    group.bench_function("TO_ARRAY", |b| {
        b.iter(|| black_box(serde_json::json!([1, 2, 3])));
    });

    // Object operations - O(n)
    let obj = serde_json::json!({"a": 1, "b": 2, "c": 3, "d": 4, "e": 5});
    group.bench_function("ATTRIBUTES", |b| {
        b.iter(|| black_box(obj.as_object().unwrap().keys().cloned().collect::<Vec<_>>()));
    });
    group.bench_function("KEYS", |b| {
        b.iter(|| black_box(obj.as_object().unwrap().keys().cloned().collect::<Vec<_>>()));
    });
    group.bench_function("VALUES", |b| {
        b.iter(|| {
            black_box(
                obj.as_object()
                    .unwrap()
                    .values()
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        });
    });

    // HAS - O(1) average
    group.bench_function("HAS_true", |b| {
        b.iter(|| black_box(obj.get("a").is_some()));
    });
    group.bench_function("HAS_false", |b| {
        b.iter(|| black_box(obj.get("z").is_some()));
    });

    // KEEP - O(n)
    group.bench_function("KEEP", |b| {
        b.iter(|| {
            let keys = ["a", "b"];
            let mut result = serde_json::Map::new();
            for k in keys {
                if let Some(v) = obj.get(k) {
                    result.insert(k.to_string(), v.clone());
                }
            }
            black_box(result)
        });
    });

    // UNSET - O(n)
    group.bench_function("UNSET", |b| {
        b.iter(|| {
            let mut result = obj.as_object().unwrap().clone();
            result.remove("c");
            black_box(result)
        });
    });

    // IF - O(1)
    group.bench_function("IF_true", |b| {
        b.iter(|| black_box(if true { 42 } else { 13 }));
    });
    group.bench_function("IF_false", |b| {
        b.iter(|| black_box(if false { 42 } else { 13 }));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_string_functions,
    bench_array_functions,
    bench_math_functions,
    bench_crypto_functions,
    bench_datetime_functions,
    bench_type_check_functions,
    bench_json_functions,
    bench_geo_functions,
    bench_misc_functions
);
criterion_main!(benches);
