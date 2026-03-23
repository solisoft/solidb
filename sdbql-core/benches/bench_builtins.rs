//! Benchmark suite for SDBQL built-in functions
//! Run with: cargo bench --bench bench_builtins -- --sample-size=10

use criterion::{black_box, criterion_group, criterion_main, Criterion};

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
    group.bench_function("ENDS_WITH", |b| {
        b.iter(|| black_box("hello world".ends_with("world")));
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

    // Length
    group.bench_function("LENGTH", |b| {
        b.iter(|| black_box("hello world".chars().count()));
    });

    // Modification
    group.bench_function("REVERSE", |b| {
        b.iter(|| black_box("hello".chars().rev().collect::<String>()));
    });
    group.bench_function("REPLACE", |b| {
        b.iter(|| black_box("hello world".replace("world", "rust")));
    });
    group.bench_function("CONCAT", |b| {
        b.iter(|| black_box(format!("{}{}{}", "hello", " ", "world")));
    });

    // Split & Join
    group.bench_function("SPLIT", |b| {
        b.iter(|| {
            let parts: Vec<&str> = black_box("a,b,c,d,e").split(',').collect();
            black_box(parts)
        });
    });
    group.bench_function("JOIN", |b| {
        b.iter(|| {
            let parts = ["a", "b", "c", "d", "e"];
            black_box(parts.join(","))
        });
    });

    // Long string operations
    let long_str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(10);
    group.bench_function("UPPER_long", |b| {
        b.iter(|| black_box(long_str.to_uppercase()));
    });
    group.bench_function("CONTAINS_long", |b| {
        b.iter(|| black_box(long_str.contains("ipsum")));
    });
    group.bench_function("REPLACE_long", |b| {
        b.iter(|| black_box(long_str.replace("dolor", "RUST")));
    });

    group.finish();
}

fn bench_array_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("array_functions");

    let arr = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let arr2 = vec![7, 8, 9, 10, 11, 12];
    let nested_arr = vec![vec![1, 2], vec![3, 4], vec![5, 6]];

    // Access
    group.bench_function("FIRST", |b| {
        b.iter(|| black_box(arr.first().copied()));
    });
    group.bench_function("LAST", |b| {
        b.iter(|| black_box(arr.last().copied()));
    });
    group.bench_function("NTH", |b| {
        b.iter(|| black_box(arr.get(5).copied()));
    });
    group.bench_function("LENGTH", |b| {
        b.iter(|| black_box(arr.len()));
    });

    // Search
    group.bench_function("CONTAINS_hit", |b| {
        b.iter(|| black_box(arr.contains(&7)));
    });
    group.bench_function("CONTAINS_miss", |b| {
        b.iter(|| black_box(arr.contains(&100)));
    });
    group.bench_function("POSITION", |b| {
        b.iter(|| black_box(arr.iter().position(|&x| x == 7)));
    });

    // Modification
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

    // Slicing
    group.bench_function("SLICE", |b| {
        b.iter(|| black_box(&arr[2..8]));
    });

    // Ordering
    group.bench_function("REVERSE", |b| {
        b.iter(|| {
            let mut a = arr.clone();
            a.reverse();
            black_box(a)
        });
    });
    group.bench_function("SORT", |b| {
        b.iter(|| {
            let mut a = arr.clone();
            a.sort();
            black_box(a)
        });
    });

    // Deduplication
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

    // Set operations
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

    // Flatten
    group.bench_function("FLATTEN", |b| {
        b.iter(|| nested_arr.iter().flatten().cloned().collect::<Vec<_>>());
    });

    // Map/Filter/Reduce
    group.bench_function("MAP", |b| {
        b.iter(|| arr.iter().map(|x| x * 2).collect::<Vec<_>>());
    });
    group.bench_function("FILTER", |b| {
        b.iter(|| arr.iter().filter(|x| **x > 5).cloned().collect::<Vec<_>>());
    });
    group.bench_function("REDUCE", |b| {
        b.iter(|| arr.iter().fold(0, |acc, x| acc + x));
    });

    group.finish();
}

fn bench_math_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("math_functions");

    // Basic
    group.bench_function("ABS", |b| b.iter(|| black_box((-42.5_f64).abs())));
    group.bench_function("FLOOR", |b| b.iter(|| black_box(42.7_f64.floor())));
    group.bench_function("CEIL", |b| b.iter(|| black_box(42.2_f64.ceil())));
    group.bench_function("ROUND", |b| b.iter(|| black_box(42.5_f64.round())));

    // Powers & Roots
    group.bench_function("SQRT", |b| b.iter(|| black_box(144.0_f64.sqrt())));
    group.bench_function("POW", |b| b.iter(|| black_box(2.0_f64.powf(10.0))));
    group.bench_function("EXP", |b| b.iter(|| black_box(1.0_f64.exp())));
    group.bench_function("LN", |b| b.iter(|| black_box(100.0_f64.ln())));
    group.bench_function("LOG10", |b| b.iter(|| black_box(100.0_f64.log10())));

    // Trigonometry
    group.bench_function("SIN", |b| b.iter(|| black_box(1.0_f64.sin())));
    group.bench_function("COS", |b| b.iter(|| black_box(1.0_f64.cos())));
    group.bench_function("TAN", |b| b.iter(|| black_box(1.0_f64.tan())));

    // Clamping
    group.bench_function("CLAMP", |b| b.iter(|| black_box(15.0_f64.clamp(0.0, 10.0))));
    group.bench_function("MAX", |b| b.iter(|| black_box(42_f64.max(13.0))));
    group.bench_function("MIN", |b| b.iter(|| black_box(42_f64.min(13.0))));

    // Aggregations on 10K elements
    let big_arr: Vec<f64> = (0..10000).map(|i| i as f64).collect();

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

    group.finish();
}

fn bench_crypto_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("crypto_functions");

    let data = b"hello world this is a test message for hashing";

    // Fast hash
    group.bench_function("FAST_HASH", |b| {
        b.iter(|| {
            let mut hash: u64 = 0;
            for &byte in black_box(data) {
                hash = hash.wrapping_mul(31).wrapping_add(byte as u64);
            }
            black_box(hash)
        });
    });

    // CRC32
    group.bench_function("CRC32", |b| {
        b.iter(|| {
            let mut crc: u32 = 0xFFFFFFFF;
            for byte in black_box(data) {
                crc ^= *byte as u32;
                for _ in 0..8 {
                    crc = if crc & 1 != 0 {
                        0xEDB88320 ^ (crc >> 1)
                    } else {
                        crc >> 1
                    };
                }
            }
            black_box(crc ^ 0xFFFFFFFF)
        });
    });

    // Simulated MD5 cost
    group.bench_function("MD5", |b| {
        b.iter(|| {
            let mut hash: u128 = 0;
            for &byte in black_box(data) {
                hash = hash
                    .wrapping_mul(0x6a09e667f3bcc908_u128)
                    .wrapping_add(byte as u128);
            }
            black_box(hash)
        });
    });

    group.finish();
}

fn bench_type_check_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("type_check_functions");

    let json_str = serde_json::json!("hello");
    let json_num = serde_json::json!(42);
    let json_bool = serde_json::json!(true);
    let json_null = serde_json::json!(null);
    let json_arr = serde_json::json!([1, 2, 3]);
    let json_obj = serde_json::json!({"a": 1});

    group.bench_function("IS_STRING", |b| {
        b.iter(|| black_box(json_str.is_string()));
    });
    group.bench_function("IS_NUMBER", |b| {
        b.iter(|| black_box(json_num.is_number()));
    });
    group.bench_function("IS_INTEGER", |b| {
        b.iter(|| black_box(json_num.as_i64().is_some()));
    });
    group.bench_function("IS_BOOL", |b| {
        b.iter(|| black_box(json_bool.is_boolean()));
    });
    group.bench_function("IS_NULL", |b| {
        b.iter(|| black_box(json_null.is_null()));
    });
    group.bench_function("IS_ARRAY", |b| {
        b.iter(|| black_box(json_arr.is_array()));
    });
    group.bench_function("IS_OBJECT", |b| {
        b.iter(|| black_box(json_obj.is_object()));
    });

    group.finish();
}

fn bench_json_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_functions");

    let obj = serde_json::json!({
        "name": "John",
        "age": 30,
        "active": true,
        "tags": ["a", "b", "c"]
    });
    let obj_str = r#"{"name":"John","age":30,"active":true,"tags":["a","b","c"]}"#;

    group.bench_function("JSON_STRINGIFY", |b| {
        b.iter(|| black_box(serde_json::to_string(&obj).unwrap()));
    });
    group.bench_function("JSON_PARSE", |b| {
        b.iter(|| black_box(serde_json::from_str::<serde_json::Value>(obj_str).unwrap()));
    });

    let nested = serde_json::json!({
        "a": {"b": {"c": {"d": {"e": "deep"}}}}
    });
    group.bench_function("JSON_ACCESS_deep", |b| {
        b.iter(|| black_box(nested.pointer("/a/b/c/d/e")));
    });

    group.finish();
}

fn bench_geo_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("geo_functions");

    fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
        let r = 6371.0;
        let dlat = (lat2 - lat1).to_radians();
        let dlon = (lon2 - lon1).to_radians();
        let a = (dlat / 2.0).sin().powi(2)
            + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
        2.0 * r * a.sqrt().asin()
    }

    group.bench_function("DISTANCE", |b| {
        b.iter(|| black_box(haversine(48.8566, 2.3522, 51.5074, -0.1278)));
    });

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

    let polygon = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
    group.bench_function("GEO_WITHIN", |b| {
        b.iter(|| black_box(point_in_polygon(2.0, 2.0, &polygon)));
    });

    group.finish();
}

fn bench_misc_functions(c: &mut Criterion) {
    let mut group = c.benchmark_group("misc_functions");

    fn rand_simple() -> u64 {
        use std::time::SystemTime;
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();
        now.as_nanos() as u64
    }

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
                rand_simple()
            ))
        });
    });

    group.bench_function("COALESCE_first", |b| {
        b.iter(|| {
            let a: Option<i32> = Some(42);
            let b: Option<i32> = None;
            black_box(a.or(b).unwrap_or(0))
        });
    });

    let obj = serde_json::json!({"a": 1, "b": 2, "c": 3});
    group.bench_function("KEYS", |b| {
        b.iter(|| black_box(obj.as_object().unwrap().keys().collect::<Vec<_>>()));
    });
    group.bench_function("VALUES", |b| {
        b.iter(|| black_box(obj.as_object().unwrap().values().collect::<Vec<_>>()));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_string_functions,
    bench_array_functions,
    bench_math_functions,
    bench_crypto_functions,
    bench_type_check_functions,
    bench_json_functions,
    bench_geo_functions,
    bench_misc_functions
);
criterion_main!(benches);
