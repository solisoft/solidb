//! Row policies (`ROW_POLICY`) must hold on every collection read path, and
//! only an admin may change one.
//!
//! Audit C4 (setter needed only Read), A11 (`APPLY`/`CALL` hid state-changing
//! builtins from the mutation check) and H2 (the policy was applied on one
//! scan path only: the fast path, index reads, JOIN and point reads all
//! returned every row).

use serde_json::{json, Value};
use solidb::sdbql::QueryPrincipal;
use solidb::storage::StorageEngine;
use solidb::{parse, IndexType, QueryExecutor};
use tempfile::TempDir;

fn principal(user: &str, admin: bool, write: bool) -> QueryPrincipal {
    QueryPrincipal {
        user: user.into(),
        roles: if admin {
            vec!["admin".into()]
        } else if write {
            vec!["editor".into()]
        } else {
            vec!["viewer".into()]
        },
        can_read: true,
        can_write: write || admin,
        can_admin: admin,
    }
}

/// `orders` holds one "acme" row (`1`) and one "other" row (`2`), with a
/// persistent index on `n` and a policy admitting only "acme".
fn setup() -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let e = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    e.create_collection("orders".to_string(), None).unwrap();
    let orders = e.get_collection("orders").unwrap();
    orders
        .create_index(
            "idx_n".into(),
            vec!["n".into()],
            IndexType::Persistent,
            false,
        )
        .unwrap();
    orders
        .insert(json!({"_key": "1", "tenant": "acme", "n": 1, "user": "u1"}))
        .unwrap();
    orders
        .insert(json!({"_key": "2", "tenant": "other", "n": 2, "user": "u1"}))
        .unwrap();

    e.create_collection("users".to_string(), None).unwrap();
    e.get_collection("users")
        .unwrap()
        .insert(json!({"_key": "u1"}))
        .unwrap();

    run(
        &e,
        principal("root", true, true),
        r#"RETURN ROW_POLICY("orders", "doc.tenant == \"acme\"")"#,
    )
    .unwrap();
    (e, tmp)
}

fn run(e: &StorageEngine, p: QueryPrincipal, q: &str) -> Result<Vec<Value>, String> {
    let query = parse(q).map_err(|err| format!("parse {q}: {err}"))?;
    QueryExecutor::new(e)
        .with_principal(p)
        .execute(&query)
        .map_err(|err| err.to_string())
}

fn viewer(e: &StorageEngine, q: &str) -> Vec<Value> {
    run(e, principal("ada", false, false), q).unwrap_or_else(|err| panic!("{q}: {err}"))
}

#[test]
fn fast_path_scan_is_filtered() {
    let (e, _t) = setup();
    assert_eq!(
        viewer(&e, "FOR d IN orders RETURN d")
            .iter()
            .map(|d| d["_key"].clone())
            .collect::<Vec<_>>(),
        vec![json!("1")]
    );
    assert_eq!(viewer(&e, "FOR d IN orders LIMIT 5 RETURN d").len(), 1);
    // A LIMIT applies to what the policy lets through, not before it.
    assert_eq!(
        viewer(&e, "FOR d IN orders LIMIT 1 RETURN d.tenant"),
        vec![json!("acme")]
    );
    // An admin still sees everything.
    let all = run(
        &e,
        principal("root", true, true),
        "FOR d IN orders RETURN d",
    )
    .unwrap();
    assert_eq!(all.len(), 2);
}

#[test]
fn index_paths_are_filtered() {
    let (e, _t) = setup();
    // Index-backed FILTER.
    assert!(viewer(&e, "FOR d IN orders FILTER d.n == 2 RETURN d").is_empty());
    assert_eq!(
        viewer(&e, "FOR d IN orders FILTER d.n == 1 RETURN d._key"),
        vec![json!("1")]
    );
    // Index-sorted SORT + LIMIT and SORT alone.
    assert_eq!(
        viewer(&e, "FOR d IN orders SORT d.n DESC LIMIT 10 RETURN d._key"),
        vec![json!("1")]
    );
    assert_eq!(
        viewer(&e, "FOR d IN orders SORT d.n DESC RETURN d._key"),
        vec![json!("1")]
    );
    assert_eq!(
        viewer(
            &e,
            "FOR d IN orders SORT d._key DESC LIMIT 10 RETURN d._key"
        ),
        vec![json!("1")]
    );
}

#[test]
fn join_side_is_filtered() {
    let (e, _t) = setup();
    let rows = viewer(
        &e,
        "FOR u IN users JOIN orders ON u._key == orders.user RETURN orders[*]._key",
    );
    assert_eq!(rows, vec![json!(["1"])]);
}

#[test]
fn point_reads_are_filtered() {
    let (e, _t) = setup();
    assert_eq!(
        viewer(&e, r#"RETURN DOCUMENT("orders/2")"#),
        vec![Value::Null]
    );
    assert_eq!(
        viewer(&e, r#"RETURN DOCUMENT("orders", "2")"#),
        vec![Value::Null]
    );
    assert_eq!(
        viewer(&e, r#"RETURN DOCUMENT("orders/1")._key"#),
        vec![json!("1")]
    );
    assert_eq!(
        viewer(&e, r#"RETURN DOCUMENT("orders", ["1", "2"])[*]._key"#),
        vec![json!(["1"])]
    );
    assert_eq!(
        viewer(&e, r#"RETURN DOCUMENT(["orders/1", "orders/2"])[*]._key"#),
        vec![json!(["1"])]
    );
}

#[test]
fn sample_is_filtered() {
    let (e, _t) = setup();
    let sampled = viewer(&e, r#"RETURN SAMPLE("orders", 10)[*]._key"#);
    assert_eq!(sampled, vec![json!(["1"])]);
}

#[test]
fn search_view_alias_uses_backing_policy() {
    let (e, _t) = setup();
    run(
        &e,
        principal("root", true, true),
        r#"RETURN CREATE_VIEW("orders_v", {collection: "orders", fields: ["tenant"]})"#,
    )
    .unwrap();
    assert_eq!(
        viewer(&e, "FOR d IN orders_v FILTER d.n > 0 RETURN d._key"),
        vec![json!("1")]
    );
}

#[test]
fn setter_requires_admin() {
    let (e, _t) = setup();
    for p in [principal("ada", false, false), principal("ed", false, true)] {
        let err = run(&e, p.clone(), r#"RETURN ROW_POLICY("orders", null)"#)
            .expect_err("non-admin lifted the policy");
        assert!(err.contains("admin"), "{err}");
        assert!(run(&e, p, r#"RETURN ROW_POLICY("orders", "true")"#).is_err());
    }
    // No principal at all is not an admin either.
    let q = parse(r#"RETURN ROW_POLICY("orders", null)"#).unwrap();
    assert!(QueryExecutor::new(&e).execute(&q).is_err());

    // The getter stays a read.
    assert_eq!(
        viewer(&e, r#"RETURN ROW_POLICY("orders")"#),
        vec![json!("doc.tenant == \"acme\"")]
    );
    // Still in force.
    assert_eq!(viewer(&e, "FOR d IN orders RETURN d").len(), 1);
}

#[test]
fn setter_rejects_bad_predicates() {
    let (e, _t) = setup();
    let admin = principal("root", true, true);
    assert!(run(
        &e,
        admin.clone(),
        r#"RETURN ROW_POLICY("orders", "doc.a ==")"#
    )
    .is_err());
    assert!(run(
        &e,
        admin,
        r#"RETURN ROW_POLICY("orders", "LENGTH([DROP_GRAPH(\"g\")]) >= 0")"#
    )
    .is_err());
}

#[test]
fn apply_and_call_refuse_state_changing_builtins() {
    let (e, _t) = setup();
    let admin = principal("root", true, true);
    for q in [
        r#"RETURN APPLY("ROW_POLICY", ["orders", null])"#,
        r#"RETURN CALL("row_policy", "orders", null)"#,
        r#"RETURN APPLY("DROP_GRAPH", ["g"])"#,
        r#"RETURN CALL("APPLY", "DROP_VIEW", ["v"])"#,
    ] {
        assert!(run(&e, admin.clone(), q).is_err(), "{q} must be refused");
    }
    // The policy survived every attempt.
    assert_eq!(viewer(&e, "FOR d IN orders RETURN d").len(), 1);
    // Pure functions still dispatch, including nested.
    assert_eq!(
        run(&e, admin.clone(), r#"RETURN CALL("ABS", -3)"#).unwrap(),
        vec![json!(3.0)]
    );
    assert_eq!(
        run(
            &e,
            admin.clone(),
            r#"RETURN APPLY("ROW_POLICY", ["orders"])"#
        )
        .unwrap(),
        vec![json!("doc.tenant == \"acme\"")]
    );
    assert_eq!(
        run(&e, admin, r#"RETURN APPLY("CALL", ["UPPER", "hi"])"#).unwrap(),
        vec![json!("HI")]
    );
}
