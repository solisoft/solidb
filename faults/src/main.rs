//! Fault tests for SoliDB: what it keeps when it is killed mid-write.
//!
//! ```text
//! faults crash --bin ./solidb --engine solistore --durability full --cycles 20 --out crash.jsonl
//! faults probe-client-tx --bin ./solidb
//! faults isolation --bin ./solidb --mode doc --isolation serializable --history h.json
//! ```
//!
//! The server is a real binary started and killed by the harness, on a data
//! directory of its own (`--data`, deleted first). `crash` is described in
//! `crash.rs`; `probe-client-tx` checks what the Rust client's transaction
//! calls actually do.

mod crash;
mod isolation;
mod probe;
mod server;

use std::collections::HashMap;
use std::path::PathBuf;

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// xorshift64*, seeded: a run is reproducible up to the kill timings.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }
}

fn usage() -> ! {
    eprintln!(
        "usage:\n  faults crash --bin PATH [--engine rocksdb|solistore] [--durability eventual|full] \
         [--cycles N] [--tx-writers N] [--single-writers N] [--single-rate N] [--min-ms N] [--max-ms N] \
         [--tx-isolation read_committed|read_uncommitted|repeatable_read|serializable] [--seed N] \
         [--port N] [--data DIR] [--out FILE]\n  faults probe-client-tx --bin PATH [--port N] [--data DIR]\n  \
         faults isolation --bin PATH [--mode doc|sdbql|cursor] [--isolation LEVEL] [--clients N] [--keys N] \
         [--max-writes N] [--secs N] [--seed N] [--history FILE] [--port N] [--data DIR]"
    );
    std::process::exit(2)
}

#[tokio::main]
async fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().cloned() else {
        usage()
    };
    let mut opt: HashMap<String, String> = HashMap::new();
    let mut it = args[1..].iter();
    while let Some(k) = it.next() {
        let Some(name) = k.strip_prefix("--") else {
            usage()
        };
        let Some(v) = it.next() else { usage() };
        opt.insert(name.to_string(), v.clone());
    }
    let get = |k: &str, d: &str| opt.get(k).cloned().unwrap_or_else(|| d.to_string());
    let num = |k: &str, d: u64| {
        get(k, &d.to_string())
            .parse::<u64>()
            .unwrap_or_else(|_| usage())
    };

    let bin = PathBuf::from(opt.get("bin").cloned().unwrap_or_else(|| usage()));
    let engine = get("engine", "rocksdb");
    let durability = get("durability", "eventual");
    let port = num("port", 6900) as u16;
    let data = PathBuf::from(get(
        "data",
        &format!("/tmp/faults-{cmd}-{engine}-{durability}"),
    ));
    if data.exists() {
        std::fs::remove_dir_all(&data)?;
    }
    std::fs::create_dir_all(&data)?;
    let log = data.with_extension("server.log");
    let _ = std::fs::remove_file(&log);
    let env = vec![
        ("SOLIDB_ENGINE".to_string(), engine.clone()),
        ("SOLIDB_DURABILITY".to_string(), durability.clone()),
    ];
    let mut server = server::Server::new(bin, data, port, env, log);

    match cmd.as_str() {
        "crash" => {
            let opts = crash::Opts {
                cycles: num("cycles", 10) as u32,
                tx_writers: num("tx-writers", 4) as u32,
                single_writers: num("single-writers", 4) as u32,
                single_rate: num("single-rate", 2000) as u32,
                min_ms: num("min-ms", 500),
                max_ms: num("max-ms", 4000),
                tx_isolation: get("tx-isolation", "read_committed"),
                seed: num("seed", 1),
            };
            let mut out: Box<dyn std::io::Write> = match opt.get("out") {
                Some(p) => Box::new(std::fs::File::create(p)?),
                None => Box::new(std::io::stdout()),
            };
            eprintln!(
                "[faults] crash: engine {engine}, durability {durability}, {} cycles",
                opts.cycles
            );
            let violations = crash::run(&mut server, opts, &mut out).await?;
            eprintln!("[faults] done: {violations} violation(s)");
            std::process::exit(if violations == 0 { 0 } else { 1 });
        }
        "isolation" => {
            let opts = isolation::Opts {
                mode: get("mode", "doc"),
                isolation: get("isolation", "serializable"),
                clients: num("clients", 10) as u32,
                keys: num("keys", 8),
                max_writes: num("max-writes", 32),
                secs: num("secs", 30),
                seed: num("seed", 1),
            };
            let history = get("history", "history.json");
            let anomalies = isolation::run(&mut server, opts, &history).await?;
            std::process::exit(if anomalies == 0 { 0 } else { 1 });
        }
        "probe-client-tx" => {
            let findings = probe::run(&mut server).await?;
            println!("{}", serde_json::to_string_pretty(&findings)?);
        }
        _ => usage(),
    }
    Ok(())
}
