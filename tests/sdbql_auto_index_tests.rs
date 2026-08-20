//! Query-driven auto-index: opt-in FILTER creates `_auto_*` indexes.

mod common;

use common::{create_test_engine, execute_query};
use serde_json::json;
use solidb::sdbql::QueryPrincipal;
use solidb::storage::IndexType;
use solidb::{parse, QueryExecutor};
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn index_names(engine: &solidb::storage::StorageEngine, coll: &str) -> Vec<String> {
    engine
        .get_collection(coll)
        .unwrap()
        .list_indexes()
        .into_iter()
        .map(|i| i.name)
        .collect()
}

/// Creating an index is a write, so every test that expects one has to run as a
/// principal that may write. `common::execute_query` runs without a principal
/// on purpose — see `auto_index_without_principal_does_not_create`.
fn writer() -> QueryPrincipal {
    QueryPrincipal {
        user: "editor".into(),
        roles: vec!["editor".into()],
        can_read: true,
        can_write: true,
        can_admin: false,
    }
}

fn execute_as_writer(
    engine: &solidb::storage::StorageEngine,
    query: &str,
) -> Vec<serde_json::Value> {
    let parsed = parse(query).unwrap();
    QueryExecutor::new(engine)
        .with_principal(writer())
        .execute(&parsed)
        .unwrap()
}

fn with_env_cleared<R>(f: impl FnOnce() -> R) -> R {
    let _guard = env_lock().lock().unwrap();
    let prev = std::env::var("SOLIDB_AUTO_INDEX").ok();
    let prev_max = std::env::var("SOLIDB_AUTO_INDEX_MAX_DOCS").ok();
    std::env::remove_var("SOLIDB_AUTO_INDEX");
    std::env::remove_var("SOLIDB_AUTO_INDEX_MAX_DOCS");
    let out = f();
    match prev {
        Some(v) => std::env::set_var("SOLIDB_AUTO_INDEX", v),
        None => std::env::remove_var("SOLIDB_AUTO_INDEX"),
    }
    match prev_max {
        Some(v) => std::env::set_var("SOLIDB_AUTO_INDEX_MAX_DOCS", v),
        None => std::env::remove_var("SOLIDB_AUTO_INDEX_MAX_DOCS"),
    }
    out
}

#[test]
fn auto_index_off_does_not_create() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.insert(json!({"_key": "1", "city": "Paris"})).unwrap();

        let _ = execute_as_writer(
            &engine,
            r#"FOR d IN items FILTER d.city == "Paris" RETURN d"#,
        );
        assert!(
            !index_names(&engine, "items")
                .iter()
                .any(|n| n == "_auto_city"),
            "flag off must not create _auto_city"
        );
    });
}

#[test]
fn auto_index_on_creates_for_filter() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        items.insert(json!({"_key": "1", "city": "Paris"})).unwrap();

        let rows = execute_as_writer(
            &engine,
            r#"FOR d IN items FILTER d.city == "Paris" RETURN d.city"#,
        );
        assert_eq!(rows, vec![json!("Paris")]);
        assert!(
            index_names(&engine, "items").contains(&"_auto_city".to_string()),
            "{:?}",
            index_names(&engine, "items")
        );
        assert!(items.index_lookup_eq("city", &json!("Paris")).is_some());
    });
}

/// The Write check must fail closed: an executor built without a principal —
/// internal refreshes, background jobs, any handler that forgets to pass one —
/// does not create indexes either.
#[test]
fn auto_index_without_principal_does_not_create() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        items.insert(json!({"_key": "1", "city": "Paris"})).unwrap();

        let rows = execute_query(
            &engine,
            r#"FOR d IN items FILTER d.city == "Paris" RETURN d.city"#,
        );
        assert_eq!(rows, vec![json!("Paris")], "the query must still answer");
        assert!(
            !index_names(&engine, "items")
                .iter()
                .any(|n| n == "_auto_city"),
            "no principal must not create _auto_city"
        );
    });
}

#[test]
fn auto_index_read_principal_does_not_create() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        items.insert(json!({"_key": "1", "city": "Paris"})).unwrap();

        let query = parse(r#"FOR d IN items FILTER d.city == "Paris" RETURN d"#).unwrap();
        let principal = QueryPrincipal {
            user: "viewer".into(),
            roles: vec!["viewer".into()],
            can_read: true,
            can_write: false,
            can_admin: false,
        };
        let _ = QueryExecutor::new(&engine)
            .with_principal(principal)
            .execute(&query)
            .unwrap();
        assert!(!index_names(&engine, "items")
            .iter()
            .any(|n| n == "_auto_city"));
    });
}

#[test]
fn auto_index_respects_existing_user_index() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        items
            .create_index(
                "by_city".to_string(),
                vec!["city".to_string()],
                IndexType::Persistent,
                false,
            )
            .unwrap();
        items.insert(json!({"_key": "1", "city": "Paris"})).unwrap();

        let _ = execute_as_writer(
            &engine,
            r#"FOR d IN items FILTER d.city == "Paris" RETURN d"#,
        );
        assert!(!index_names(&engine, "items")
            .iter()
            .any(|n| n == "_auto_city"));
    });
}

/// A composite index already serves the FILTER, so there is nothing to add.
/// This is what the (single-document) existence probe is for: the per-field
/// checks cannot see a composite index.
#[test]
fn auto_index_skips_filter_covered_by_composite_index() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        items
            .create_index(
                "by_city_age".to_string(),
                vec!["city".to_string(), "age".to_string()],
                IndexType::Hash,
                false,
            )
            .unwrap();
        items
            .insert(json!({"_key": "1", "city": "Paris", "age": 30}))
            .unwrap();

        let rows = execute_as_writer(
            &engine,
            r#"FOR d IN items FILTER d.city == "Paris" AND d.age == 30 RETURN d._key"#,
        );
        assert_eq!(rows, vec![json!("1")]);
        let names = index_names(&engine, "items");
        assert!(
            !names.iter().any(|n| n.starts_with("_auto_")),
            "{:?}",
            names
        );
    });
}

#[test]
fn auto_index_env_enables_without_collection_flag() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.insert(json!({"_key": "1", "city": "Paris"})).unwrap();

        std::env::set_var("SOLIDB_AUTO_INDEX", "1");
        let _ = execute_as_writer(
            &engine,
            r#"FOR d IN items FILTER d.city == "Paris" RETURN d"#,
        );
        assert!(index_names(&engine, "items").contains(&"_auto_city".to_string()));
    });
}

#[test]
fn auto_index_disable_overrides_env() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.disable_auto_index().unwrap();
        items.insert(json!({"_key": "1", "city": "Paris"})).unwrap();
        std::env::set_var("SOLIDB_AUTO_INDEX", "1");
        let _ = execute_as_writer(
            &engine,
            r#"FOR d IN items FILTER d.city == "Paris" RETURN d"#,
        );
        assert!(!index_names(&engine, "items")
            .iter()
            .any(|n| n == "_auto_city"));
    });
}

#[test]
fn auto_index_caps_at_sixteen() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        let mut doc = json!({"_key": "1"});
        for i in 0..17 {
            doc.as_object_mut()
                .unwrap()
                .insert(format!("f{i}"), json!(i));
        }
        items.insert(doc).unwrap();

        for i in 0..17 {
            let q = format!(r#"FOR d IN items FILTER d.f{i} == {i} RETURN d"#);
            let _ = execute_as_writer(&engine, &q);
        }
        let auto = index_names(&engine, "items")
            .into_iter()
            .filter(|n| n.starts_with("_auto_"))
            .count();
        assert_eq!(auto, 16);
    });
}

/// A hand-made index that merely looks like an auto-index is not one: it must
/// not silently consume a slot of the cap.
#[test]
fn auto_index_cap_ignores_lookalike_user_index() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        // `_auto_elsewhere` over `city` is not what auto-indexing would create.
        items
            .create_index(
                "_auto_elsewhere".to_string(),
                vec!["city".to_string()],
                IndexType::Persistent,
                false,
            )
            .unwrap();
        items
            .insert(json!({"_key": "1", "city": "Paris", "zone": "z1"}))
            .unwrap();

        let _ = execute_as_writer(&engine, r#"FOR d IN items FILTER d.zone == "z1" RETURN d"#);
        assert!(
            index_names(&engine, "items").contains(&"_auto_zone".to_string()),
            "{:?}",
            index_names(&engine, "items")
        );
    });
}

#[test]
fn auto_index_explain_does_not_create() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        items.insert(json!({"_key": "1", "city": "Paris"})).unwrap();

        let query = parse(r#"FOR d IN items FILTER d.city == "Paris" RETURN d"#).unwrap();
        let explain = QueryExecutor::new(&engine)
            .with_principal(writer())
            .explain(&query)
            .unwrap();
        assert!(explain
            .collections
            .iter()
            .any(|c| c.auto_index_candidate.as_deref() == Some("city")));
        assert!(!index_names(&engine, "items")
            .iter()
            .any(|n| n == "_auto_city"));
    });
}

/// EXPLAIN answers for the caller who asked, so a read-only principal must not
/// be told an index is coming.
#[test]
fn auto_index_explain_reports_nothing_for_reader() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        items.insert(json!({"_key": "1", "city": "Paris"})).unwrap();

        let query = parse(r#"FOR d IN items FILTER d.city == "Paris" RETURN d"#).unwrap();
        let explain = QueryExecutor::new(&engine).explain(&query).unwrap();
        assert!(explain
            .collections
            .iter()
            .all(|c| c.auto_index_candidate.is_none()));
    });
}

#[test]
fn auto_index_skips_key() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        items.insert(json!({"_key": "k1"})).unwrap();

        let _ = execute_as_writer(&engine, r#"FOR d IN items FILTER d._key == "k1" RETURN d"#);
        assert!(!index_names(&engine, "items")
            .iter()
            .any(|n| n.contains("_auto_")));
    });
}

#[test]
fn auto_index_sort_does_not_create_or_drop_missing() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        items.insert(json!({"_key": "a", "age": 2})).unwrap();
        items.insert(json!({"_key": "b"})).unwrap();
        items.insert(json!({"_key": "c", "age": 1})).unwrap();

        let rows = execute_as_writer(&engine, "FOR d IN items SORT d.age RETURN d._key");
        assert_eq!(rows.len(), 3, "{:?}", rows);
        assert!(!index_names(&engine, "items")
            .iter()
            .any(|n| n == "_auto_age"));
    });
}

#[test]
fn auto_index_skips_null_filter() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        items
            .insert(json!({"_key": "1", "deleted_at": serde_json::Value::Null}))
            .unwrap();

        let _ = execute_as_writer(
            &engine,
            "FOR d IN items FILTER d.deleted_at == null RETURN d",
        );
        assert!(!index_names(&engine, "items")
            .iter()
            .any(|n| n == "_auto_deleted_at"));
    });
}

/// A field no document carries — a typo, or a key that only *looks* like a
/// path — must not leave an empty index behind holding one of the 16 slots.
#[test]
fn auto_index_drops_index_no_document_can_fill() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        // `a.b` is read as a path, so the flat key below never matches it.
        items.insert(json!({"_key": "1", "a.b": 5})).unwrap();

        let rows = execute_as_writer(
            &engine,
            r#"FOR d IN items FILTER d["a.b"] == 5 RETURN d._key"#,
        );
        assert!(rows.is_empty(), "{:?}", rows);
        let names = index_names(&engine, "items");
        assert!(!names.iter().any(|n| n == "_auto_a.b"), "{:?}", names);

        // Same for a plain misspelling.
        let _ = execute_as_writer(
            &engine,
            r#"FOR d IN items FILTER d.citty == "Paris" RETURN d"#,
        );
        let names = index_names(&engine, "items");
        assert!(!names.iter().any(|n| n == "_auto_citty"), "{:?}", names);
    });
}

#[test]
fn auto_index_nested_field_keeps_dots() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        items
            .insert(json!({"_key": "1", "address": {"city": "Paris"}}))
            .unwrap();

        let _ = execute_as_writer(
            &engine,
            r#"FOR d IN items FILTER d.address.city == "Paris" RETURN d"#,
        );
        assert!(
            index_names(&engine, "items").contains(&"_auto_address.city".to_string()),
            "{:?}",
            index_names(&engine, "items")
        );
        items
            .insert(json!({"_key": "2", "address_city": "Lyon"}))
            .unwrap();
        let _ = execute_as_writer(
            &engine,
            r#"FOR d IN items FILTER d.address_city == "Lyon" RETURN d"#,
        );
        let names = index_names(&engine, "items");
        assert!(names.contains(&"_auto_address.city".to_string()));
        assert!(names.contains(&"_auto_address_city".to_string()));
    });
}

/// The backfill runs inside the request that triggered it, so a collection
/// past the ceiling is left alone.
#[test]
fn auto_index_skips_collection_over_doc_ceiling() {
    with_env_cleared(|| {
        let (engine, _tmp) = create_test_engine();
        engine.create_collection("items".to_string(), None).unwrap();
        let items = engine.get_collection("items").unwrap();
        items.enable_auto_index().unwrap();
        items.insert(json!({"_key": "1", "city": "Paris"})).unwrap();
        items.insert(json!({"_key": "2", "city": "Lyon"})).unwrap();

        std::env::set_var("SOLIDB_AUTO_INDEX_MAX_DOCS", "1");
        let rows = execute_as_writer(
            &engine,
            r#"FOR d IN items FILTER d.city == "Paris" RETURN d._key"#,
        );
        assert_eq!(rows, vec![json!("1")], "the query still answers by scan");
        assert!(!index_names(&engine, "items")
            .iter()
            .any(|n| n == "_auto_city"));
    });
}
