//! Same-harness before/after bench. Only calls functions that existed
//! on the pre-change tree so both trees can run this file unchanged.
//!
//!   cargo test --test sdbql_compare_bench -- --ignored --nocapture

use serde_json::json;
use solidb::sdbql::executor::builtins;
use solidb::sdbql::executor::phonetic;
use std::time::Instant;

fn call(name: &str, args: &[serde_json::Value]) {
    if let Some(v) = phonetic::evaluate(name, args).unwrap() {
        let _ = v;
        return;
    }
    let _ = builtins::evaluate(name, args)
        .unwrap()
        .expect(name);
}

fn bench(name: &str, iters: u32, mut f: impl FnMut()) {
    for _ in 0..iters / 10 {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let ns = start.elapsed().as_nanos() / u128::from(iters);
    println!("{:<28} {:>8} ns/iter", name, ns);
}

#[test]
#[ignore]
fn bench_sdbql_compare() {
    let n = if cfg!(debug_assertions) {
        5_000u32
    } else {
        50_000u32
    };
    println!(
        "\n=== SDBQL compare ({} , {} iters) ===\n",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        n
    );

    let hello = json!("Hello World");
    let iso = json!("2024-12-30T10:30:45.000Z");
    let ts = json!(1_735_555_845_000i64);
    let arr = json!([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    let dups: Vec<serde_json::Value> = (0..256).map(|i| json!(i % 64)).collect();
    let big = serde_json::Value::Array(dups);
    let obj = json!({"a": 1, "b": 2, "c": 3, "d": 4});

    println!("-- string --");
    bench("UPPER", n, || call("UPPER", &[hello.clone()]));
    bench("LOWER", n, || call("LOWER", &[hello.clone()]));
    bench("TRIM", n, || call("TRIM", &[json!("  padded  ")]));
    bench("CONCAT", n, || {
        call("CONCAT", &[json!("a"), json!("b"), json!("c")])
    });
    bench("CONTAINS_str", n, || {
        call("CONTAINS", &[hello.clone(), json!("World")])
    });
    bench("SUBSTRING", n, || {
        call("SUBSTRING", &[hello.clone(), json!(0), json!(5)])
    });
    bench("SPLIT", n, || {
        call("SPLIT", &[json!("a,b,c,d,e"), json!(",")])
    });
    bench("SUBSTITUTE", n, || {
        call("SUBSTITUTE", &[hello.clone(), json!("World"), json!("Rust")])
    });
    bench("FIND_FIRST", n, || {
        call("FIND_FIRST", &[hello.clone(), json!("World")])
    });
    bench("REGEX_TEST", n, || {
        call("REGEX_TEST", &[hello.clone(), json!(r"W\w+")])
    });

    println!("\n-- array / object --");
    bench("FIRST", n, || call("FIRST", &[arr.clone()]));
    bench("NTH", n, || call("NTH", &[arr.clone(), json!(3)]));
    bench("SLICE", n, || call("SLICE", &[arr.clone(), json!(2), json!(4)]));
    bench("UNIQUE_small", n, || {
        call("UNIQUE", &[json!([1, 2, 2, 3, 3, 3])])
    });
    bench("UNIQUE_256", n / 5, || call("UNIQUE", &[big.clone()]));
    bench("UNION_256", n / 5, || {
        call("UNION", &[big.clone(), big.clone()])
    });
    bench("MINUS_256", n / 5, || {
        call("MINUS", &[big.clone(), json!([1, 2, 3, 4])])
    });
    bench("INTERSECTION", n, || {
        call("INTERSECTION", &[arr.clone(), json!([2, 4, 6, 8, 10])])
    });
    bench("SUM", n, || call("SUM", &[arr.clone()]));
    bench("KEEP", n, || {
        call("KEEP", &[obj.clone(), json!("a"), json!("c")])
    });
    bench("HAS", n, || call("HAS", &[obj.clone(), json!("b")]));

    println!("\n-- date --");
    bench("DATE_NOW", n, || call("DATE_NOW", &[]));
    bench("DATE_YEAR_iso", n, || call("DATE_YEAR", &[iso.clone()]));
    bench("DATE_YEAR_ts", n, || call("DATE_YEAR", &[ts.clone()]));
    bench("DATE_ISO8601", n, || call("DATE_ISO8601", &[ts.clone()]));
    bench("DATE_TIMESTAMP", n, || call("DATE_TIMESTAMP", &[iso.clone()]));
    bench("DATE_TRUNC_day", n, || {
        call("DATE_TRUNC", &[iso.clone(), json!("day")])
    });
    bench("DATE_ADD_day", n, || {
        call("DATE_ADD", &[iso.clone(), json!(7), json!("day")])
    });
    bench("DATE_ADD_month", n, || {
        call("DATE_ADD", &[iso.clone(), json!(1), json!("month")])
    });
    bench("DATE_SUBTRACT", n, || {
        call("DATE_SUBTRACT", &[iso.clone(), json!(3), json!("hour")])
    });
    bench("DATE_DIFF_days", n, || {
        call(
            "DATE_DIFF",
            &[iso.clone(), json!("2024-01-01T00:00:00Z"), json!("days")],
        )
    });
    bench("DATE_FORMAT", n, || {
        call("DATE_FORMAT", &[iso.clone(), json!("%Y-%m-%d")])
    });
    bench("TIME_BUCKET", n, || {
        call("TIME_BUCKET", &[json!(90_000), json!("1m")])
    });
    bench("HUMAN_TIME", n, || call("HUMAN_TIME", &[ts.clone()]));

    println!("\n-- crypto --");
    bench("MD5", n, || call("MD5", &[hello.clone()]));
    bench("SHA256", n, || call("SHA256", &[hello.clone()]));
}
