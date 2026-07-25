//! Index definitions must survive the trip to another node.
//!
//! Index creation used to be a purely local side effect: no replication log
//! entry was written and nothing was forwarded to physical shards. The
//! consequences were silent —
//!
//! - a peer ran unindexed full scans for queries the origin node served from
//!   an index;
//! - a peer never enforced a unique index it did not have;
//! - worst, `ttl::cleanup` skips any collection whose `list_ttl_indexes()` is
//!   empty, so documents expired on the node where the TTL index was created
//!   and lived forever on every other node.
//!
//! These tests drive the same `IndexSpec` path the replication worker uses, so
//! they fail if the spec stops round-tripping or an index family is dropped
//! from `create_index_from_spec`.

use solidb::storage::{
    IndexKind, IndexSpec, IndexType, StorageEngine, VectorIndexConfig, VectorMetric,
};
use tempfile::TempDir;

/// Two independent engines standing in for two nodes.
fn two_nodes() -> (StorageEngine, StorageEngine, TempDir, TempDir) {
    let a_dir = TempDir::new().unwrap();
    let b_dir = TempDir::new().unwrap();
    let a = StorageEngine::new(a_dir.path().to_str().unwrap()).unwrap();
    let b = StorageEngine::new(b_dir.path().to_str().unwrap()).unwrap();
    for e in [&a, &b] {
        e.create_database("app".to_string()).unwrap();
        e.get_database("app")
            .unwrap()
            .create_collection("items".to_string(), None)
            .unwrap();
    }
    (a, b, a_dir, b_dir)
}

fn items(engine: &StorageEngine) -> solidb::storage::Collection {
    engine
        .get_database("app")
        .unwrap()
        .get_collection("items")
        .unwrap()
}

/// Serialise on the origin, deserialise on the peer — exactly what the
/// replication log does with the `CreateIndex` payload.
fn replicate(spec: &IndexSpec, target: &solidb::storage::Collection) {
    let payload = serde_json::to_vec(spec).expect("spec serialises");
    let decoded: IndexSpec = serde_json::from_slice(&payload).expect("spec deserialises");
    target
        .create_index_from_spec(&decoded)
        .expect("peer applies spec");
}

#[test]
fn regular_index_replicates() {
    let (a, b, _da, _db) = two_nodes();
    let spec = IndexSpec::Regular {
        name: "by_email".to_string(),
        fields: vec!["email".to_string()],
        index_type: IndexType::Persistent,
        unique: true,
    };

    items(&a).create_index_from_spec(&spec).unwrap();
    replicate(&spec, &items(&b));

    let on_b = items(&b).list_indexes();
    let found = on_b
        .iter()
        .find(|i| i.name == "by_email")
        .expect("index present on peer");
    assert!(found.unique, "uniqueness must survive replication");
}

#[test]
fn fulltext_index_replicates() {
    let (a, b, _da, _db) = two_nodes();
    let spec = IndexSpec::Fulltext {
        name: "ft_body".to_string(),
        fields: vec!["body".to_string()],
        min_length: Some(3),
    };

    items(&a).create_index_from_spec(&spec).unwrap();
    replicate(&spec, &items(&b));

    assert!(items(&b).list_indexes().iter().any(|i| i.name == "ft_body"));
}

#[test]
fn geo_index_replicates() {
    let (a, b, _da, _db) = two_nodes();
    let spec = IndexSpec::Geo {
        name: "geo_loc".to_string(),
        field: "loc".to_string(),
    };

    items(&a).create_index_from_spec(&spec).unwrap();
    replicate(&spec, &items(&b));

    assert!(items(&b)
        .list_geo_indexes()
        .iter()
        .any(|i| i.name == "geo_loc"));
}

/// The one with data-visible consequences: without the TTL index the peer's
/// expiry sweep skips the collection entirely.
#[test]
fn ttl_index_replicates_so_both_nodes_expire() {
    let (a, b, _da, _db) = two_nodes();
    let spec = IndexSpec::Ttl {
        name: "ttl_created".to_string(),
        field: "created_at".to_string(),
        expire_after_seconds: 3600,
    };

    items(&a).create_index_from_spec(&spec).unwrap();
    // The bug this guards: creating on A left B with nothing, so B's expiry
    // sweep skipped the collection and B kept the documents forever.
    assert!(
        items(&b).list_ttl_indexes().is_empty(),
        "peer must start without the index, or this proves nothing"
    );

    replicate(&spec, &items(&b));

    let on_b = items(&b).list_ttl_indexes();
    let found = on_b
        .iter()
        .find(|i| i.name == "ttl_created")
        .expect("TTL index present on peer");
    assert_eq!(
        found.expire_after_seconds, 3600,
        "expiry window must match, or the two nodes retain data differently"
    );
}

#[test]
fn vector_index_replicates() {
    let (a, b, _da, _db) = two_nodes();
    let config = VectorIndexConfig::new("vec_emb".to_string(), "embedding".to_string(), 8)
        .with_metric(VectorMetric::Euclidean);
    let spec = IndexSpec::Vector(config);

    items(&a).create_index_from_spec(&spec).unwrap();
    replicate(&spec, &items(&b));

    let on_b = items(&b).list_vector_indexes();
    let found = on_b
        .iter()
        .find(|i| i.name == "vec_emb")
        .expect("vector index present on peer");
    assert_eq!(found.dimension, 8);
    assert_eq!(found.metric, VectorMetric::Euclidean);
}

/// Replication entries are replayed at least once, and the shard fan-out can
/// overlap with an entry the node already applied. Re-applying must not error.
#[test]
fn applying_the_same_spec_twice_is_idempotent() {
    let (a, _b, _da, _db) = two_nodes();
    let spec = IndexSpec::Regular {
        name: "dup".to_string(),
        fields: vec!["x".to_string()],
        index_type: IndexType::Hash,
        unique: false,
    };

    items(&a).apply_index_spec(&spec).unwrap();
    items(&a)
        .apply_index_spec(&spec)
        .expect("re-applying a replicated spec must succeed");

    assert_eq!(
        items(&a)
            .list_indexes()
            .iter()
            .filter(|i| i.name == "dup")
            .count(),
        1,
        "no duplicate index entry"
    );
}

/// Drops replay too, and may arrive after the index is already gone.
#[test]
fn dropping_a_missing_index_is_idempotent() {
    let (a, _b, _da, _db) = two_nodes();
    let spec = IndexSpec::Regular {
        name: "gone".to_string(),
        fields: vec!["y".to_string()],
        index_type: IndexType::Hash,
        unique: false,
    };
    items(&a).create_index_from_spec(&spec).unwrap();

    items(&a)
        .apply_index_drop(IndexKind::Regular, "gone")
        .unwrap();
    items(&a)
        .apply_index_drop(IndexKind::Regular, "gone")
        .expect("re-applying a replicated drop must succeed");

    assert!(!items(&a).list_indexes().iter().any(|i| i.name == "gone"));
}

/// Tolerance is for replication only. A client creating an index that already
/// exists must still get an error — making the shared helper idempotent for
/// replication must not quietly turn `POST .../index` into a no-op that
/// reports success.
#[test]
fn client_facing_create_still_rejects_a_duplicate() {
    let (a, _b, _da, _db) = two_nodes();
    let spec = IndexSpec::Regular {
        name: "strict".to_string(),
        fields: vec!["z".to_string()],
        index_type: IndexType::Hash,
        unique: false,
    };

    items(&a).create_index_from_spec(&spec).unwrap();
    let err = items(&a)
        .create_index_from_spec(&spec)
        .expect_err("duplicate must be reported to the caller");
    assert!(
        err.to_string().contains("already exists"),
        "unexpected error: {err}"
    );
}

/// The same split for drops: dropping something absent is an error for a
/// client, tolerated only on the replication path.
#[test]
fn client_facing_drop_still_rejects_a_missing_index() {
    let (a, _b, _da, _db) = two_nodes();
    let err = items(&a)
        .drop_index_of_kind(IndexKind::Regular, "never_existed")
        .expect_err("missing index must be reported to the caller");
    assert!(
        err.to_string().contains("not found"),
        "unexpected error: {err}"
    );
}

/// Every family must be reachable through the one spec type; a family missing
/// from `create_index_from_spec` would replicate as a silent no-op.
#[test]
fn every_index_family_maps_to_a_kind() {
    let cases: Vec<(IndexSpec, IndexKind)> = vec![
        (
            IndexSpec::Regular {
                name: "r".into(),
                fields: vec!["f".into()],
                index_type: IndexType::Hash,
                unique: false,
            },
            IndexKind::Regular,
        ),
        (
            IndexSpec::Fulltext {
                name: "f".into(),
                fields: vec!["f".into()],
                min_length: None,
            },
            IndexKind::Regular,
        ),
        (
            IndexSpec::Geo {
                name: "g".into(),
                field: "f".into(),
            },
            IndexKind::Geo,
        ),
        (
            IndexSpec::Ttl {
                name: "t".into(),
                field: "f".into(),
                expire_after_seconds: 1,
            },
            IndexKind::Ttl,
        ),
        (
            IndexSpec::Vector(VectorIndexConfig::new("v".into(), "f".into(), 4)),
            IndexKind::Vector,
        ),
    ];

    for (spec, expected_kind) in cases {
        assert_eq!(spec.kind(), expected_kind, "kind for {:?}", spec);
        // Round-trips through the wire format used by the replication log.
        let bytes = serde_json::to_vec(&spec).unwrap();
        let back: IndexSpec = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.name(), spec.name());
        assert_eq!(back.kind(), spec.kind());
    }
}

/// `SyncMessage` travels between nodes as bincode, which identifies enum
/// variants by position. Renumbering `Operation` makes a new node's message
/// decode as a *different* operation on an old node — a silent
/// misinterpretation during a rolling upgrade rather than a loud failure.
///
/// This pins the discriminants that existed before `CreateIndex`/`DropIndex`
/// were added. New variants must be appended, never inserted.
#[test]
fn operation_wire_discriminants_are_stable() {
    use solidb::sync::protocol::Operation;

    let expected = [
        (Operation::Insert, 0u32),
        (Operation::Update, 1),
        (Operation::Delete, 2),
        (Operation::CreateCollection, 3),
        (Operation::DeleteCollection, 4),
        (Operation::TruncateCollection, 5),
        (Operation::CreateDatabase, 6),
        (Operation::DeleteDatabase, 7),
        (Operation::PutBlobChunk, 8),
        (Operation::DeleteBlob, 9),
        (Operation::ColumnarInsert, 10),
        (Operation::ColumnarDelete, 11),
        (Operation::ColumnarCreateCollection, 12),
        (Operation::ColumnarDropCollection, 13),
        (Operation::ColumnarTruncate, 14),
        // Appended after the fact — safe, because nothing shifted.
        (Operation::CreateIndex, 15),
        (Operation::DropIndex, 16),
    ];

    for (op, index) in expected {
        let encoded = bincode::serialize(&op).expect("operation serialises");
        // bincode writes an enum discriminant as a little-endian u32.
        let got = u32::from_le_bytes(encoded[0..4].try_into().unwrap());
        assert_eq!(
            got, index,
            "{op:?} moved to wire index {got}, expected {index} — inserting a \
             variant renumbers every later one and breaks rolling upgrades. \
             Append instead."
        );
    }
}
