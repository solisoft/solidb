//! `NEAR(collection, center, radius, field)` reads the geo index.
//!
//! Results are the documents inside the radius, closest first, ties broken
//! by `_key`. That order is what a pushed-down `LIMIT` keeps a prefix of, so
//! the tests cover the function, the filter that follows it, and `EXPLAIN`.

use std::collections::HashMap;

use serde_json::{json, Value};
use solidb::sdbql::QueryPrincipal;
use solidb::storage::StorageEngine;
use solidb::{distance_meters, parse, QueryExecutor};
use tempfile::TempDir;

const DB: &str = "testdb";
const CENTER_LAT: f64 = 48.8566;
const CENTER_LON: f64 = 2.3522;

fn engine() -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).expect("engine");
    engine.create_database(DB.to_string()).unwrap();
    (engine, tmp)
}

fn places(engine: &StorageEngine) -> solidb::Collection {
    engine
        .get_database(DB)
        .unwrap()
        .get_collection("places")
        .unwrap()
}

fn seed(engine: &StorageEngine) {
    let db = engine.get_database(DB).unwrap();
    db.create_collection("places".to_string(), None).unwrap();
    let c = db.get_collection("places").unwrap();
    // Distances from Paris, roughly: Lille 200 km, London 340 km, Lyon 390 km.
    let rows = [
        ("aa-closed", CENTER_LAT, CENTER_LON, false, "other"),
        ("bb-paris", CENTER_LAT, CENTER_LON, true, "acme"),
        ("cc-paris", CENTER_LAT, CENTER_LON, true, "other"),
        ("lille", 50.6292, 3.0573, true, "acme"),
        ("london", 51.5074, -0.1278, true, "acme"),
        ("lyon", 45.7640, 4.8357, true, "acme"),
        ("nyc", 40.7128, -74.0060, true, "acme"),
    ];
    for (key, lat, lon, open, tenant) in rows {
        c.insert(json!({
            "_key": key,
            "loc": {"lat": lat, "lon": lon},
            "open": open,
            "tenant": tenant,
        }))
        .unwrap();
    }
    c.insert(json!({"_key": "nowhere", "open": true, "tenant": "acme"}))
        .unwrap();
    c.insert(json!({
        "_key": "bad",
        "loc": {"lat": "nope", "lon": 2.0},
        "open": true,
        "tenant": "acme",
    }))
    .unwrap();
    c.create_geo_index("geo_loc".to_string(), "loc".to_string())
        .unwrap();
}

fn binds(radius: f64) -> HashMap<String, Value> {
    HashMap::from([
        (
            "here".to_string(),
            json!({"lat": CENTER_LAT, "lon": CENTER_LON}),
        ),
        ("r".to_string(), json!(radius)),
    ])
}

fn run(engine: &StorageEngine, q: &str) -> Vec<Value> {
    run_with(engine, q, HashMap::new())
}

fn run_with(engine: &StorageEngine, q: &str, binds: HashMap<String, Value>) -> Vec<Value> {
    let exec = QueryExecutor::with_database_and_bind_vars(engine, DB.to_string(), binds);
    exec.execute(&parse(q).unwrap_or_else(|e| panic!("parse {q}: {e}")))
        .unwrap_or_else(|e| panic!("execute {q}: {e}"))
}

fn run_err(engine: &StorageEngine, q: &str) -> String {
    let exec = QueryExecutor::with_database(engine, DB.to_string());
    exec.execute(&parse(q).unwrap_or_else(|e| panic!("parse {q}: {e}")))
        .expect_err(&format!("expected {q} to fail"))
        .to_string()
}

fn keys(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .map(|v| {
            v.as_str()
                .or_else(|| v.get("_key").and_then(Value::as_str))
                .unwrap_or_else(|| panic!("no key in {v}"))
                .to_string()
        })
        .collect()
}

/// Radius that includes Lille and excludes London.
fn radius_m() -> f64 {
    let lille = distance_meters(CENTER_LAT, CENTER_LON, 50.6292, 3.0573);
    let london = distance_meters(CENTER_LAT, CENTER_LON, 51.5074, -0.1278);
    assert!(lille + 1_000.0 < london, "lille {lille} london {london}");
    lille + 1_000.0
}

const NEAR: &str = r#"FOR p IN NEAR("places", @here, @r, "loc") RETURN p._key"#;

fn near(engine: &StorageEngine, radius: f64) -> Vec<Value> {
    run_with(engine, NEAR, binds(radius))
}

#[test]
fn near_matches_a_guarded_distance_filter_and_orders_by_distance_then_key() {
    let (engine, _tmp) = engine();
    seed(&engine);
    let radius = radius_m();

    let near = near(&engine, radius);
    let scan = run_with(
        &engine,
        r#"FOR p IN places
             FILTER IS_NUMBER(p.loc.lat) AND GEO_DISTANCE(p.loc, @here) <= @r
             SORT GEO_DISTANCE(p.loc, @here), p._key
             RETURN p._key"#,
        binds(radius),
    );
    assert_eq!(near, scan);
    assert_eq!(
        near,
        vec![
            json!("aa-closed"),
            json!("bb-paris"),
            json!("cc-paris"),
            json!("lille"),
        ]
    );
    // Missing and unreadable points are not hits. London is outside the radius.
    assert!(!near
        .iter()
        .any(|k| k == "nowhere" || k == "bad" || k == "london"));
}

#[test]
fn near_accepts_geojson_array_and_geo_point_centers() {
    let (engine, _tmp) = engine();
    seed(&engine);
    let radius = radius_m();
    let object = near(&engine, radius);
    // Bare arrays are [lon, lat], the same convention as GEO_DISTANCE.
    let mut array_binds = binds(radius);
    array_binds.insert("here".to_string(), json!([CENTER_LON, CENTER_LAT]));
    let array = run_with(&engine, NEAR, array_binds);
    let point = run_with(
        &engine,
        r#"FOR p IN NEAR("places", GEO_POINT(@lat, @lon), @r, "loc") RETURN p._key"#,
        HashMap::from([
            ("lat".to_string(), json!(CENTER_LAT)),
            ("lon".to_string(), json!(CENTER_LON)),
            ("r".to_string(), json!(radius)),
        ]),
    );
    assert_eq!(object, array);
    assert_eq!(object, point);
    let lower = run_with(
        &engine,
        r#"FOR p IN near("places", @here, @r, "loc") RETURN p._key"#,
        binds(radius),
    );
    assert_eq!(object, lower);
}

#[test]
fn limit_keeps_a_prefix_of_the_nearest_and_a_filter_runs_first() {
    let (engine, _tmp) = engine();
    seed(&engine);
    let radius = radius_m();
    let b = binds(radius);
    assert_eq!(
        run_with(
            &engine,
            r#"FOR p IN NEAR("places", @here, @r, "loc") LIMIT 1 RETURN p._key"#,
            b.clone(),
        ),
        vec![json!("aa-closed")]
    );
    assert_eq!(
        run_with(
            &engine,
            r#"FOR p IN NEAR("places", @here, @r, "loc") LIMIT 1 OFFSET 1 RETURN p._key"#,
            b.clone(),
        ),
        vec![json!("bb-paris")]
    );
    // The closed document is the nearest. LIMIT must not drop it before FILTER.
    assert_eq!(
        run_with(
            &engine,
            r#"FOR p IN NEAR("places", @here, @r, "loc") FILTER p.open == true LIMIT 1 RETURN p._key"#,
            b,
        ),
        vec![json!("bb-paris")]
    );
}

#[test]
fn radius_is_inclusive_and_zero_keeps_only_the_center() {
    let (engine, _tmp) = engine();
    seed(&engine);
    // The radius is the distance the builtin itself reports, so the bound is
    // the same number both sides compare against.
    let lyon = run_with(
        &engine,
        r#"FOR p IN places FILTER p._key == "lyon" RETURN GEO_DISTANCE(p.loc, @here)"#,
        binds(0.0),
    );
    let lyon_m = lyon[0].as_f64().unwrap();
    assert!(lyon_m > 1.0);
    assert!(keys(&near(&engine, lyon_m)).contains(&"lyon".to_string()));
    assert!(!keys(&near(&engine, lyon_m - 1.0)).contains(&"lyon".to_string()));

    assert_eq!(
        near(&engine, 0.0),
        vec![json!("aa-closed"), json!("bb-paris"), json!("cc-paris")]
    );
    assert!(run_with(
        &engine,
        NEAR,
        HashMap::from([
            ("here".to_string(), json!({"lat": 0.0, "lon": 0.0})),
            ("r".to_string(), json!(1.0)),
        ]),
    )
    .is_empty());
}

#[test]
fn documents_inserted_after_the_index_and_nested_fields_are_found() {
    let (engine, _tmp) = engine();
    seed(&engine);
    places(&engine)
        .insert(json!({
            "_key": "added",
            "loc": {"lat": CENTER_LAT, "lon": CENTER_LON},
            "open": true,
            "tenant": "acme",
        }))
        .unwrap();
    let keys = keys(&near(&engine, 0.0));
    assert!(keys.contains(&"added".to_string()), "{keys:?}");

    let db = engine.get_database(DB).unwrap();
    db.create_collection("shops".to_string(), None).unwrap();
    let shops = db.get_collection("shops").unwrap();
    shops
        .insert(json!({
            "_key": "corner",
            "addr": {"pos": {"lat": 48.85, "lon": 2.35}},
        }))
        .unwrap();
    shops
        .insert(json!({"_key": "far", "addr": {"pos": {"lat": 0, "lon": 0}}}))
        .unwrap();
    shops
        .create_geo_index("geo_pos".to_string(), "addr.pos".to_string())
        .unwrap();
    assert_eq!(
        run(
            &engine,
            r#"FOR s IN NEAR("shops", {lat: 48.85, lon: 2.35}, 1000, "addr.pos") RETURN s._key"#,
        ),
        vec![json!("corner")]
    );
}

#[test]
fn bind_variables_work_and_explain_names_the_geo_index() {
    let (engine, _tmp) = engine();
    seed(&engine);
    let radius = radius_m();
    let binds = HashMap::from([
        ("coll".to_string(), json!("places")),
        (
            "here".to_string(),
            json!({"lat": CENTER_LAT, "lon": CENTER_LON}),
        ),
        ("radius".to_string(), json!(radius)),
        ("field".to_string(), json!("loc")),
    ]);
    let q = r#"FOR p IN NEAR(@coll, @here, @radius, @field) RETURN p._key"#;
    assert_eq!(run_with(&engine, q, binds.clone()), near(&engine, radius));

    let exec = QueryExecutor::with_database_and_bind_vars(&engine, DB.to_string(), binds);
    let explained = exec.explain(&parse(q).unwrap()).unwrap();
    assert_eq!(explained.collections[0].access_type, "geo_near");
    assert_eq!(explained.collections[0].name, "places");
    assert_eq!(explained.collections[0].index_type.as_deref(), Some("Geo"));
    assert_eq!(
        explained.collections[0].index_used.as_deref(),
        Some("geo_loc")
    );
    assert!(
        explained
            .warnings
            .iter()
            .any(|w| w.contains("geo index 'geo_loc'")),
        "{:?}",
        explained.warnings
    );
}

#[test]
fn missing_index_bad_arguments_and_protected_collections_fail() {
    let (engine, _tmp) = engine();
    seed(&engine);
    let db = engine.get_database(DB).unwrap();
    db.create_collection("bare".to_string(), None).unwrap();
    db.get_collection("bare")
        .unwrap()
        .insert(json!({"_key": "x", "loc": {"lat": 1, "lon": 2}}))
        .unwrap();

    let err = run_err(
        &engine,
        r#"RETURN NEAR("bare", {lat: 1, lon: 2}, 10, "loc")"#,
    );
    assert!(err.contains("no geo index"), "{err}");
    assert!(err.contains("bare"), "{err}");

    let err = run_err(
        &engine,
        r#"RETURN NEAR("places", {lat: 1, lon: 2}, 10, "missing")"#,
    );
    assert!(err.contains("no geo index"), "{err}");

    for q in [
        r#"RETURN NEAR("places", {lat: 1, lon: 2}, -1, "loc")"#,
        r#"RETURN NEAR("places", {lat: 1, lon: 2}, "10", "loc")"#,
        r#"RETURN NEAR("places", [1], 10, "loc")"#,
        r#"RETURN NEAR("places", {lat: 91, lon: 0}, 10, "loc")"#,
        r#"RETURN NEAR("places", null, 10, "loc")"#,
        r#"RETURN NEAR(1, {lat: 1, lon: 2}, 10, "loc")"#,
        r#"RETURN NEAR("places", {lat: 1, lon: 2}, 10, "")"#,
        r#"RETURN NEAR("places", {lat: 1, lon: 2}, 10)"#,
    ] {
        let err = run_err(&engine, q);
        assert!(!err.is_empty(), "{q}");
        assert!(!err.contains("Unknown function"), "{q}: {err}");
    }

    let err = run_err(
        &engine,
        r#"RETURN NEAR("_env", {lat: 0, lon: 0}, 1, "loc")"#,
    );
    assert!(err.contains("_env"), "{err}");
    assert!(err.contains("Access denied"), "{err}");
}

#[test]
fn row_policy_drops_hidden_documents_and_keeps_distance_order() {
    let (engine, _tmp) = engine();
    seed(&engine);
    let radius = radius_m();
    let admin = QueryPrincipal {
        user: "root".into(),
        roles: vec!["admin".into()],
        can_read: true,
        can_write: true,
        can_admin: true,
    };
    let set = r#"RETURN ROW_POLICY("places", "doc.tenant == \"acme\"")"#;
    QueryExecutor::with_database(&engine, DB.to_string())
        .with_principal(admin)
        .execute(&parse(set).unwrap())
        .unwrap();

    let viewer = QueryPrincipal {
        user: "ada".into(),
        roles: vec!["viewer".into()],
        can_read: true,
        can_write: false,
        can_admin: false,
    };
    let seen = QueryExecutor::with_database_and_bind_vars(&engine, DB.to_string(), binds(radius))
        .with_principal(viewer)
        .execute(&parse(NEAR).unwrap())
        .unwrap();
    // aa-closed and cc-paris are the other tenant; both sit at the center,
    // so a policy applied after the sort still leaves bb-paris ahead of lille.
    assert_eq!(seen, vec![json!("bb-paris"), json!("lille")]);
}
