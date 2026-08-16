//! Micro-benchmarks for the live SDBQL string builtins.
//!
//!   cargo test --release --test sdbql_string_bench -- --nocapture --ignored

use serde_json::json;
use solidb::sdbql::executor::builtins;
use std::time::Instant;

fn call(name: &str, args: &[serde_json::Value]) {
    builtins::evaluate(name, args).unwrap().unwrap();
}

fn bench(name: &str, iters: u32, mut f: impl FnMut()) {
    // Warmup
    for _ in 0..iters / 10 {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let elapsed = start.elapsed();
    let ns = elapsed.as_nanos() / u128::from(iters);
    println!("{:<24} {:>8} ns/iter  ({} iters)", name, ns, iters);
}

#[test]
#[ignore]
fn bench_sdbql_string_functions() {
    let hello = json!("Hello World");
    let cafe = json!("café au lait — 日本語");
    let long = json!("Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(20));
    let n = if cfg!(debug_assertions) {
        5_000u32
    } else {
        50_000u32
    };

    println!("\nSDBQL string builtins (release, {} iters)\n", n);

    bench("UPPER", n, || call("UPPER", &[hello.clone()]));
    bench("LOWER_unicode", n, || call("LOWER", &[cafe.clone()]));
    bench("TRIM", n, || call("TRIM", &[json!("  padded  ")]));
    bench("CONCAT", n, || {
        call("CONCAT", &[json!("a"), json!("b"), json!("c")])
    });
    bench("CONTAINS_hit", n, || {
        call("CONTAINS", &[hello.clone(), json!("World")])
    });
    bench("CONTAINS_index", n, || {
        call("CONTAINS", &[cafe.clone(), json!("日"), json!(true)])
    });
    bench("FIND_FIRST", n, || {
        call("FIND_FIRST", &[hello.clone(), json!("World")])
    });
    bench("SUBSTRING", n, || {
        call("SUBSTRING", &[hello.clone(), json!(0), json!(5)])
    });
    bench("SUBSTRING_neg", n, || {
        call("SUBSTRING", &[hello.clone(), json!(-5)])
    });
    bench("SPLIT", n, || {
        call("SPLIT", &[json!("a,b,c,d,e"), json!(",")])
    });
    bench("SUBSTITUTE", n, || {
        call(
            "SUBSTITUTE",
            &[hello.clone(), json!("World"), json!("Rust")],
        )
    });
    bench("LIKE", n, || {
        call("LIKE", &[hello.clone(), json!("H%World")])
    });
    bench("REGEX_TEST_cached", n, || {
        call("REGEX_TEST", &[hello.clone(), json!(r"W\w+")])
    });
    bench("REGEX_MATCHES", n, || {
        call("REGEX_MATCHES", &[json!("a1b2c3"), json!(r"\d")])
    });
    bench("REGEX_SPLIT", n, || {
        call("REGEX_SPLIT", &[json!("a,b,c"), json!(",")])
    });
    bench("REPEAT", n, || call("REPEAT", &[json!("ab"), json!(16)]));
    bench("PAD_LEFT", n, || {
        call("PAD_LEFT", &[json!("1"), json!(8), json!("0")])
    });
    bench("ENCODE_URI", n, || call("ENCODE_URI", &[cafe.clone()]));
    bench("CHAR_LENGTH_long", n, || {
        call("CHAR_LENGTH", &[long.clone()])
    });
    bench("UPPER_long", n, || call("UPPER", &[long.clone()]));
    bench("CONTAINS_long", n, || {
        call("CONTAINS", &[long.clone(), json!("ipsum")])
    });

    println!("\nArray / math / object\n");

    let arr = json!([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    bench("FIRST", n, || call("FIRST", &[arr.clone()]));
    bench("NTH", n, || call("NTH", &[arr.clone(), json!(3)]));
    bench("SLICE", n, || {
        call("SLICE", &[arr.clone(), json!(2), json!(4)])
    });
    bench("TAKE", n, || call("TAKE", &[arr.clone(), json!(4)]));
    bench("ZIP", n, || call("ZIP", &[arr.clone(), arr.clone()]));
    bench("UNIQUE", n, || call("UNIQUE", &[json!([1, 2, 2, 3, 3, 3])]));
    let big: Vec<serde_json::Value> = (0..256).map(|i| json!(i % 64)).collect();
    let big_arr = serde_json::Value::Array(big);
    bench("UNIQUE_256", n / 5, || call("UNIQUE", &[big_arr.clone()]));
    bench("UNION_256", n / 5, || {
        call("UNION", &[big_arr.clone(), big_arr.clone()])
    });
    bench("MINUS_256", n / 5, || {
        call("MINUS", &[big_arr.clone(), json!([1, 2, 3, 4])])
    });
    bench("CONTAINS_array", n, || {
        call("CONTAINS", &[arr.clone(), json!(7)])
    });
    bench("MOD", n, || call("MOD", &[json!(7), json!(3)]));
    bench("CLAMP", n, || {
        call("CLAMP", &[json!(10), json!(0), json!(5)])
    });
    bench("MIN_variadic", n, || {
        call("MIN", &[json!(3), json!(1), json!(2)])
    });
    bench("SUM", n, || call("SUM", &[arr.clone()]));
    bench("GET", n, || {
        call("GET", &[json!({"a": {"b": 2}}), json!("a.b")])
    });
    bench("DEEP_MERGE", n, || {
        call(
            "DEEP_MERGE",
            &[json!({"a": {"x": 1}}), json!({"a": {"y": 2}})],
        )
    });
    bench("JSON_POINTER", n, || {
        call("JSON_POINTER", &[json!({"a": {"b": 3}}), json!("/a/b")])
    });
}
