//! Neo4j and Memgraph through the Bolt protocol (neo4rs), Cypher with
//! parameters. The two share every query but the shortest path and the
//! index statements, which `--flavor` selects.

use super::{Engine, Knows, Person, Res, Settings, Tally};
use neo4rs::{query, ConfigBuilder, Graph, Query};

fn q_hops(depth: u32) -> String {
    format!(
        "MATCH (a:Person {{n: $n}})-[:KNOWS*1..{depth}]->(b) WHERE b <> a \
         WITH DISTINCT b RETURN count(b) AS c, coalesce(sum(b.n), 0) AS s"
    )
}
const Q_FILTERED: &str = "MATCH (a:Person {n: $n})-[:KNOWS]->(m)-[e:KNOWS]->(b) \
     WHERE e.since >= $year AND b.country = $country AND b <> a \
     WITH DISTINCT b RETURN count(b) AS c, coalesce(sum(b.n), 0) AS s";
const Q_FOF: &str = "MATCH (a:Person {n: $n})-[:KNOWS*1..2]->(b)-[:BOUGHT]->(:Product {n: $product}) \
     WHERE b <> a WITH DISTINCT b RETURN count(b) AS c, coalesce(sum(b.n), 0) AS s";
const Q_SHORTEST_NEO4J: &str = "MATCH (a:Person {n: $from}), (b:Person {n: $to}) \
     MATCH p = shortestPath((a)-[:KNOWS*..15]->(b)) RETURN length(p) AS l";
const Q_SHORTEST_MEMGRAPH: &str = "MATCH p = (a:Person {n: $from})-[:KNOWS *BFS ..15]->(b:Person {n: $to}) \
     RETURN size(relationships(p)) AS l";

pub struct Bolt {
    graph: Graph,
    memgraph: bool,
}

pub struct Conn {
    graph: Graph,
    shortest: &'static str,
}

async fn tally(graph: &Graph, q: Query) -> Res<Tally> {
    let mut rows = graph.execute(q).await?;
    let row = rows.next().await?.ok_or("no row")?;
    Ok((row.get::<i64>("c")? as u64, row.get::<i64>("s")? as u64))
}

fn ints(values: impl Iterator<Item = i64>) -> Vec<i64> {
    values.collect()
}

impl Engine for Bolt {
    type Conn = Conn;

    async fn open(s: &Settings) -> Res<Self> {
        let memgraph = s.flavor == "memgraph";
        let config = ConfigBuilder::default()
            // neo4rs asks for the database `neo4j` unless told otherwise.
            .db(if memgraph { "memgraph" } else { "neo4j" })
            .uri(s.addr.as_str())
            .user(if memgraph { "" } else { "neo4j" })
            .password(if memgraph { "" } else { s.password.as_str() })
            .max_connections(64)
            .fetch_size(1000)
            .build()?;
        Ok(Bolt {
            graph: Graph::connect(config).await?,
            memgraph,
        })
    }

    async fn version(&self) -> Res<String> {
        let q = if self.memgraph {
            "SHOW VERSION"
        } else {
            "CALL dbms.components() YIELD versions RETURN versions[0] AS version"
        };
        let mut rows = self.graph.execute(query(q)).await?;
        let row = rows.next().await?.ok_or("no version row")?;
        Ok(row.get::<String>("version")?)
    }

    async fn reset(&self) -> Res<()> {
        // A fresh container: nothing to delete.
        let statements: &[&str] = if self.memgraph {
            &[
                "CREATE INDEX ON :Person(n)",
                "CREATE INDEX ON :Product(n)",
            ]
        } else {
            &[
                "CREATE INDEX person_n IF NOT EXISTS FOR (p:Person) ON (p.n)",
                "CREATE INDEX product_n IF NOT EXISTS FOR (p:Product) ON (p.n)",
            ]
        };
        for statement in statements {
            self.graph.run(query(statement)).await?;
        }
        if !self.memgraph {
            self.graph.run(query("CALL db.awaitIndexes(300)")).await?;
        }
        Ok(())
    }

    async fn connect(&self) -> Res<Conn> {
        Ok(Conn {
            graph: self.graph.clone(),
            shortest: if self.memgraph {
                Q_SHORTEST_MEMGRAPH
            } else {
                Q_SHORTEST_NEO4J
            },
        })
    }

    async fn load_people(c: &mut Conn, rows: Vec<Person>) -> Res<()> {
        let q = query(
            "UNWIND range(0, size($n) - 1) AS i \
             CREATE (:Person {n: $n[i], country: $country[i], age: $age[i]})",
        )
        .param("n", ints(rows.iter().map(|r| r.0 as i64)))
        .param(
            "country",
            rows.iter().map(|r| r.1.to_string()).collect::<Vec<_>>(),
        )
        .param("age", ints(rows.iter().map(|r| r.2)));
        Ok(c.graph.run(q).await?)
    }

    async fn load_products(c: &mut Conn, rows: Vec<u32>) -> Res<()> {
        let q = query("UNWIND $n AS n CREATE (:Product {n: n})")
            .param("n", ints(rows.iter().map(|r| *r as i64)));
        Ok(c.graph.run(q).await?)
    }

    async fn load_knows(c: &mut Conn, rows: Vec<Knows>) -> Res<()> {
        let q = query(
            "UNWIND range(0, size($src) - 1) AS i \
             MATCH (a:Person {n: $src[i]}), (b:Person {n: $dst[i]}) \
             CREATE (a)-[:KNOWS {since: $since[i], weight: $weight[i]}]->(b)",
        )
        .param("src", ints(rows.iter().map(|r| r.0 as i64)))
        .param("dst", ints(rows.iter().map(|r| r.1 as i64)))
        .param("since", ints(rows.iter().map(|r| r.2)))
        .param("weight", ints(rows.iter().map(|r| r.3)));
        Ok(c.graph.run(q).await?)
    }

    async fn load_bought(c: &mut Conn, rows: Vec<(u32, u32)>) -> Res<()> {
        let q = query(
            "UNWIND range(0, size($person) - 1) AS i \
             MATCH (a:Person {n: $person[i]}), (p:Product {n: $product[i]}) \
             CREATE (a)-[:BOUGHT]->(p)",
        )
        .param("person", ints(rows.iter().map(|r| r.0 as i64)))
        .param("product", ints(rows.iter().map(|r| r.1 as i64)));
        Ok(c.graph.run(q).await?)
    }

    async fn after_load(&self) -> Res<()> {
        Ok(())
    }

    async fn hops(c: &mut Conn, start: u32, depth: u32) -> Res<Tally> {
        tally(&c.graph, query(&q_hops(depth)).param("n", start as i64)).await
    }

    async fn filtered(c: &mut Conn, start: u32, year: i64, country: &str) -> Res<Tally> {
        let q = query(Q_FILTERED)
            .param("n", start as i64)
            .param("year", year)
            .param("country", country);
        tally(&c.graph, q).await
    }

    async fn fof_bought(c: &mut Conn, start: u32, product: u32) -> Res<Tally> {
        let q = query(Q_FOF)
            .param("n", start as i64)
            .param("product", product as i64);
        tally(&c.graph, q).await
    }

    async fn shortest(c: &mut Conn, from: u32, to: u32) -> Res<Option<u64>> {
        let q = query(c.shortest)
            .param("from", from as i64)
            .param("to", to as i64);
        let mut rows = c.graph.execute(q).await?;
        match rows.next().await? {
            Some(row) => Ok(Some(row.get::<i64>("l")? as u64)),
            None => Ok(None),
        }
    }

    fn queries(s: &Settings) -> Vec<(&'static str, String)> {
        let shortest = if s.flavor == "memgraph" {
            Q_SHORTEST_MEMGRAPH
        } else {
            Q_SHORTEST_NEO4J
        };
        vec![
            ("hop1", q_hops(1)),
            ("hop2", q_hops(2)),
            ("hop3", q_hops(3)),
            ("filtered", Q_FILTERED.into()),
            ("fof_bought", Q_FOF.into()),
            ("shortest", shortest.into()),
        ]
    }
}
