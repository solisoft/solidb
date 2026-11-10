//! `faults isolation`: concurrent read/append transactions, their history in
//! Elle's list-append format, and the anomalies that can be read off it
//! without a cycle search.
//!
//! The database holds lists, one document per key (`{_key: "k3", vals:
//! [..]}`). `--keys` of them are in use at a time; a key that has taken
//! `--max-writes` appends is retired and replaced by a new one, as Jepsen does,
//! so that lists stay short and the history stays small enough for Elle. `--clients` clients run transactions of one to four
//! micro-operations on random keys for `--secs` seconds: `["r", k, null]`
//! reads the list, `["append", k, v]` adds `v`, unique per key, at its end.
//! Every transaction goes through the HTTP transaction API at
//! `--isolation`, by one of two paths (`--mode`):
//!
//! - `doc`: a read is `GET …/transaction/{tx}/document/la/{k}` (a shared lock,
//!   and the transaction's own writes); an append reads the same way and
//!   writes the new list with `PUT …/document/la/{k}` (an exclusive lock).
//! - `sdbql`: a read is `FOR d IN la FILTER d._key == @k RETURN d.vals` and an
//!   append `FOR d IN la FILTER d._key == @k UPDATE d WITH {vals: PUSH(d.vals,
//!   @v)} IN la`, both through `POST …/transaction/{tx}/query`.
//! - `cursor`: no transaction. Each operation is one micro-operation, the same
//!   statement as `sdbql`, through `POST …/cursor`: what an application's
//!   `UPDATE d WITH {n: d.n + 1}` does.
//!
//! A refusal (a lock conflict, a commit that fails validation) rolls the
//! transaction back and records it as failed; no answer records it as
//! indeterminate. After the run every list is read once more, outside any
//! transaction.
//!
//! The history (`--history`) is a JSON array Elle reads as it is
//! (`elle-cli --model list-append`), which finds dependency cycles (G0, G1c,
//! G2, write skew). This command checks, by itself, what needs no cycle:
//!
//! | Anomaly | Meaning |
//! |---|---|
//! | `lost-append` | an append whose transaction committed is not in the final list |
//! | `aborted-read` (G1a) | a value whose transaction failed is read, or kept |
//! | `incompatible-order` | two reads of a list where neither is a prefix of the other |
//! | `duplicate-elements` | a value appears twice in one read |
//! | `garbage-read` | a value nobody appended is read |
//! | `internal` | a read after the transaction's own append does not end with it |

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::server::{login, Server};
use crate::{Res, Rng};

const DB: &str = "faults_iso";
const COLL: &str = "la";

pub struct Opts {
    pub mode: String,
    pub isolation: String,
    pub clients: u32,
    pub keys: u64,
    pub max_writes: u64,
    pub secs: u64,
    pub seed: u64,
}

#[derive(Clone, Debug)]
enum Mop {
    Read(u64, Option<Vec<i64>>),
    Append(u64, i64),
}

impl Mop {
    fn to_json(&self) -> Value {
        match self {
            Mop::Read(k, v) => json!(["r", k, v]),
            Mop::Append(k, v) => json!(["append", k, v]),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Outcome {
    Ok,
    Fail,
    Info,
}

/// One completed transaction: what was asked, what came back.
struct Txn {
    process: u32,
    invoked_ns: u128,
    completed_ns: u128,
    outcome: Outcome,
    mops: Vec<Mop>,
    error: Option<String>,
}

struct Ctx {
    http: reqwest::Client,
    url: String,
    token: String,
    mode: String,
    isolation: String,
    /// The keys in use, and how many appends each has taken. A key's appends
    /// are numbered 1, 2, …: unique per key, as Elle needs.
    slots: tokio::sync::Mutex<Vec<(u64, u64)>>,
    next_key: AtomicU64,
    max_writes: u64,
    start: Instant,
}

pub async fn run(server: &mut Server, opts: Opts, history_path: &str) -> Res<usize> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    server.start(&http).await?;
    let token = login(&http, &server.url()).await?;
    setup(&http, &server.url(), &token, opts.keys).await?;

    let ctx = Arc::new(Ctx {
        http: http.clone(),
        url: server.url(),
        token: token.clone(),
        mode: opts.mode.clone(),
        isolation: opts.isolation.clone(),
        slots: tokio::sync::Mutex::new((0..opts.keys).map(|k| (k, 0)).collect()),
        next_key: AtomicU64::new(opts.keys),
        max_writes: opts.max_writes,
        start: Instant::now(),
    });
    let done: Arc<Mutex<Vec<Txn>>> = Arc::new(Mutex::new(Vec::new()));
    let deadline = Instant::now() + Duration::from_secs(opts.secs);
    let mut tasks = Vec::new();
    for p in 0..opts.clients {
        let (ctx, done) = (ctx.clone(), done.clone());
        let mut rng = Rng::new(opts.seed.wrapping_mul(7919).wrapping_add(p as u64 + 1));
        let keys = opts.keys;
        tasks.push(tokio::spawn(async move {
            while Instant::now() < deadline {
                let mops = match generate(&mut rng, keys, &ctx).await {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("[faults] creating a key: {e}");
                        break;
                    }
                };
                let txn = execute(&ctx, p, mops).await;
                done.lock().unwrap().push(txn);
            }
        }));
    }
    for t in tasks {
        t.await?;
    }

    // Every list once more, outside any transaction.
    let mut finals: BTreeMap<u64, Vec<i64>> = BTreeMap::new();
    for k in 0..ctx.next_key.load(Ordering::SeqCst) {
        finals.insert(k, plain_read(&http, &server.url(), &token, k).await?);
    }
    server.kill().await?;

    let mut txns = Arc::try_unwrap(done).ok().unwrap().into_inner().unwrap();
    txns.sort_by_key(|t| t.invoked_ns);
    write_history(&txns, &finals, opts.clients, history_path)?;

    let ok = txns.iter().filter(|t| t.outcome == Outcome::Ok).count();
    let fail = txns.iter().filter(|t| t.outcome == Outcome::Fail).count();
    let info = txns.iter().filter(|t| t.outcome == Outcome::Info).count();
    let mut reasons: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for t in txns.iter().filter(|t| t.outcome != Outcome::Ok) {
        let e = t.error.clone().unwrap_or_default();
        let kind = if e.contains("changed since this query read it") {
            "stale write refused".to_string()
        } else if e.contains("since this transaction") {
            "read changed since: refused".to_string()
        } else if e.contains("Write conflict") {
            "lock conflict: held by a writer".to_string()
        } else if e.contains("Read conflict") {
            "lock conflict: held by a reader".to_string()
        } else if e.starts_with("commit") {
            "commit refused".to_string()
        } else {
            e.chars().take(80).collect()
        };
        let entry = reasons
            .entry(kind)
            .or_insert((0, e.chars().take(200).collect()));
        entry.0 += 1;
    }
    let anomalies = check(&txns, &finals);
    let summary = json!({
        "mode": opts.mode, "isolation": opts.isolation, "clients": opts.clients,
        "keys": opts.keys, "secs": opts.secs,
        "transactions": {"ok": ok, "fail": fail, "info": info},
        "failed_because": reasons.iter().map(|(k, (n, e))| (k.clone(), json!({"count": n, "example": e}))).collect::<BTreeMap<_, _>>(),
        "anomalies": anomalies.iter().fold(BTreeMap::<String, usize>::new(), |mut m, (kind, _)| {
            *m.entry(kind.clone()).or_default() += 1; m
        }),
        "examples": anomalies.iter().fold(BTreeMap::<String, Vec<String>>::new(), |mut m, (kind, what)| {
            let e = m.entry(kind.clone()).or_default();
            if e.len() < 3 { e.push(what.clone()); }
            m
        }),
        "history": history_path,
    });
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(anomalies.len())
}

async fn setup(http: &reqwest::Client, url: &str, token: &str, keys: u64) -> Res<()> {
    let ok = |r: reqwest::Response| async move {
        let s = r.status();
        if s.is_success() {
            Ok(())
        } else {
            Err::<(), Box<dyn std::error::Error + Send + Sync>>(
                format!("HTTP {s}: {}", r.text().await.unwrap_or_default()).into(),
            )
        }
    };
    ok(http
        .post(format!("{url}/_api/database"))
        .bearer_auth(token)
        .json(&json!({"name": DB}))
        .send()
        .await?)
    .await?;
    ok(http
        .post(format!("{url}/_api/database/{DB}/collection"))
        .bearer_auth(token)
        .json(&json!({"name": COLL}))
        .send()
        .await?)
    .await?;
    for k in 0..keys {
        create_key(http, url, token, k).await?;
    }
    Ok(())
}

async fn create_key(http: &reqwest::Client, url: &str, token: &str, k: u64) -> Res<()> {
    let r = http
        .post(format!("{url}/_api/database/{DB}/document/{COLL}"))
        .bearer_auth(token)
        .json(&json!({"_key": format!("k{k}"), "vals": []}))
        .send()
        .await?;
    if !r.status().is_success() {
        return Err(format!("create k{k}: HTTP {}", r.status()).into());
    }
    Ok(())
}

/// One to four micro-operations (one without a transaction), reads and
/// appends half and half, on the keys in use.
async fn generate(rng: &mut Rng, keys: u64, ctx: &Ctx) -> Res<Vec<Mop>> {
    let n = if ctx.mode == "cursor" {
        1
    } else {
        1 + rng.below(4)
    };
    let mut mops = Vec::new();
    for _ in 0..n {
        let slot = rng.below(keys) as usize;
        let read = rng.below(2) == 0;
        let mut slots = ctx.slots.lock().await;
        if read {
            mops.push(Mop::Read(slots[slot].0, None));
            continue;
        }
        if slots[slot].1 >= ctx.max_writes {
            // Retire the key; its replacement exists before anyone uses it.
            let k = ctx.next_key.fetch_add(1, Ordering::SeqCst);
            create_key(&ctx.http, &ctx.url, &ctx.token, k).await?;
            slots[slot] = (k, 0);
        }
        slots[slot].1 += 1;
        mops.push(Mop::Append(slots[slot].0, slots[slot].1 as i64));
    }
    Ok(mops)
}

enum Step {
    /// The server answered with a refusal: the transaction is rolled back.
    Refused(String),
    /// No answer: the outcome is unknown.
    Lost(String),
}

async fn send(rb: reqwest::RequestBuilder) -> Result<(reqwest::StatusCode, Value), Step> {
    match rb.send().await {
        Err(e) => Err(Step::Lost(e.to_string())),
        Ok(r) => {
            let status = r.status();
            let text = r.text().await.map_err(|e| Step::Lost(e.to_string()))?;
            let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
            Ok((status, body))
        }
    }
}

fn vals(doc: &Value) -> Vec<i64> {
    doc["vals"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}

async fn execute(ctx: &Ctx, process: u32, mops: Vec<Mop>) -> Txn {
    if ctx.mode == "cursor" {
        return execute_plain(ctx, process, mops).await;
    }
    execute_in_transaction(ctx, process, mops).await
}

/// One statement through `/cursor`, outside any transaction.
async fn execute_plain(ctx: &Ctx, process: u32, mut mops: Vec<Mop>) -> Txn {
    let invoked_ns = ctx.start.elapsed().as_nanos();
    let url = format!("{}/_api/database/{DB}/cursor", ctx.url);
    let (query, vars) = match &mops[0] {
        Mop::Read(k, _) => (format!("FOR d IN {COLL} FILTER d._key == @k RETURN d.vals"), json!({"k": format!("k{k}")})),
        Mop::Append(k, v) => (
            format!("FOR d IN {COLL} FILTER d._key == @k UPDATE d WITH {{vals: PUSH(d.vals, @v)}} IN {COLL}"),
            json!({"k": format!("k{k}"), "v": v}),
        ),
    };
    let sent = send(ctx.http.post(&url).bearer_auth(&ctx.token).json(&json!({
        "query": query, "bindVars": vars, "cache": false,
    })))
    .await;
    let (outcome, error) = match sent {
        Err(Step::Lost(e)) | Err(Step::Refused(e)) => (Outcome::Info, Some(e)),
        Ok((status, body)) if !status.is_success() => (Outcome::Fail, Some(body.to_string())),
        Ok((_, body)) => {
            if let Mop::Read(_, out) = &mut mops[0] {
                let list = body["result"][0].as_array().cloned().unwrap_or_default();
                *out = Some(list.iter().filter_map(Value::as_i64).collect());
            }
            (Outcome::Ok, None)
        }
    };
    Txn {
        process,
        invoked_ns,
        completed_ns: ctx.start.elapsed().as_nanos(),
        outcome,
        mops,
        error,
    }
}

async fn execute_in_transaction(ctx: &Ctx, process: u32, mut mops: Vec<Mop>) -> Txn {
    let invoked_ns = ctx.start.elapsed().as_nanos();
    let base = format!("{}/_api/database/{DB}/transaction", ctx.url);
    let http = &ctx.http;
    let token = &ctx.token;

    let result: Result<(), Step> = async {
        let (status, begin) = send(
            http.post(format!("{base}/begin")).bearer_auth(token).json(&json!({"isolationLevel": ctx.isolation})),
        )
        .await?;
        if !status.is_success() {
            return Err(Step::Refused(format!("begin: {begin}")));
        }
        let tx = begin["id"].as_str().unwrap_or_default().to_string();
        let body: Result<(), Step> = async {
            for mop in mops.iter_mut() {
                match (ctx.mode.as_str(), mop) {
                    ("doc", Mop::Read(k, out)) => {
                        let (s, doc) = send(
                            http.get(format!("{base}/{tx}/document/{COLL}/k{k}")).bearer_auth(token),
                        )
                        .await?;
                        if !s.is_success() {
                            return Err(Step::Refused(format!("read k{k}: {doc}")));
                        }
                        *out = Some(vals(&doc));
                    }
                    ("doc", Mop::Append(k, v)) => {
                        let (s, doc) = send(
                            http.get(format!("{base}/{tx}/document/{COLL}/k{k}")).bearer_auth(token),
                        )
                        .await?;
                        if !s.is_success() {
                            return Err(Step::Refused(format!("read k{k} to append: {doc}")));
                        }
                        let mut list = vals(&doc);
                        list.push(*v);
                        let (s, out) = send(
                            http.put(format!("{base}/{tx}/document/{COLL}/k{k}"))
                                .bearer_auth(token)
                                .json(&json!({"vals": list})),
                        )
                        .await?;
                        if !s.is_success() {
                            return Err(Step::Refused(format!("append k{k}: {out}")));
                        }
                    }
                    (_, Mop::Read(k, out)) => {
                        let (s, r) = send(http.post(format!("{base}/{tx}/query")).bearer_auth(token).json(&json!({
                            "query": format!("FOR d IN {COLL} FILTER d._key == @k RETURN d.vals"),
                            "bind_vars": {"k": format!("k{k}")},
                        })))
                        .await?;
                        if !s.is_success() {
                            return Err(Step::Refused(format!("read k{k}: {r}")));
                        }
                        let list = r["result"][0].as_array().cloned().unwrap_or_default();
                        *out = Some(list.iter().filter_map(Value::as_i64).collect());
                    }
                    (_, Mop::Append(k, v)) => {
                        let (s, r) = send(http.post(format!("{base}/{tx}/query")).bearer_auth(token).json(&json!({
                            "query": format!(
                                "FOR d IN {COLL} FILTER d._key == @k UPDATE d WITH {{vals: PUSH(d.vals, @v)}} IN {COLL}"
                            ),
                            "bind_vars": {"k": format!("k{k}"), "v": *v},
                        })))
                        .await?;
                        if !s.is_success() || r["mutationCount"].as_u64() != Some(1) {
                            return Err(Step::Refused(format!("append k{k}: {r}")));
                        }
                    }
                }
            }
            Ok(())
        }
        .await;
        if let Err(step) = body {
            // Roll back what was staged; its answer does not matter.
            let _ = http.post(format!("{base}/{tx}/rollback")).bearer_auth(token).send().await;
            return Err(step);
        }
        let (s, r) = send(http.post(format!("{base}/{tx}/commit")).bearer_auth(token)).await?;
        if !s.is_success() {
            return Err(Step::Refused(format!("commit: {r}")));
        }
        Ok(())
    }
    .await;

    let (outcome, error) = match result {
        Ok(()) => (Outcome::Ok, None),
        Err(Step::Refused(e)) => (Outcome::Fail, Some(e)),
        Err(Step::Lost(e)) => (Outcome::Info, Some(e)),
    };
    Txn {
        process,
        invoked_ns,
        completed_ns: ctx.start.elapsed().as_nanos(),
        outcome,
        mops,
        error,
    }
}

async fn plain_read(http: &reqwest::Client, url: &str, token: &str, k: u64) -> Res<Vec<i64>> {
    let r = http
        .get(format!("{url}/_api/database/{DB}/document/{COLL}/k{k}"))
        .bearer_auth(token)
        .send()
        .await?;
    let doc: Value = r.json().await?;
    Ok(vals(&doc))
}

/// The history as Elle reads it: an invoke and a completion per
/// transaction, then one final read per key by an extra process.
fn write_history(
    txns: &[Txn],
    finals: &BTreeMap<u64, Vec<i64>>,
    clients: u32,
    path: &str,
) -> Res<()> {
    let mut events: Vec<(u128, Value)> = Vec::new();
    for t in txns {
        let invoke: Vec<Value> = t
            .mops
            .iter()
            .map(|m| match m {
                Mop::Read(k, _) => Mop::Read(*k, None).to_json(),
                other => other.to_json(),
            })
            .collect();
        events.push((t.invoked_ns, json!({"type": "invoke", "f": "txn", "process": t.process, "time": t.invoked_ns as u64, "value": invoke})));
        let (kind, value) = match t.outcome {
            Outcome::Ok => ("ok", t.mops.iter().map(Mop::to_json).collect::<Vec<_>>()),
            Outcome::Fail => ("fail", invoke_of(t)),
            Outcome::Info => ("info", invoke_of(t)),
        };
        events.push((t.completed_ns, json!({"type": kind, "f": "txn", "process": t.process, "time": t.completed_ns as u64, "value": value})));
    }
    let end = events.iter().map(|(t, _)| *t).max().unwrap_or(0) + 1;
    for (i, (k, list)) in finals.iter().enumerate() {
        let t = end + 2 * i as u128;
        events.push((t, json!({"type": "invoke", "f": "txn", "process": clients, "time": t as u64, "value": [["r", k, null]]})));
        events.push((t + 1, json!({"type": "ok", "f": "txn", "process": clients, "time": (t + 1) as u64, "value": [["r", k, list]]})));
    }
    events.sort_by_key(|(t, _)| *t);
    let ops: Vec<Value> = events
        .into_iter()
        .enumerate()
        .map(|(i, (_, mut op))| {
            op["index"] = json!(i);
            op
        })
        .collect();
    std::fs::write(path, serde_json::to_string(&ops)?)?;
    Ok(())
}

fn invoke_of(t: &Txn) -> Vec<Value> {
    t.mops
        .iter()
        .map(|m| match m {
            Mop::Read(k, _) => Mop::Read(*k, None).to_json(),
            other => other.to_json(),
        })
        .collect()
}

/// The anomalies that need no cycle search.
fn check(txns: &[Txn], finals: &BTreeMap<u64, Vec<i64>>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // Who appended what, and how it ended.
    let mut appended: HashMap<(u64, i64), Outcome> = HashMap::new();
    for t in txns {
        for m in &t.mops {
            if let Mop::Append(k, v) = m {
                appended.insert((*k, *v), t.outcome);
            }
        }
    }
    // Every committed read, and the final ones.
    let mut reads: HashMap<u64, Vec<Vec<i64>>> = HashMap::new();
    for t in txns.iter().filter(|t| t.outcome == Outcome::Ok) {
        let mut own: HashMap<u64, Vec<i64>> = HashMap::new();
        for m in &t.mops {
            match m {
                Mop::Append(k, v) => own.entry(*k).or_default().push(*v),
                Mop::Read(k, Some(list)) => {
                    reads.entry(*k).or_default().push(list.clone());
                    if let Some(mine) = own.get(k) {
                        if !list.ends_with(mine) {
                            out.push((
                                "internal".into(),
                                format!(
                                    "k{k}: process {} appended {mine:?}, then read {}",
                                    t.process,
                                    short(list)
                                ),
                            ));
                        }
                    }
                }
                Mop::Read(_, None) => {}
            }
        }
    }
    for (k, list) in finals {
        reads.entry(*k).or_default().push(list.clone());
    }
    for (k, lists) in &reads {
        for list in lists {
            let mut seen = HashSet::new();
            for v in list {
                if !seen.insert(v) {
                    out.push((
                        "duplicate-elements".into(),
                        format!("k{k}: {v} twice in {}", short(list)),
                    ));
                }
                match appended.get(&(*k, *v)) {
                    None => out.push((
                        "garbage-read".into(),
                        format!("k{k}: {v} was never appended"),
                    )),
                    Some(Outcome::Fail) => out.push((
                        "aborted-read".into(),
                        format!("k{k}: {v}, from a failed transaction, read"),
                    )),
                    _ => {}
                }
            }
        }
        let longest = lists
            .iter()
            .max_by_key(|l| l.len())
            .cloned()
            .unwrap_or_default();
        for list in lists {
            if !longest.starts_with(list) {
                out.push((
                    "incompatible-order".into(),
                    format!(
                        "k{k}: {} is not a prefix of {}",
                        short(list),
                        short(&longest)
                    ),
                ));
                break;
            }
        }
    }
    for (k, list) in finals {
        let present: HashSet<i64> = list.iter().copied().collect();
        for ((key, v), outcome) in &appended {
            if key == k && *outcome == Outcome::Ok && !present.contains(v) {
                out.push((
                    "lost-append".into(),
                    format!("k{k}: {v} committed, not in the final list"),
                ));
            }
        }
    }
    out
}

fn short(list: &[i64]) -> String {
    if list.len() <= 6 {
        format!("{list:?}")
    } else {
        format!("[{} values …, {:?}]", list.len(), &list[list.len() - 4..])
    }
}
