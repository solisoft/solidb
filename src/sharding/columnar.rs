//! Sharded columnar collections (card #899).
//!
//! A sharded columnar collection `m` is an empty columnar collection that
//! says how it is split ([`ColumnarShards`], in its meta, replicated with
//! its creation), and a columnar collection per shard, `m_s{shard}`, on the
//! nodes of that shard's assignment: its primary and its replicas. A row
//! goes to the shard its `shard_key` column hashes to, as a document goes
//! to the shard its key hashes to.
//!
//! The coordinator of a write is the node it reaches: it checks the rows
//! against the schema, gives them their ids, and writes each shard on each
//! of its nodes, here or over HTTP (`X-Shard-Direct`, the cluster secret):
//! a shard's copies hold the same rows under the same ids. Nothing of it is
//! logged: the shards are created where they live.
//!
//! Reads gather the shards' rows ([`gather`]) and read them as one
//! collection would; aggregations are computed on each shard first
//! (`sdbql::executor::execution::columnar_scan`, fold states merged here).
//!
//! What this version does not route (deletes, truncate, columns, indexes,
//! retention, compaction, tags, branches) is refused on a sharded
//! collection (`sync::columnar::refuse_if_sharded`).

use crate::error::{DbError, DbResult};
use crate::sharding::coordinator::{CollectionShardConfig, ShardCoordinator};
use crate::storage::columnar::{ColumnType, ColumnarShards};
use crate::storage::database::Database;
use crate::storage::{ColumnDef, ColumnarCollection, CompressionType};
use crate::sync::log::SyncLog;
use serde_json::{Map, Value};

/// A row and its id.
pub type IdRow = (String, Map<String, Value>);

/// The name of the collection that holds shard `shard` of `name`.
pub fn physical(name: &str, shard: u16) -> String {
    format!("{name}_s{shard}")
}

/// The shard a row goes to.
pub fn shard_of(shards: &ColumnarShards, row: &Map<String, Value>) -> u16 {
    let key = match row.get(&shards.shard_key) {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => "null".to_string(),
    };
    crate::sharding::router::ShardRouter::route(&key, shards.num_shards)
}

/// Where requests to other nodes go, and as whom.
struct Peers<'a> {
    coord: Option<&'a ShardCoordinator>,
}

impl Peers<'_> {
    fn is_local(&self, node: &str) -> bool {
        match self.coord {
            None => true,
            Some(coord) => node == "local" || node == coord.my_node_id(),
        }
    }

    fn url(&self, node: &str, path: &str) -> DbResult<String> {
        let coord = self
            .coord
            .ok_or_else(|| DbError::InternalError("no shard coordinator".to_string()))?;
        let addr = coord.get_node_api_address(node).ok_or_else(|| {
            DbError::InternalError(format!("node '{node}' of a shard has no address"))
        })?;
        Ok(crate::cluster::http::peer_url(&addr, path))
    }

    fn secret(&self) -> String {
        self.coord.map(|c| c.cluster_secret()).unwrap_or_default()
    }

    fn send(
        &self,
        request: reqwest::blocking::RequestBuilder,
    ) -> DbResult<reqwest::blocking::Response> {
        let response = request
            .header("X-Shard-Direct", "true")
            .header("X-Cluster-Secret", self.secret())
            .send()
            .map_err(|e| DbError::InternalError(format!("shard request: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().unwrap_or_default();
            return Err(DbError::InternalError(format!(
                "shard request answered {status}: {text}"
            )));
        }
        Ok(response)
    }
}

fn type_name(t: &ColumnType) -> &'static str {
    match t {
        ColumnType::Int64 => "int64",
        ColumnType::Float64 => "float64",
        ColumnType::String => "string",
        ColumnType::Bool => "bool",
        ColumnType::Timestamp => "timestamp",
        ColumnType::Json => "json",
    }
}

/// Create the sharded columnar collection `name`: the collection that
/// stands for it, logged for the peers with how it is split, and each shard
/// on each of its nodes.
#[allow(clippy::too_many_arguments)]
pub fn create(
    coord: Option<&ShardCoordinator>,
    db: &Database,
    log: Option<&SyncLog>,
    name: &str,
    columns: Vec<ColumnDef>,
    compression: CompressionType,
    num_shards: u16,
    shard_key: &str,
    replication_factor: u16,
) -> DbResult<ColumnarCollection> {
    if !columns.iter().any(|c| c.name == shard_key) {
        return Err(DbError::BadRequest(format!(
            "The shard key '{shard_key}' is not a column of '{name}'"
        )));
    }
    let config = CollectionShardConfig {
        num_shards,
        shard_key: shard_key.to_string(),
        replication_factor: replication_factor.max(1),
    };
    let assignments = match coord {
        Some(coord) => {
            coord
                .compute_shard_table(&db.name, name, &config)
                .map_err(DbError::InternalError)?
                .assignments
        }
        None => (0..num_shards)
            .map(|shard| {
                (
                    shard,
                    crate::sharding::coordinator::ShardAssignment {
                        shard_id: shard,
                        primary_node: "local".to_string(),
                        replica_nodes: Vec::new(),
                    },
                )
            })
            .collect(),
    };
    let shards = ColumnarShards {
        num_shards,
        shard_key: shard_key.to_string(),
        replication_factor: config.replication_factor,
        assignments,
    };
    let peers = Peers { coord };
    let client = crate::storage::http_client::get_blocking_http_client();
    for (shard, assignment) in &shards.assignments {
        let phys = physical(name, *shard);
        for node in std::iter::once(&assignment.primary_node).chain(&assignment.replica_nodes) {
            if peers.is_local(node) {
                match crate::sync::columnar::create(
                    db,
                    None,
                    &phys,
                    columns.clone(),
                    compression.clone(),
                ) {
                    Ok(_) | Err(DbError::CollectionAlreadyExists(_)) => {}
                    Err(e) => return Err(e),
                }
            } else {
                let body = serde_json::json!({
                    "name": phys,
                    "columns": columns.iter().map(|c| serde_json::json!({
                        "name": c.name, "type": type_name(&c.data_type), "nullable": c.nullable,
                    })).collect::<Vec<_>>(),
                    "compression": match compression {
                        CompressionType::None => "none",
                        _ => "lz4",
                    },
                });
                let url = peers.url(node, &format!("/_api/database/{}/columnar", db.name))?;
                peers.send(client.post(url).json(&body))?;
            }
        }
    }
    crate::sync::columnar::create_sharded(db, log, name, columns, compression, shards)
}

/// Insert `rows` into the sharded collection `name`: checked against its
/// schema, given their ids here, and written to each node of their shard.
/// Returns the ids, in the order of `rows`. A node that refuses fails the
/// insert; the shards already written keep their rows.
pub fn insert(
    coord: Option<&ShardCoordinator>,
    db: &Database,
    name: &str,
    rows: Vec<Value>,
) -> DbResult<Vec<String>> {
    let logical = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    let shards = logical
        .sharding()
        .ok_or_else(|| DbError::InternalError(format!("'{name}' is not sharded")))?;
    let rows = logical.conform_rows(rows)?;
    let ids: Vec<String> = rows
        .iter()
        .map(|_| uuid::Uuid::now_v7().to_string())
        .collect();
    let mut by_shard: std::collections::BTreeMap<u16, Vec<IdRow>> = Default::default();
    for (id, row) in ids.iter().zip(rows) {
        by_shard
            .entry(shard_of(&shards, &row))
            .or_default()
            .push((id.clone(), row));
    }
    let peers = Peers { coord };
    let client = crate::storage::http_client::get_blocking_http_client();
    for (shard, mut part) in by_shard {
        part.sort_by(|a, b| a.0.cmp(&b.0));
        let assignment = shards.assignments.get(&shard).ok_or_else(|| {
            DbError::InternalError(format!("shard {shard} of '{name}' has no assignment"))
        })?;
        let phys = physical(name, shard);
        for node in std::iter::once(&assignment.primary_node).chain(&assignment.replica_nodes) {
            if peers.is_local(node) {
                ColumnarCollection::load(phys.clone(), &db.name, db.db_arc())?
                    .insert_rows_with_ids(part.clone())?;
            } else {
                let body = rmp_serde::to_vec(&part)
                    .map_err(|e| DbError::InternalError(format!("shard rows: {e}")))?;
                let url = peers.url(
                    node,
                    &format!("/_api/database/{}/columnar/{phys}/rows", db.name),
                )?;
                peers.send(
                    client
                        .post(url)
                        .header("Content-Type", "application/msgpack")
                        .body(body),
                )?;
            }
        }
    }
    Ok(ids)
}

/// The rows of every shard of `name`, ascending by id: each from a node of
/// its shard, this one when it holds it.
pub fn gather(
    coord: Option<&ShardCoordinator>,
    db: &Database,
    name: &str,
    shards: &ColumnarShards,
) -> DbResult<Vec<IdRow>> {
    let peers = Peers { coord };
    let client = crate::storage::http_client::get_blocking_http_client();
    let mut rows = Vec::new();
    for shard in 0..shards.num_shards {
        let assignment = shards.assignments.get(&shard).ok_or_else(|| {
            DbError::InternalError(format!("shard {shard} of '{name}' has no assignment"))
        })?;
        let phys = physical(name, shard);
        let nodes: Vec<&String> = std::iter::once(&assignment.primary_node)
            .chain(&assignment.replica_nodes)
            .collect();
        if nodes.iter().any(|n| peers.is_local(n)) {
            rows.extend(ColumnarCollection::load(phys, &db.name, db.db_arc())?.rows_with_ids()?);
            continue;
        }
        let mut last = None;
        let mut got = None;
        for node in nodes {
            let url = peers.url(
                node,
                &format!("/_api/database/{}/columnar/{phys}/rows", db.name),
            )?;
            match peers.send(client.get(url)).and_then(|r| {
                r.bytes()
                    .map_err(|e| DbError::InternalError(format!("shard rows: {e}")))
            }) {
                Ok(bytes) => {
                    got = Some(
                        rmp_serde::from_slice::<Vec<IdRow>>(&bytes)
                            .map_err(|e| DbError::InternalError(format!("shard rows: {e}")))?,
                    );
                    break;
                }
                Err(e) => last = Some(e),
            }
        }
        match got {
            Some(part) => rows.extend(part),
            None => {
                return Err(last.unwrap_or_else(|| {
                    DbError::InternalError(format!("shard {shard} of '{name}' answered nothing"))
                }))
            }
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(rows)
}

/// Shard folds computed or asked for since the process started: for
/// tests, which must know a COLLECT was not gathered.
pub static SHARD_FOLDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The fold state of shard `shard` of `name` for a COLLECT (card #899):
/// computed by `local` when this node holds the shard, else asked of a node
/// that does (`POST …/columnar/{shard}/fold` with `request`: the clauses
/// over that shard and the bind variables).
pub fn fold_shard(
    coord: Option<&ShardCoordinator>,
    db: &Database,
    name: &str,
    shards: &ColumnarShards,
    shard: u16,
    request: &Value,
    local: impl FnOnce(&ColumnarCollection) -> DbResult<Vec<u8>>,
) -> DbResult<Vec<u8>> {
    SHARD_FOLDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let peers = Peers { coord };
    let assignment = shards.assignments.get(&shard).ok_or_else(|| {
        DbError::InternalError(format!("shard {shard} of '{name}' has no assignment"))
    })?;
    let phys = physical(name, shard);
    let nodes: Vec<&String> = std::iter::once(&assignment.primary_node)
        .chain(&assignment.replica_nodes)
        .collect();
    if nodes.iter().any(|n| peers.is_local(n)) {
        let here = ColumnarCollection::load(phys, &db.name, db.db_arc())?;
        return local(&here);
    }
    let client = crate::storage::http_client::get_blocking_http_client();
    let mut last = None;
    for node in nodes {
        let url = peers.url(
            node,
            &format!("/_api/database/{}/columnar/{phys}/fold", db.name),
        )?;
        match peers.send(client.post(url).json(request)).and_then(|r| {
            r.bytes()
                .map_err(|e| DbError::InternalError(format!("shard fold: {e}")))
        }) {
            Ok(bytes) => return Ok(bytes.to_vec()),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| {
        DbError::InternalError(format!("shard {shard} of '{name}' answered nothing"))
    }))
}

/// Drop the sharded collection `name`: each shard on each of its nodes,
/// then the collection that stands for them (logged for the peers).
pub fn drop(
    coord: Option<&ShardCoordinator>,
    db: &Database,
    log: Option<&SyncLog>,
    name: &str,
    shards: &ColumnarShards,
) -> DbResult<()> {
    let peers = Peers { coord };
    let client = crate::storage::http_client::get_blocking_http_client();
    for (shard, assignment) in &shards.assignments {
        let phys = physical(name, *shard);
        for node in std::iter::once(&assignment.primary_node).chain(&assignment.replica_nodes) {
            if peers.is_local(node) {
                match crate::sync::columnar::drop(db, None, &phys) {
                    Ok(()) | Err(DbError::CollectionNotFound(_)) => {}
                    Err(e) => return Err(e),
                }
            } else {
                let url =
                    peers.url(node, &format!("/_api/database/{}/columnar/{phys}", db.name))?;
                peers.send(client.delete(url))?;
            }
        }
    }
    crate::sync::columnar::drop(db, log, name)
}
