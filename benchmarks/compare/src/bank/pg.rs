//! PostgreSQL: accounts in a typed table, one connection per worker, every
//! statement prepared once per connection. Durability is the server's
//! `synchronous_commit`, set on its command line by bank.sh.
//!
//! Three ways to move money, each a transaction of five or six statements:
//!
//! - `update`: the idiom, at READ COMMITTED. A conditional
//!   `UPDATE … SET balance = balance - $2 WHERE id = $1 AND balance >= $2`,
//!   then the credit, both in account order so that two transfers between
//!   the same accounts never deadlock; then the ledger row.
//! - `serializable`: read both balances, decide in the client, write the new
//!   balances (in account order), at SERIALIZABLE. Serialization failures
//!   are retried.
//! - `naive`: the same read-then-write at READ COMMITTED. Not safe; a
//!   control that shows the checks catch lost updates.

use super::{Attempt, Res, Totals, Transfer};
use std::collections::HashMap;
use tokio_postgres::error::SqlState;
use tokio_postgres::{Client, IsolationLevel, NoTls, Statement};

pub struct Pg {
    url: String,
}

pub struct Conn {
    c: Client,
    debit: Statement,
    credit: Statement,
    read: Statement,
    set: Statement,
    ledger: Statement,
    audit: Statement,
}

impl Pg {
    pub fn new(addr: &str, password: &str) -> Self {
        let (host, port) = addr.split_once(':').unwrap_or((addr, "5432"));
        Pg {
            url: format!(
                "host={host} port={port} user=postgres password={password} dbname=postgres"
            ),
        }
    }

    async fn client(&self) -> Res<Client> {
        let (c, conn) = tokio_postgres::connect(&self.url, NoTls).await?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        Ok(c)
    }

    pub async fn ready(&self) -> Res<()> {
        let c = self.client().await?;
        c.simple_query("SELECT 1").await?;
        Ok(())
    }

    pub async fn connect(&self) -> Res<Conn> {
        let c = self.client().await?;
        Ok(Conn {
            debit: c
                .prepare("UPDATE accounts SET balance = balance - $2 WHERE id = $1 AND balance >= $2")
                .await?,
            credit: c
                .prepare("UPDATE accounts SET balance = balance + $2 WHERE id = $1")
                .await?,
            read: c
                .prepare("SELECT id, balance FROM accounts WHERE id = ANY($1)")
                .await?,
            set: c
                .prepare("UPDATE accounts SET balance = $2 WHERE id = $1")
                .await?,
            ledger: c
                .prepare(
                    "INSERT INTO transfers (id, w, seq, src, dst, amount) VALUES ($1, $2, $3, $4, $5, $6)",
                )
                .await?,
            audit: c
                .prepare("SELECT coalesce(sum(balance), 0)::bigint, count(*), coalesce(min(balance), 0) FROM accounts")
                .await?,
            c,
        })
    }

    pub async fn reset(&self, accounts: u32, initial: i64) -> Res<()> {
        let c = self.client().await?;
        c.batch_execute(
            "DROP TABLE IF EXISTS accounts; DROP TABLE IF EXISTS transfers; \
             CREATE TABLE accounts (id int PRIMARY KEY, balance bigint NOT NULL); \
             CREATE TABLE transfers (id text PRIMARY KEY, w int NOT NULL, seq bigint NOT NULL, \
               src int NOT NULL, dst int NOT NULL, amount bigint NOT NULL);",
        )
        .await?;
        c.execute(
            "INSERT INTO accounts SELECT g, $1::bigint FROM generate_series(0, $2::int - 1) g",
            &[&initial, &(accounts as i32)],
        )
        .await?;
        c.batch_execute("VACUUM ANALYZE accounts").await?;
        Ok(())
    }

    pub async fn balances(&self) -> Res<Vec<(u32, i64)>> {
        let c = self.client().await?;
        let rows = c.query("SELECT id, balance FROM accounts", &[]).await?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<_, i32>(0) as u32, r.get::<_, i64>(1)))
            .collect())
    }

    /// Per account: (debited, credited) by the ledger.
    pub async fn ledger_by_account(&self) -> Res<HashMap<u32, (i64, i64)>> {
        let c = self.client().await?;
        let mut out: HashMap<u32, (i64, i64)> = HashMap::new();
        for r in c
            .query(
                "SELECT src, sum(amount)::bigint FROM transfers GROUP BY src",
                &[],
            )
            .await?
        {
            out.entry(r.get::<_, i32>(0) as u32).or_default().0 = r.get(1);
        }
        for r in c
            .query(
                "SELECT dst, sum(amount)::bigint FROM transfers GROUP BY dst",
                &[],
            )
            .await?
        {
            out.entry(r.get::<_, i32>(0) as u32).or_default().1 = r.get(1);
        }
        Ok(out)
    }

    /// Per worker: (rows, sum of seq, sum of seq²).
    pub async fn ledger_by_worker(&self) -> Res<HashMap<u32, (u64, u64, u64)>> {
        let c = self.client().await?;
        let rows = c
            .query(
                "SELECT w, count(*), sum(seq)::bigint, sum(seq * seq)::bigint FROM transfers GROUP BY w",
                &[],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get::<_, i32>(0) as u32,
                    (
                        r.get::<_, i64>(1) as u64,
                        r.get::<_, i64>(2) as u64,
                        r.get::<_, i64>(3) as u64,
                    ),
                )
            })
            .collect())
    }

    pub async fn present(&self, id: &str) -> Res<bool> {
        let c = self.client().await?;
        Ok(!c
            .query("SELECT 1 FROM transfers WHERE id = $1", &[&id])
            .await?
            .is_empty())
    }
}

/// What an error before the commit means: refused (and rolled back), or no
/// answer (the transaction never committed either way).
fn refused(e: tokio_postgres::Error) -> Attempt {
    match e.as_db_error() {
        Some(db)
            if *db.code() == SqlState::T_R_SERIALIZATION_FAILURE
                || *db.code() == SqlState::T_R_DEADLOCK_DETECTED =>
        {
            Attempt::Conflict(db.message().to_string())
        }
        Some(db) => Attempt::Failed(format!("{}: {}", db.code().code(), db.message())),
        None => Attempt::Lost(e.to_string()),
    }
}

fn commit_failed(e: tokio_postgres::Error) -> Attempt {
    match e.as_db_error() {
        Some(_) => refused(e),
        None => Attempt::Unknown(e.to_string()),
    }
}

macro_rules! step {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return refused(e),
        }
    };
}

impl Conn {
    pub async fn transfer(&mut self, mode: &str, t: &Transfer) -> Attempt {
        let isolation = if mode == "serializable" {
            IsolationLevel::Serializable
        } else {
            IsolationLevel::ReadCommitted
        };
        let tx = step!(
            self.c
                .build_transaction()
                .isolation_level(isolation)
                .start()
                .await
        );
        let (from, to) = (t.from as i32, t.to as i32);
        match mode {
            "update" => {
                // In account order: a debit that finds too little money
                // rolls the credit back with it.
                let debit_first = from < to;
                if !debit_first {
                    step!(tx.execute(&self.credit, &[&to, &t.amount]).await);
                }
                if step!(tx.execute(&self.debit, &[&from, &t.amount]).await) == 0 {
                    let _ = tx.rollback().await;
                    return Attempt::Insufficient;
                }
                if debit_first {
                    step!(tx.execute(&self.credit, &[&to, &t.amount]).await);
                }
            }
            _ => {
                let rows = step!(tx.query(&self.read, &[&vec![from, to]]).await);
                let bal = |id: i32| {
                    rows.iter()
                        .find(|r| r.get::<_, i32>(0) == id)
                        .map(|r| r.get::<_, i64>(1))
                };
                let (Some(src), Some(dst)) = (bal(from), bal(to)) else {
                    return Attempt::Failed("account missing".into());
                };
                if src < t.amount {
                    let _ = tx.rollback().await;
                    return Attempt::Insufficient;
                }
                // In account order, as in `update`: no deadlocks.
                let mut writes = [(from, src - t.amount), (to, dst + t.amount)];
                writes.sort_unstable();
                for (id, balance) in writes {
                    step!(tx.execute(&self.set, &[&id, &balance]).await);
                }
            }
        }
        step!(
            tx.execute(
                &self.ledger,
                &[
                    &t.id(),
                    &(t.w as i32),
                    &(t.seq as i64),
                    &from,
                    &to,
                    &t.amount
                ],
            )
            .await
        );
        match tx.commit().await {
            Ok(()) => Attempt::Committed,
            Err(e) => commit_failed(e),
        }
    }

    pub async fn audit(&mut self) -> Res<Totals> {
        let r = self.c.query_one(&self.audit, &[]).await?;
        Ok(Totals {
            sum: r.get(0),
            count: r.get::<_, i64>(1) as u64,
            min: r.get(2),
        })
    }
}
