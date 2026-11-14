//! PostgreSQL through tokio-postgres: the graph as two tables of people and
//! products and two tables of edges, indexed on both ends, and SQL with
//! prepared statements. Fixed depths are joins; the shortest path is a
//! recursive CTE, which is how PostgreSQL walks a graph.

use super::{Engine, Knows, Person, Res, Settings, Tally};
use tokio_postgres::{Client, NoTls, Statement};

const Q_HOP1: &str =
    "SELECT count(*), coalesce(sum(dst), 0)::bigint FROM knows WHERE src = $1";
const Q_HOP2: &str = "WITH h1 AS (SELECT dst FROM knows WHERE src = $1), \
     h2 AS (SELECT k.dst FROM knows k JOIN h1 ON k.src = h1.dst) \
     SELECT count(*), coalesce(sum(n), 0)::bigint \
     FROM (SELECT dst AS n FROM h1 UNION SELECT dst FROM h2) u WHERE n <> $1";
const Q_HOP3: &str = "WITH h1 AS (SELECT dst FROM knows WHERE src = $1), \
     h2 AS (SELECT DISTINCT k.dst FROM knows k JOIN h1 ON k.src = h1.dst), \
     h3 AS (SELECT k.dst FROM knows k JOIN h2 ON k.src = h2.dst) \
     SELECT count(*), coalesce(sum(n), 0)::bigint \
     FROM (SELECT dst AS n FROM h1 UNION SELECT dst FROM h2 UNION SELECT dst FROM h3) u \
     WHERE n <> $1";
const Q_FILTERED: &str = "SELECT count(*), coalesce(sum(n), 0)::bigint FROM ( \
     SELECT DISTINCT p.n FROM knows a JOIN knows b ON b.src = a.dst \
     JOIN people p ON p.n = b.dst \
     WHERE a.src = $1 AND b.since >= $2 AND p.country = $3 AND b.dst <> $1) u";
const Q_FOF: &str = "WITH h1 AS (SELECT dst FROM knows WHERE src = $1), \
     h2 AS (SELECT k.dst FROM knows k JOIN h1 ON k.src = h1.dst) \
     SELECT count(*), coalesce(sum(u.n), 0)::bigint \
     FROM (SELECT dst AS n FROM h1 UNION SELECT dst FROM h2) u \
     JOIN bought b ON b.person = u.n AND b.product = $2 WHERE u.n <> $1";
/// Breadth first, one level per iteration, to a fixed depth: `UNION` drops
/// a person already reached at the same depth, not one reached earlier.
const Q_SHORTEST: &str = "WITH RECURSIVE r(id, d) AS ( \
     SELECT $1::int, 0 UNION \
     SELECT k.dst, r.d + 1 FROM r JOIN knows k ON k.src = r.id WHERE r.d < 6) \
     SELECT min(d)::bigint FROM r WHERE id = $2";

pub struct Postgres {
    s: Settings,
}

pub struct Conn {
    client: Client,
    hops: [Statement; 3],
    filtered: Statement,
    fof: Statement,
    shortest: Statement,
}

async fn client(s: &Settings) -> Res<Client> {
    let (host, port) = s.addr.split_once(':').unwrap_or((&s.addr, "5432"));
    let config = format!(
        "host={host} port={port} user=postgres password={} dbname=postgres",
        s.password
    );
    let (client, connection) = tokio_postgres::connect(&config, NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("postgres connection: {e}");
        }
    });
    Ok(client)
}

fn tally(row: &tokio_postgres::Row) -> Tally {
    (row.get::<_, i64>(0) as u64, row.get::<_, i64>(1) as u64)
}

impl Engine for Postgres {
    type Conn = Conn;

    async fn open(s: &Settings) -> Res<Self> {
        Ok(Postgres { s: s.clone() })
    }

    async fn version(&self) -> Res<String> {
        let c = client(&self.s).await?;
        Ok(c.query_one("SHOW server_version", &[]).await?.get(0))
    }

    async fn reset(&self) -> Res<()> {
        let c = client(&self.s).await?;
        c.batch_execute(
            "DROP TABLE IF EXISTS bought, knows, products, people;
             CREATE TABLE people (n int PRIMARY KEY, country text NOT NULL, age int NOT NULL);
             CREATE TABLE products (n int PRIMARY KEY);
             CREATE TABLE knows (src int NOT NULL, dst int NOT NULL, since int NOT NULL,
                                 weight int NOT NULL, PRIMARY KEY (src, dst));
             CREATE INDEX knows_dst ON knows (dst);
             CREATE TABLE bought (person int NOT NULL, product int NOT NULL,
                                  PRIMARY KEY (person, product));
             CREATE INDEX bought_product ON bought (product);",
        )
        .await?;
        Ok(())
    }

    async fn connect(&self) -> Res<Conn> {
        let client = client(&self.s).await?;
        Ok(Conn {
            hops: [
                client.prepare(Q_HOP1).await?,
                client.prepare(Q_HOP2).await?,
                client.prepare(Q_HOP3).await?,
            ],
            filtered: client.prepare(Q_FILTERED).await?,
            fof: client.prepare(Q_FOF).await?,
            shortest: client.prepare(Q_SHORTEST).await?,
            client,
        })
    }

    async fn load_people(c: &mut Conn, rows: Vec<Person>) -> Res<()> {
        let n: Vec<i32> = rows.iter().map(|r| r.0 as i32).collect();
        let country: Vec<&str> = rows.iter().map(|r| r.1).collect();
        let age: Vec<i32> = rows.iter().map(|r| r.2 as i32).collect();
        c.client
            .execute(
                "INSERT INTO people SELECT * FROM unnest($1::int[], $2::text[], $3::int[])",
                &[&n, &country, &age],
            )
            .await?;
        Ok(())
    }

    async fn load_products(c: &mut Conn, rows: Vec<u32>) -> Res<()> {
        let n: Vec<i32> = rows.iter().map(|r| *r as i32).collect();
        c.client
            .execute("INSERT INTO products SELECT * FROM unnest($1::int[])", &[&n])
            .await?;
        Ok(())
    }

    async fn load_knows(c: &mut Conn, rows: Vec<Knows>) -> Res<()> {
        let src: Vec<i32> = rows.iter().map(|r| r.0 as i32).collect();
        let dst: Vec<i32> = rows.iter().map(|r| r.1 as i32).collect();
        let since: Vec<i32> = rows.iter().map(|r| r.2 as i32).collect();
        let weight: Vec<i32> = rows.iter().map(|r| r.3 as i32).collect();
        c.client
            .execute(
                "INSERT INTO knows SELECT * FROM unnest($1::int[], $2::int[], $3::int[], $4::int[])",
                &[&src, &dst, &since, &weight],
            )
            .await?;
        Ok(())
    }

    async fn load_bought(c: &mut Conn, rows: Vec<(u32, u32)>) -> Res<()> {
        let person: Vec<i32> = rows.iter().map(|r| r.0 as i32).collect();
        let product: Vec<i32> = rows.iter().map(|r| r.1 as i32).collect();
        c.client
            .execute(
                "INSERT INTO bought SELECT * FROM unnest($1::int[], $2::int[])",
                &[&person, &product],
            )
            .await?;
        Ok(())
    }

    async fn after_load(&self) -> Res<()> {
        client(&self.s).await?.batch_execute("ANALYZE").await?;
        Ok(())
    }

    async fn hops(c: &mut Conn, start: u32, depth: u32) -> Res<Tally> {
        let row = c
            .client
            .query_one(&c.hops[depth as usize - 1], &[&(start as i32)])
            .await?;
        Ok(tally(&row))
    }

    async fn filtered(c: &mut Conn, start: u32, year: i64, country: &str) -> Res<Tally> {
        let row = c
            .client
            .query_one(&c.filtered, &[&(start as i32), &(year as i32), &country])
            .await?;
        Ok(tally(&row))
    }

    async fn fof_bought(c: &mut Conn, start: u32, product: u32) -> Res<Tally> {
        let row = c
            .client
            .query_one(&c.fof, &[&(start as i32), &(product as i32)])
            .await?;
        Ok(tally(&row))
    }

    async fn shortest(c: &mut Conn, from: u32, to: u32) -> Res<Option<u64>> {
        let row = c
            .client
            .query_one(&c.shortest, &[&(from as i32), &(to as i32)])
            .await?;
        Ok(row.get::<_, Option<i64>>(0).map(|d| d as u64))
    }

    fn queries(_: &Settings) -> Vec<(&'static str, String)> {
        vec![
            ("hop1", Q_HOP1.into()),
            ("hop2", Q_HOP2.into()),
            ("hop3", Q_HOP3.into()),
            ("filtered", Q_FILTERED.into()),
            ("fof_bought", Q_FOF.into()),
            ("shortest", Q_SHORTEST.into()),
        ]
    }
}
