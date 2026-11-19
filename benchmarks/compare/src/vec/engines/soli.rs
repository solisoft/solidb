//! SoliDB through its native driver: a document per vector,
//! `{_key, id, cat, emb}`, an HNSW index on `emb` (cosine) created over the
//! loaded documents, searched with `VECTOR_SEARCH` and its `ef` option.

use super::{Engine, Res, Settings};
use serde_json::{json, Value};
use solidb_client::{SoliDBClient, SoliDBClientBuilder};
use std::collections::HashMap;
use std::time::Duration;

const DB: &str = "vec";
const Q_SEARCH: &str =
    "FOR r IN VECTOR_SEARCH(\"items\", \"emb_idx\", @v, @k, {ef: @ef}) RETURN r.doc.id";
const Q_FILTERED: &str =
    "FOR r IN VECTOR_SEARCH(\"items\", \"emb_idx\", @v, @k, {ef: @ef, filter: {cat: @cat}}) RETURN r.doc.id";

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

/// An HTTP call (the driver has no command for vector indexes).
async fn http(s: &Settings, path: &str, body: Value) -> Res<Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3600))
        .build()?;
    let login: Value = client
        .post(format!("http://{}/auth/login", s.addr))
        .json(&json!({"username": "admin", "password": s.password}))
        .send()
        .await?
        .json()
        .await?;
    let token = login["token"]
        .as_str()
        .ok_or_else(|| format!("login: {login}"))?;
    let answer = client
        .post(format!("http://{}/_api/database/{DB}{path}", s.addr))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await?;
    let status = answer.status();
    let text = answer.text().await?;
    if !status.is_success() {
        return Err(format!("{path}: {status} {text}").into());
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

impl Engine for Soli {
    type Conn = SoliDBClient;

    async fn open(s: &Settings) -> Res<Self> {
        Ok(Soli { s: s.clone() })
    }

    async fn version(&self) -> Res<String> {
        Ok("see the image".into())
    }

    async fn reset(&self, _dim: usize) -> Res<()> {
        let mut c = connect(&self.s).await?;
        c.delete_database(DB).await.ok();
        c.create_database(DB).await?;
        c.create_collection(DB, "items", None).await?;
        Ok(())
    }

    async fn connect(&self) -> Res<SoliDBClient> {
        connect(&self.s).await
    }

    async fn insert(c: &mut SoliDBClient, rows: Vec<(u64, Vec<f32>)>) -> Res<()> {
        let docs = rows
            .into_iter()
            .map(|(id, v)| json!({"_key": format!("v{id}"), "id": id, "cat": id % crate::CATEGORIES, "emb": v}))
            .collect();
        c.bulk_insert(DB, "items", docs).await?;
        Ok(())
    }

    async fn build_index(&self) -> Res<()> {
        let mut body = json!({"name": "emb_idx", "field": "emb", "dimension": dimension(&self.s).await?,
                   "metric": "cosine", "m": self.s.m, "ef_construction": self.s.ef_construction});
        if let Some(q) = &self.s.quantization {
            body["quantization"] = json!(q);
        }
        let said = http(&self.s, "/vector/items", body).await?;
        eprintln!("[vec] solidb index: {said}");
        Ok(())
    }

    async fn search(
        c: &mut SoliDBClient,
        q: &[f32],
        k: usize,
        ef: usize,
        cat: Option<u64>,
    ) -> Res<Vec<u64>> {
        let mut vars: HashMap<String, Value> = HashMap::new();
        vars.insert("v".into(), json!(q));
        vars.insert("k".into(), json!(k));
        vars.insert("ef".into(), json!(ef));
        let text = match cat {
            Some(cat) => {
                vars.insert("cat".into(), json!(cat));
                Q_FILTERED
            }
            None => Q_SEARCH,
        };
        let rows = c.query_with_cache(DB, text, Some(vars), false).await?;
        Ok(rows.iter().filter_map(Value::as_u64).collect())
    }

    fn queries() -> Vec<(&'static str, String)> {
        vec![("search", Q_SEARCH.into()), ("filtered", Q_FILTERED.into())]
    }
}

/// The dimension of the vectors loaded, read back from one of them.
async fn dimension(s: &Settings) -> Res<usize> {
    let mut c = connect(s).await?;
    let rows = c
        .query_with_cache(
            DB,
            "FOR d IN items LIMIT 1 RETURN LENGTH(d.emb)",
            None,
            false,
        )
        .await?;
    rows.first()
        .and_then(Value::as_u64)
        .map(|d| d as usize)
        .ok_or_else(|| "no vector loaded".into())
}
