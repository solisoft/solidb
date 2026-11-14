//! SoliDB through its native driver, SDBQL with bind variables, query cache
//! off. People and products are documents keyed by their number; `knows`
//! and `bought` are edge collections with an index on each end.

use super::{err, Engine, Knows, Person, Res, Settings, Tally};
use serde_json::{json, Value};
use solidb_client::{SoliDBClient, SoliDBClientBuilder};
use std::collections::HashMap;
use std::time::Duration;

const DB: &str = "graph";

fn q_hops(depth: u32) -> String {
    format!(
        "FOR v IN 1..{depth} OUTBOUND @start knows \
         COLLECT AGGREGATE c = COUNT(), s = SUM(v.n) RETURN [c, s]"
    )
}
const Q_FILTERED: &str = "LET ns = (FOR v, e IN 2..2 OUTBOUND @start knows \
     OPTIONS { uniqueVertices: \"path\" } \
     FILTER e.since >= @year AND v.country == @country RETURN DISTINCT v.n) \
     RETURN [LENGTH(ns), SUM(ns)]";
const Q_FOF: &str = "LET buyers = (FOR b IN INBOUND @product bought RETURN b.n) \
     LET ns = (FOR v IN 1..2 OUTBOUND @start knows FILTER v.n IN buyers RETURN v.n) \
     RETURN [LENGTH(ns), SUM(ns)]";
const Q_SHORTEST: &str =
    "FOR v, e, p IN SHORTEST_PATH @from TO @to OUTBOUND knows RETURN LENGTH(p.edges)";

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

/// `[count, sum]`, the sum null when the count is zero.
fn tally(rows: Vec<Value>) -> Res<Tally> {
    let Some(row) = rows.first().and_then(Value::as_array) else {
        return err(format!("not a [count, sum] row: {rows:?}"));
    };
    let n = |v: Option<&Value>| v.and_then(Value::as_f64).unwrap_or(0.0) as u64;
    Ok((n(row.first()), n(row.get(1))))
}

fn person(n: u32) -> Value {
    json!(format!("people/{n}"))
}

impl Engine for Soli {
    type Conn = SoliDBClient;

    async fn open(s: &Settings) -> Res<Self> {
        Ok(Soli { s: s.clone() })
    }

    async fn version(&self) -> Res<String> {
        Ok("see meta.server_version".into())
    }

    async fn reset(&self) -> Res<()> {
        let mut c = connect(&self.s).await?;
        c.delete_database(DB).await.ok();
        c.create_database(DB).await?;
        for t in ["people", "products"] {
            c.create_collection(DB, t, None).await?;
        }
        for t in ["knows", "bought"] {
            c.create_collection(DB, t, Some("edge")).await?;
            // Indexes exist before the load, as in a running application.
            for end in ["_from", "_to"] {
                c.create_index(DB, t, &format!("idx{end}"), vec![end.to_string()], false, false)
                    .await?;
            }
        }
        Ok(())
    }

    async fn connect(&self) -> Res<SoliDBClient> {
        connect(&self.s).await
    }

    async fn load_people(c: &mut SoliDBClient, rows: Vec<Person>) -> Res<()> {
        let docs = rows
            .into_iter()
            .map(|(n, country, age)| json!({"_key": n.to_string(), "n": n, "country": country, "age": age}))
            .collect();
        c.bulk_insert(DB, "people", docs).await?;
        Ok(())
    }

    async fn load_products(c: &mut SoliDBClient, rows: Vec<u32>) -> Res<()> {
        let docs = rows
            .into_iter()
            .map(|n| json!({"_key": n.to_string(), "n": n}))
            .collect();
        c.bulk_insert(DB, "products", docs).await?;
        Ok(())
    }

    async fn load_knows(c: &mut SoliDBClient, rows: Vec<Knows>) -> Res<()> {
        let docs = rows
            .into_iter()
            .map(|(from, to, since, weight)| {
                json!({"_from": format!("people/{from}"), "_to": format!("people/{to}"),
                       "since": since, "weight": weight})
            })
            .collect();
        c.bulk_insert(DB, "knows", docs).await?;
        Ok(())
    }

    async fn load_bought(c: &mut SoliDBClient, rows: Vec<(u32, u32)>) -> Res<()> {
        let docs = rows
            .into_iter()
            .map(|(person, product)| {
                json!({"_from": format!("people/{person}"), "_to": format!("products/{product}")})
            })
            .collect();
        c.bulk_insert(DB, "bought", docs).await?;
        Ok(())
    }

    async fn after_load(&self) -> Res<()> {
        Ok(())
    }

    async fn hops(c: &mut SoliDBClient, start: u32, depth: u32) -> Res<Tally> {
        tally(query(c, &q_hops(depth), vec![("start", person(start))]).await?)
    }

    async fn filtered(c: &mut SoliDBClient, start: u32, year: i64, country: &str) -> Res<Tally> {
        let vars = vec![
            ("start", person(start)),
            ("year", json!(year)),
            ("country", json!(country)),
        ];
        tally(query(c, Q_FILTERED, vars).await?)
    }

    async fn fof_bought(c: &mut SoliDBClient, start: u32, product: u32) -> Res<Tally> {
        let vars = vec![
            ("start", person(start)),
            ("product", json!(format!("products/{product}"))),
        ];
        tally(query(c, Q_FOF, vars).await?)
    }

    async fn shortest(c: &mut SoliDBClient, from: u32, to: u32) -> Res<Option<u64>> {
        let rows = query(c, Q_SHORTEST, vec![("from", person(from)), ("to", person(to))]).await?;
        Ok(rows.first().and_then(Value::as_u64))
    }

    fn queries(_: &Settings) -> Vec<(&'static str, String)> {
        vec![
            ("hop1", q_hops(1)),
            ("hop2", q_hops(2)),
            ("hop3", q_hops(3)),
            ("filtered", Q_FILTERED.into()),
            ("fof_bought", Q_FOF.into()),
            ("shortest", Q_SHORTEST.into()),
        ]
    }
}
