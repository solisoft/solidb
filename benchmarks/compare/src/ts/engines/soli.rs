//! SoliDB through its native driver. The points are documents of a
//! `timeseries` collection, `{series, ts, v}` with `ts` in milliseconds,
//! indexed on `[series, ts]` (and on `ts` when there are no column blocks).

use super::{err, Engine, Res, Settings};
use crate::{name, Point};
use serde_json::{json, Value};
use solidb_client::{SoliDBClient, SoliDBClientBuilder};
use std::collections::HashMap;
use std::time::Duration;

const DB: &str = "ts";

const Q_RANGE: &str = "FOR p IN points FILTER p.series == @s AND p.ts >= @lo AND p.ts < @hi \
     COLLECT AGGREGATE c = COUNT(), t = SUM(p.v) RETURN [c, t]";
const Q_BUCKET: &str = "FOR p IN points FILTER p.series == @s AND p.ts >= @lo AND p.ts < @hi \
     COLLECT b = TIME_BUCKET(p.ts, \"5m\") AGGREGATE a = AVG(p.v) RETURN a";
const Q_LAST: &str =
    "FOR p IN points FILTER p.series == @s SORT p.ts DESC LIMIT 1 RETURN [p.ts, p.v]";
const Q_WINDOW: &str = "FOR p IN points FILTER p.ts >= @lo AND p.ts < @hi \
     COLLECT AGGREGATE c = COUNT(), t = SUM(p.v) RETURN [c, t]";

pub struct Soli {
    s: Settings,
}

async fn connect(s: &Settings) -> Res<SoliDBClient> {
    let mut attempt = 0;
    loop {
        match SoliDBClientBuilder::new(&s.addr)
            .auth("_system", "admin", &s.password)
            .pool_size(1)
            .build()
            .await
        {
            Ok(c) => return Ok(c),
            Err(e) if attempt < 50 => {
                attempt += 1;
                eprintln!("solidb connect attempt {attempt}: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => return Err(e.to_string().into()),
        }
    }
}

async fn query(c: &mut SoliDBClient, q: &str, vars: Vec<(&str, Value)>) -> Res<Vec<Value>> {
    let vars: HashMap<String, Value> = vars.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    Ok(c.query_with_cache(DB, q, Some(vars), false).await?)
}

/// A call to the collection's column blocks over HTTP (the driver has no
/// command for them): the answer, or `None` from a server without them.
async fn blocks(
    s: &Settings,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Res<Option<String>> {
    let http = reqwest::Client::new();
    let login: Value = http
        .post(format!("http://{}/auth/login", s.addr))
        .json(&json!({"username": "admin", "password": s.password}))
        .send()
        .await?
        .json()
        .await?;
    let Some(token) = login["token"].as_str() else {
        return err(format!("login: {login}"));
    };
    let url = format!(
        "http://{}/_api/database/{DB}/collection/points/blocks{path}",
        s.addr
    );
    let mut request = http.request(method, url).bearer_auth(token);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let answer = request.send().await?;
    let status = answer.status();
    let text = answer.text().await?;
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
    {
        return Ok(None);
    }
    if !status.is_success() {
        return err(format!("column blocks: {status} {text}"));
    }
    Ok(Some(text))
}

fn pair(rows: Vec<Value>) -> Res<(u64, f64)> {
    let Some(row) = rows.first().and_then(Value::as_array) else {
        return err(format!("not a [count, sum] row: {rows:?}"));
    };
    let n = |v: Option<&Value>| v.and_then(Value::as_f64).unwrap_or(0.0);
    Ok((n(row.first()) as u64, n(row.get(1))))
}

impl Engine for Soli {
    type Conn = SoliDBClient;

    async fn open(s: &Settings) -> Res<Self> {
        Ok(Soli { s: s.clone() })
    }

    async fn version(&self) -> Res<String> {
        Ok("see the image".into())
    }

    async fn reset(&self) -> Res<()> {
        let mut c = connect(&self.s).await?;
        c.delete_database(DB).await.ok();
        c.create_database(DB).await?;
        c.create_collection(DB, "points", Some("timeseries"))
            .await?;
        c.create_index(
            DB,
            "points",
            "by_series_ts",
            vec!["series".into(), "ts".into()],
            false,
            false,
        )
        .await?;
        // The index on the time alone serves the questions that name no
        // series. With column blocks they are answered from the blocks,
        // and what no block covers is read by key: the index would only be
        // written. SOLIDB_TS_INDEX=1 creates it all the same.
        let blocks_on = std::env::var("SOLIDB_TS_BLOCKS").as_deref() != Ok("0");
        if !blocks_on || std::env::var("SOLIDB_TS_INDEX").as_deref() == Ok("1") {
            c.create_index(DB, "points", "by_ts", vec!["ts".into()], false, false)
                .await?;
        }
        if blocks_on {
            let def = json!({"time": "ts", "fields": ["series", "v"]});
            if blocks(&self.s, reqwest::Method::PUT, "", Some(def))
                .await?
                .is_none()
            {
                eprintln!("column blocks: not on this server");
            }
        }
        Ok(())
    }

    async fn connect(&self) -> Res<SoliDBClient> {
        connect(&self.s).await
    }

    /// Seal the column blocks of what was loaded, where the server has them.
    async fn after_load(&self, _: u64) -> Res<()> {
        if std::env::var("SOLIDB_TS_BLOCKS").as_deref() == Ok("0") {
            return Ok(());
        }
        let began = std::time::Instant::now();
        if let Some(said) = blocks(&self.s, reqwest::Method::POST, "/seal", None).await? {
            eprintln!(
                "column blocks: {said} (what was left sealed in {:.1} s)",
                began.elapsed().as_secs_f64()
            );
        }
        Ok(())
    }

    async fn insert(c: &mut SoliDBClient, points: Vec<Point>) -> Res<()> {
        let docs = points
            .into_iter()
            .map(|(series, ts, v)| json!({"series": name(series), "ts": ts, "v": v}))
            .collect();
        c.bulk_insert(DB, "points", docs).await?;
        Ok(())
    }

    async fn range(c: &mut SoliDBClient, series: u32, lo: i64, hi: i64) -> Res<(u64, f64)> {
        let vars = vec![
            ("s", json!(name(series))),
            ("lo", json!(lo)),
            ("hi", json!(hi)),
        ];
        pair(query(c, Q_RANGE, vars).await?)
    }

    async fn bucket(c: &mut SoliDBClient, series: u32, lo: i64, hi: i64) -> Res<(u64, f64)> {
        let vars = vec![
            ("s", json!(name(series))),
            ("lo", json!(lo)),
            ("hi", json!(hi)),
        ];
        let rows = query(c, Q_BUCKET, vars).await?;
        Ok((
            rows.len() as u64,
            rows.iter().filter_map(Value::as_f64).sum(),
        ))
    }

    async fn last(c: &mut SoliDBClient, series: u32) -> Res<(i64, f64)> {
        let rows = query(c, Q_LAST, vec![("s", json!(name(series)))]).await?;
        let Some(row) = rows.first().and_then(Value::as_array) else {
            return err(format!("no last point: {rows:?}"));
        };
        Ok((
            row.first().and_then(Value::as_f64).unwrap_or(0.0) as i64,
            row.get(1).and_then(Value::as_f64).unwrap_or(f64::NAN),
        ))
    }

    async fn window_all(c: &mut SoliDBClient, lo: i64, hi: i64) -> Res<(u64, f64)> {
        pair(query(c, Q_WINDOW, vec![("lo", json!(lo)), ("hi", json!(hi))]).await?)
    }

    fn queries() -> Vec<(&'static str, String)> {
        vec![
            ("range", Q_RANGE.into()),
            ("bucket", Q_BUCKET.into()),
            ("last", Q_LAST.into()),
            ("window_all", Q_WINDOW.into()),
            (
                "insert",
                "bulk_insert of one document {series, ts, v}".into(),
            ),
        ]
    }
}
