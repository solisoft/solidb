//! `faults crash`: write, kill the server with SIGKILL, restart it, check.
//!
//! Every cycle starts the server on the same data directory, runs writers
//! for a random time, kills the server mid-write and starts it again. Then it
//! checks everything every cycle so far wrote:
//!
//! - **Transactions** (HTTP API): each inserts three documents into `txdocs`
//!   and sets its writer's counter in `counters` to its sequence number. A
//!   writer's transactions are sequential, so after a crash the writer's
//!   transactions present must be exactly 0..=counter, each with its three
//!   documents (atomicity, within a collection and across two), and every
//!   acknowledged one must be there (commits are synced).
//! - **Single inserts** (driver, outside any transaction), sequential per
//!   writer: what survives of a writer must be a prefix of what it wrote (no
//!   hole), must include every acknowledged insert, and nothing it never
//!   sent. SOLIDB_DURABILITY=eventual only allows losing writes on a power
//!   loss, never on a process kill.
//! - **Indexes** agree with the documents: the keys a persistent index finds
//!   for a writer are exactly those its key range holds, a unique index finds
//!   its one document and refuses a duplicate, and the collection count
//!   equals the documents present.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use solidb_client::SoliDBClient;

use crate::server::{login, Server, PASSWORD};
use crate::{Res, Rng};

pub const DB: &str = "faults";
const PARTS: u64 = 3;
const OP_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Opts {
    pub cycles: u32,
    pub tx_writers: u32,
    pub single_writers: u32,
    /// Inserts per second per single writer, so a long run stays checkable.
    pub single_rate: u32,
    pub min_ms: u64,
    pub max_ms: u64,
    pub tx_isolation: String,
    pub seed: u64,
}

/// What one writer sent, and how each request ended. Writers are
/// sequential and stop at their first error, so `acked` is a count: every
/// sequence number below it was acknowledged.
#[derive(Debug, Default, Clone)]
pub struct Writer {
    pub id: String,
    pub acked: u64,
    /// The request in flight when the connection broke: written or not.
    pub unknown: Option<u64>,
    /// A request the server answered with an error: must not be there.
    pub refused: Option<(u64, String)>,
    /// When it stopped, against the moment the server was killed.
    pub stopped_before_kill: bool,
}

pub async fn run(server: &mut Server, opts: Opts, out: &mut dyn std::io::Write) -> Res<usize> {
    let http = reqwest::Client::builder().timeout(OP_TIMEOUT).build()?;
    let mut rng = Rng::new(opts.seed);
    let mut history: Vec<(Vec<Writer>, Vec<Writer>)> = Vec::new();
    let mut total_violations = 0;

    let ready = server.start(&http).await?;
    eprintln!("[faults] server ready in {ready:?}");
    setup(server).await?;

    for cycle in 0..opts.cycles {
        let token = login(&http, &server.url()).await?;
        let mut driver = connect(server).await?;
        // Each tx writer's counter exists before its first transaction.
        for c in 0..opts.tx_writers {
            driver
                .insert(
                    DB,
                    "counters",
                    Some(&format!("y{cycle}t{c}")),
                    json!({"seq": -1}),
                )
                .await?;
        }
        drop(driver);

        let run_for = Duration::from_millis(opts.min_ms + rng.below(opts.max_ms - opts.min_ms + 1));
        let killed_at = Arc::new(std::sync::OnceLock::<Instant>::new());
        let mut tasks = Vec::new();
        for c in 0..opts.tx_writers {
            let (http, url, token, iso) = (
                http.clone(),
                server.url(),
                token.clone(),
                opts.tx_isolation.clone(),
            );
            let killed = killed_at.clone();
            tasks.push(tokio::spawn(async move {
                tx_writer(http, url, token, iso, format!("y{cycle}t{c}"), killed).await
            }));
        }
        let mut singles = Vec::new();
        for c in 0..opts.single_writers {
            let addr = server.addr();
            let killed = killed_at.clone();
            let rate = opts.single_rate;
            singles.push(tokio::spawn(async move {
                single_writer(addr, format!("y{cycle}s{c}"), rate, killed).await
            }));
        }

        tokio::time::sleep(run_for).await;
        killed_at.set(Instant::now()).ok();
        server.kill().await?;

        let mut txw = Vec::new();
        for t in tasks {
            txw.push(t.await?);
        }
        let mut sw = Vec::new();
        for t in singles {
            sw.push(t.await?);
        }
        history.push((txw, sw));

        let restart = server.start(&http).await;
        let mut violations = Vec::new();
        let mut stats = json!({});
        match restart {
            Err(e) => violations.push(format!("server did not restart: {e}")),
            Ok(took) => {
                stats["restart_ms"] = json!(took.as_millis());
                match check(server, &history, cycle, &mut rng, &mut stats).await {
                    Ok(v) => violations.extend(v),
                    Err(e) => violations.push(format!("check failed to run: {e}")),
                }
            }
        }
        let (txw, sw) = history.last().unwrap();
        let line = json!({
            "cycle": cycle,
            "ran_ms": run_for.as_millis(),
            "tx": {
                "acked": txw.iter().map(|w| w.acked).sum::<u64>(),
                "in_flight": txw.iter().filter(|w| w.unknown.is_some()).count(),
                "stopped_before_kill": txw.iter().filter(|w| w.stopped_before_kill).map(|w| json!([w.id, w.refused])).collect::<Vec<_>>(),
            },
            "singles": {
                "acked": sw.iter().map(|w| w.acked).sum::<u64>(),
                "in_flight": sw.iter().filter(|w| w.unknown.is_some()).count(),
                "stopped_before_kill": sw.iter().filter(|w| w.stopped_before_kill).map(|w| json!([w.id, w.refused])).collect::<Vec<_>>(),
            },
            "stats": stats,
            "violations": violations,
        });
        writeln!(out, "{line}")?;
        out.flush()?;
        eprintln!(
            "[faults] cycle {cycle}: ran {} ms, {} violation(s)",
            run_for.as_millis(),
            violations.len()
        );
        for v in &violations {
            eprintln!("  - {v}");
        }
        total_violations += violations.len();
        if violations
            .iter()
            .any(|v| v.starts_with("server did not restart"))
        {
            break;
        }
    }
    server.kill().await?;
    Ok(total_violations)
}

async fn connect(server: &Server) -> Res<SoliDBClient> {
    let mut c = SoliDBClient::connect_with_pool(&server.addr(), 1).await?;
    c.auth("_system", "admin", PASSWORD).await?;
    Ok(c)
}

async fn setup(server: &Server) -> Res<()> {
    let mut c = connect(server).await?;
    c.create_database(DB).await?;
    for coll in ["txdocs", "counters", "singles"] {
        c.create_collection(DB, coll, None).await?;
    }
    c.create_index(DB, "txdocs", "idx_tx", vec!["tx".into()], false, false)
        .await?;
    c.create_index(
        DB,
        "singles",
        "idx_client",
        vec!["client".into()],
        false,
        false,
    )
    .await?;
    c.create_index(DB, "singles", "idx_uid", vec!["uid".into()], true, false)
        .await?;
    Ok(())
}

fn after_kill(killed: &std::sync::OnceLock<Instant>) -> bool {
    killed.get().is_some()
}

/// Transactions over HTTP until the server goes away.
async fn tx_writer(
    http: reqwest::Client,
    url: String,
    token: String,
    isolation: String,
    id: String,
    killed: Arc<std::sync::OnceLock<Instant>>,
) -> Writer {
    let mut w = Writer {
        id: id.clone(),
        ..Default::default()
    };
    let base = format!("{url}/_api/database/{DB}/transaction");
    let mut seq = 0u64;
    loop {
        // Before the commit, any failure means the transaction never
        // committed: it is simply absent.
        let staged: Res<String> = async {
            let r = http.post(format!("{base}/begin")).bearer_auth(&token)
                .json(&json!({"isolationLevel": isolation})).send().await?;
            let v = answered(r).await?;
            let tx = v["id"].as_str().ok_or("begin: no id")?.to_string();
            for part in 0..PARTS {
                let r = http.post(format!("{base}/{tx}/document/txdocs")).bearer_auth(&token)
                    .json(&json!({"_key": format!("{id}-{seq}-{part}"), "tx": format!("{id}-{seq}"), "part": part}))
                    .send().await?;
                answered(r).await?;
            }
            let r = http.put(format!("{base}/{tx}/document/counters/{id}")).bearer_auth(&token)
                .json(&json!({"seq": seq})).send().await?;
            answered(r).await?;
            Ok(tx)
        }
        .await;
        let tx = match staged {
            Ok(tx) => tx,
            Err(e) => {
                if !after_kill(&killed) {
                    w.refused = Some((seq, format!("before commit: {e}")));
                    w.stopped_before_kill = true;
                }
                return w;
            }
        };
        match http
            .post(format!("{base}/{tx}/commit"))
            .bearer_auth(&token)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => {
                w.acked += 1;
                seq += 1;
            }
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                w.refused = Some((seq, format!("commit answered {status}: {body}")));
                w.stopped_before_kill = !after_kill(&killed);
                return w;
            }
            Err(e) => {
                w.unknown = Some(seq);
                if !after_kill(&killed) {
                    w.refused = Some((seq, format!("commit: no answer before the kill: {e}")));
                    w.stopped_before_kill = true;
                }
                return w;
            }
        }
    }
}

async fn answered(r: reqwest::Response) -> Res<Value> {
    let status = r.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", r.text().await.unwrap_or_default()).into());
    }
    Ok(r.json().await?)
}

/// Driver inserts outside any transaction, paced at `rate` per second.
async fn single_writer(
    addr: String,
    id: String,
    rate: u32,
    killed: Arc<std::sync::OnceLock<Instant>>,
) -> Writer {
    let mut w = Writer {
        id: id.clone(),
        ..Default::default()
    };
    let mut c = match SoliDBClient::connect_with_pool(&addr, 1).await {
        Ok(c) => c,
        Err(e) => {
            w.refused = Some((0, format!("connect: {e}")));
            w.stopped_before_kill = !after_kill(&killed);
            return w;
        }
    };
    if let Err(e) = c.auth("_system", "admin", PASSWORD).await {
        w.refused = Some((0, format!("auth: {e}")));
        w.stopped_before_kill = !after_kill(&killed);
        return w;
    }
    let start = Instant::now();
    let mut seq = 0u64;
    loop {
        let key = format!("{id}-{seq}");
        let doc = json!({"uid": key, "client": id, "seq": seq});
        match tokio::time::timeout(OP_TIMEOUT, c.insert(DB, "singles", Some(&key), doc)).await {
            Ok(Ok(_)) => {
                w.acked += 1;
                seq += 1;
            }
            Ok(Err(e)) if is_connection_error(&e) => {
                w.unknown = Some(seq);
                if !after_kill(&killed) {
                    w.refused = Some((seq, format!("connection lost before the kill: {e}")));
                    w.stopped_before_kill = true;
                }
                return w;
            }
            Ok(Err(e)) => {
                w.refused = Some((seq, e.to_string()));
                w.stopped_before_kill = !after_kill(&killed);
                return w;
            }
            Err(_) => {
                w.unknown = Some(seq);
                w.refused = Some((seq, "no answer in 10 s".into()));
                w.stopped_before_kill = !after_kill(&killed);
                return w;
            }
        }
        // Pace: never ahead of `rate` per second.
        let due = Duration::from_secs_f64(seq as f64 / rate as f64);
        let elapsed = start.elapsed();
        if due > elapsed + Duration::from_millis(2) {
            tokio::time::sleep(due - elapsed).await;
        }
    }
}

fn is_connection_error(e: &solidb_client::protocol::DriverError) -> bool {
    use solidb_client::protocol::DriverError as E;
    matches!(e, E::ConnectionError(_)) || {
        let s = e.to_string().to_lowercase();
        s.contains("connection")
            || s.contains("broken pipe")
            || s.contains("reset")
            || s.contains("eof")
    }
}

async fn query(c: &mut SoliDBClient, q: &str, vars: Value) -> Res<Vec<Value>> {
    let vars: HashMap<String, Value> = serde_json::from_value(vars)?;
    Ok(c.query_with_cache(DB, q, Some(vars), false).await?)
}

/// The sequence numbers present for a writer, read from the documents by key
/// range (the document store, no secondary index).
async fn present(
    c: &mut SoliDBClient,
    coll: &str,
    writer: &str,
    field: &str,
) -> Res<Vec<(String, Value)>> {
    let rows = query(
        c,
        &format!(
            "FOR d IN {coll} FILTER d._key >= @lo AND d._key < @hi RETURN [d._key, d.{field}]"
        ),
        json!({"lo": format!("{writer}-"), "hi": format!("{writer}.")}),
    )
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r[0].as_str().unwrap_or_default().to_string(), r[1].clone()))
        .collect())
}

/// Is `got` exactly 0..n, and what is n (the count)?
fn prefix(got: &BTreeSet<u64>) -> Result<u64, String> {
    let n = got.len() as u64;
    if got.iter().next_back().is_none_or(|&max| max + 1 == n) {
        Ok(n)
    } else {
        let missing: Vec<u64> = (0..*got.iter().next_back().unwrap())
            .filter(|s| !got.contains(s))
            .take(5)
            .collect();
        Err(format!(
            "holes, first missing: {missing:?} (present {n}, highest {})",
            got.iter().next_back().unwrap()
        ))
    }
}

async fn check(
    server: &Server,
    history: &[(Vec<Writer>, Vec<Writer>)],
    this_cycle: u32,
    rng: &mut Rng,
    stats: &mut Value,
) -> Res<Vec<String>> {
    let mut c = connect(server).await?;
    let mut v = Vec::new();
    let mut singles_present = 0u64;
    let mut tx_present = 0u64;

    for (cycle, (txw, sw)) in history.iter().enumerate() {
        let fresh = cycle as u32 == this_cycle;
        for w in txw {
            // Group the writer's txdocs by transaction.
            let rows = present(&mut c, "txdocs", &w.id, "tx").await?;
            let mut parts: HashMap<u64, u64> = HashMap::new();
            for (key, tx) in &rows {
                let seq = key
                    .split('-')
                    .nth(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(u64::MAX);
                if tx.as_str() != Some(&format!("{}-{seq}", w.id)) {
                    v.push(format!("{key}: field tx is {tx}, not {}-{seq}", w.id));
                }
                *parts.entry(seq).or_default() += 1;
            }
            for (seq, n) in &parts {
                if *n != PARTS {
                    v.push(format!(
                        "transaction {}-{seq}: {n} of its {PARTS} documents present (atomicity)",
                        w.id
                    ));
                }
            }
            let seqs: BTreeSet<u64> = parts.keys().copied().collect();
            let n = match prefix(&seqs) {
                Ok(n) => n,
                Err(e) => {
                    v.push(format!("transactions of {}: {e}", w.id));
                    seqs.len() as u64
                }
            };
            tx_present += n;
            let counter = query(
                &mut c,
                "FOR d IN counters FILTER d._key == @k RETURN d.seq",
                json!({"k": w.id}),
            )
            .await?;
            let counter = counter.first().and_then(Value::as_i64);
            if counter != Some(n as i64 - 1) {
                v.push(format!(
                    "{}: counter says {:?} but transactions 0..{n} are present (atomicity across collections)",
                    w.id, counter
                ));
            }
            if n < w.acked {
                v.push(format!(
                    "{}: {} acknowledged transaction(s) lost (present {n}, acknowledged {})",
                    w.id,
                    w.acked - n,
                    w.acked
                ));
            }
            let sent = w.acked + u64::from(w.unknown.is_some());
            if n > sent {
                v.push(format!(
                    "{}: {n} transactions present but only {sent} sent or in flight",
                    w.id
                ));
            }
            if let Some((seq, why)) = &w.refused {
                if seqs.contains(seq) && !why.starts_with("commit: no answer") {
                    v.push(format!("{}-{seq}: refused ({why}) but present", w.id));
                }
            }
            // The index on `tx` finds exactly the documents of a sample of
            // transactions, and nothing for the next one.
            if fresh || rng.below(4) == 0 {
                for seq in [n.saturating_sub(1), n / 2, n] {
                    let tx = format!("{}-{seq}", w.id);
                    let by_index = query(
                        &mut c,
                        "FOR d IN txdocs FILTER d.tx == @t RETURN d._key",
                        json!({"t": tx}),
                    )
                    .await?;
                    let want = if seq < n { PARTS as usize } else { 0 };
                    if by_index.len() != want {
                        v.push(format!(
                            "index idx_tx finds {} document(s) for {tx}, the documents say {want}",
                            by_index.len()
                        ));
                    }
                }
            }
        }
        for w in sw {
            let rows = present(&mut c, "singles", &w.id, "seq").await?;
            let seqs: BTreeSet<u64> = rows.iter().filter_map(|(_, s)| s.as_u64()).collect();
            let n = match prefix(&seqs) {
                Ok(n) => n,
                Err(e) => {
                    v.push(format!("inserts of {}: {e}", w.id));
                    seqs.len() as u64
                }
            };
            singles_present += n;
            if n < w.acked {
                v.push(format!("{}: {} acknowledged insert(s) lost after a process kill (present {n}, acknowledged {})", w.id, w.acked - n, w.acked));
            }
            let sent = w.acked + u64::from(w.unknown.is_some());
            if n > sent {
                v.push(format!(
                    "{}: {n} inserts present but only {sent} sent",
                    w.id
                ));
            }
            // The persistent index on `client` finds exactly these keys.
            let keys: BTreeSet<String> = rows.iter().map(|(k, _)| k.clone()).collect();
            let by_index: BTreeSet<String> = query(
                &mut c,
                "FOR d IN singles FILTER d.client == @w RETURN d._key",
                json!({"w": w.id}),
            )
            .await?
            .into_iter()
            .filter_map(|k| k.as_str().map(str::to_string))
            .collect();
            if by_index != keys {
                let extra: Vec<_> = by_index.difference(&keys).take(3).collect();
                let missing: Vec<_> = keys.difference(&by_index).take(3).collect();
                v.push(format!(
                    "index idx_client for {}: {} keys, documents {} (extra {extra:?}, missing {missing:?})",
                    w.id, by_index.len(), keys.len()
                ));
            }
            // The unique index finds a sample of documents, nothing for the
            // next sequence number, and refuses a duplicate.
            if n > 0 && (fresh || rng.below(4) == 0) {
                for seq in [rng.below(n), n - 1, n] {
                    let uid = format!("{}-{seq}", w.id);
                    let found = query(
                        &mut c,
                        "FOR d IN singles FILTER d.uid == @u RETURN d._key",
                        json!({"u": uid}),
                    )
                    .await?;
                    let want = usize::from(seq < n);
                    if found.len() != want {
                        v.push(format!("index idx_uid finds {} document(s) for {uid}, the documents say {want}", found.len()));
                    }
                }
                let uid = format!("{}-{}", w.id, rng.below(n));
                let dup_key = format!("dup-{this_cycle}-{}", w.id);
                if c.insert(
                    DB,
                    "singles",
                    Some(&dup_key),
                    json!({"uid": uid, "client": "dup", "seq": -1}),
                )
                .await
                .is_ok()
                {
                    v.push(format!(
                        "unique index idx_uid accepted a second document with uid {uid}"
                    ));
                    let _ = query(&mut c, "REMOVE @k IN singles", json!({"k": dup_key})).await;
                }
            }
        }
    }

    // The collection counts agree with the documents.
    for (coll, want) in [("singles", singles_present), ("txdocs", tx_present * PARTS)] {
        let counted = query(
            &mut c,
            &format!("FOR d IN {coll} COLLECT WITH COUNT INTO n RETURN n"),
            json!({}),
        )
        .await?;
        let counted = counted.first().and_then(Value::as_u64).unwrap_or(u64::MAX);
        if counted != want {
            v.push(format!(
                "{coll}: COLLECT WITH COUNT says {counted}, the writers' key ranges hold {want}"
            ));
        }
    }
    stats["singles_present"] = json!(singles_present);
    stats["transactions_present"] = json!(tx_present);
    Ok(v)
}
