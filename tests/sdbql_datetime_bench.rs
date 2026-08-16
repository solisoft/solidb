//! Date/time builtin micro-benchmarks (same dispatch as the query executor).
//!
//!   cargo test --release --test sdbql_datetime_bench -- --ignored --nocapture

use serde_json::json;
use solidb::sdbql::executor::builtins;
use solidb::sdbql::executor::phonetic;
use std::time::Instant;

fn call(name: &str, args: &[serde_json::Value]) {
    if let Some(v) = phonetic::evaluate(name, args).unwrap() {
        let _ = v;
        return;
    }
    let _ = builtins::evaluate(name, args).unwrap().unwrap();
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
    println!("{:<28} {:>8} ns/iter  ({} iters)", name, ns, iters);
}

#[test]
#[ignore]
fn bench_sdbql_datetime_functions() {
    let n = if cfg!(debug_assertions) {
        5_000u32
    } else {
        50_000u32
    };
    let iso = json!("2024-12-30T10:30:45.000Z");
    let ts = json!(1_735_555_845_000i64);
    println!(
        "\nSDBQL date/time ({} profile, {} iters)\n",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        n
    );

    bench("DATE_NOW", n, || call("DATE_NOW", &[]));
    bench("DATE_YEAR_iso", n, || call("DATE_YEAR", &[iso.clone()]));
    bench("DATE_YEAR_ts", n, || call("DATE_YEAR", &[ts.clone()]));
    bench("DATE_MONTH", n, || call("DATE_MONTH", &[iso.clone()]));
    bench("DATE_DAY", n, || call("DATE_DAY", &[iso.clone()]));
    bench("DATE_HOUR", n, || call("DATE_HOUR", &[iso.clone()]));
    bench("DATE_QUARTER", n, || call("DATE_QUARTER", &[iso.clone()]));
    bench("DATE_DAYOFWEEK", n, || {
        call("DATE_DAYOFWEEK", &[iso.clone()])
    });
    bench("DATE_DAYOFYEAR", n, || {
        call("DATE_DAYOFYEAR", &[iso.clone()])
    });
    bench("DATE_ISOWEEK", n, || call("DATE_ISOWEEK", &[iso.clone()]));
    bench("DATE_ISO8601", n, || call("DATE_ISO8601", &[ts.clone()]));
    bench("DATE_TIMESTAMP", n, || {
        call("DATE_TIMESTAMP", &[iso.clone()])
    });
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
    bench("DATE_DAYS_IN_MONTH", n, || {
        call("DATE_DAYS_IN_MONTH", &[iso.clone()])
    });
    bench("TIME_BUCKET", n, || {
        call("TIME_BUCKET", &[json!(90_000), json!("1m")])
    });
    bench("HUMAN_TIME", n, || call("HUMAN_TIME", &[ts.clone()]));
}
