//! Vector search by text (card #1003): a string query is embedded with the
//! index's own provider and model, as its documents were, then searched.
//! The provider here is a fake Ollama served by the test.

mod common;
use common::{create_test_engine, execute_query};
use serde_json::{json, Value};
use solidb::storage::VectorIndexConfig;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// A fake Ollama: `/api/embeddings` answers "cat" [1,0,0], "dog" [0,1,0],
/// "wide" a vector of 5 dimensions, anything else [0,0,1]; counts its calls.
fn fake_ollama() -> (String, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let app = axum::Router::new().route(
                "/api/embeddings",
                axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                    let counted = counted.clone();
                    async move {
                        counted.fetch_add(1, Ordering::SeqCst);
                        let embedding = match body["prompt"].as_str().unwrap_or("") {
                            "cat" => json!([1.0, 0.0, 0.0]),
                            "dog" => json!([0.0, 1.0, 0.0]),
                            "wide" => json!([1.0, 0.0, 0.0, 0.0, 0.0]),
                            _ => json!([0.0, 0.0, 1.0]),
                        };
                        axum::Json(json!({ "embedding": embedding }))
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            tx.send(listener.local_addr().unwrap()).unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    });
    let addr = rx.recv().unwrap();
    (format!("http://{addr}"), calls)
}

#[test]
fn a_text_query_is_embedded_with_the_index_provider() {
    // The fake listens on the loopback, which tenant LLM URLs may not reach
    // unless the operator says so.
    std::env::set_var("SOLIDB_ALLOW_PRIVATE_LLM_URL", "1");
    let (url, calls) = fake_ollama();
    let (engine, _tmp) = create_test_engine();
    engine.initialize().unwrap();
    // `_env` holds credentials: created and written as the admin
    // endpoint does, read by the server's own lookups.
    let system = engine.get_database("_system").unwrap();
    system.create_collection("_env".to_string(), None).unwrap();
    system
        .system_collection("_env")
        .unwrap()
        .insert(json!({"_key": "OLLAMA_URL", "value": url}))
        .unwrap();

    engine.create_collection("pets".to_string(), None).unwrap();
    let pets = engine.get_collection("pets").unwrap();
    for (key, name, emb) in [
        ("p1", "cat", [1.0, 0.0, 0.0]),
        ("p2", "dog", [0.0, 1.0, 0.0]),
        ("p3", "fish", [0.0, 0.0, 1.0]),
    ] {
        pets.insert(json!({"_key": key, "name": name, "emb": emb}))
            .unwrap();
    }
    let config = VectorIndexConfig::new("idx".to_string(), "emb".to_string(), 3)
        .with_embedding_source("name".to_string())
        .with_embedding_provider("ollama".to_string())
        .with_embedding_model("fake".to_string());
    pets.create_vector_index(config).unwrap();

    let top = |q: &str| -> String {
        let rows = execute_query(&engine, q);
        rows[0].as_str().unwrap().to_string()
    };
    assert_eq!(
        top(r#"FOR r IN VECTOR_SEARCH("pets", "idx", "dog", 1) RETURN r.doc._key"#),
        "p2"
    );
    assert_eq!(
        top(r#"FOR r IN VECTOR_SEARCH("pets", "idx", "cat", 1) RETURN r.doc._key"#),
        "p1"
    );
    let after_two = calls.load(Ordering::SeqCst);
    assert_eq!(after_two, 2);
    // The same text again is not embedded again.
    assert_eq!(
        top(r#"FOR r IN VECTOR_SEARCH("pets", "idx", "dog", 1) RETURN r.doc._key"#),
        "p2"
    );
    assert_eq!(calls.load(Ordering::SeqCst), after_two);
    // A vector still works, and never calls the provider.
    assert_eq!(
        top(r#"FOR r IN VECTOR_SEARCH("pets", "idx", [0, 0, 1], 1) RETURN r.doc._key"#),
        "p3"
    );
    assert_eq!(calls.load(Ordering::SeqCst), after_two);
}

#[test]
fn a_text_query_is_refused_where_it_cannot_be_compared() {
    std::env::set_var("SOLIDB_ALLOW_PRIVATE_LLM_URL", "1");
    let (url, _calls) = fake_ollama();
    let (engine, _tmp) = create_test_engine();
    engine.initialize().unwrap();
    // `_env` holds credentials: created and written as the admin
    // endpoint does, read by the server's own lookups.
    let system = engine.get_database("_system").unwrap();
    system.create_collection("_env".to_string(), None).unwrap();
    system
        .system_collection("_env")
        .unwrap()
        .insert(json!({"_key": "OLLAMA_URL", "value": url}))
        .unwrap();
    engine.create_collection("docs".to_string(), None).unwrap();
    let docs = engine.get_collection("docs").unwrap();
    docs.insert(json!({"_key": "d1", "emb": [1.0, 0.0, 0.0]}))
        .unwrap();
    // Vectors brought from elsewhere: a text cannot be put beside them.
    docs.create_vector_index(VectorIndexConfig::new(
        "plain".to_string(),
        "emb".to_string(),
        3,
    ))
    .unwrap();
    // An index whose provider answers in another dimension.
    docs.create_vector_index(
        VectorIndexConfig::new("odd".to_string(), "emb".to_string(), 3)
            .with_embedding_provider("ollama".to_string()),
    )
    .unwrap();

    let run = |q: &str| -> Result<Vec<Value>, String> {
        let query = solidb::sdbql::parse(q).map_err(|e| e.to_string())?;
        solidb::sdbql::QueryExecutor::new(&engine)
            .execute(&query)
            .map_err(|e| e.to_string())
    };
    let err = run(r#"FOR r IN VECTOR_SEARCH("docs", "plain", "cat", 1) RETURN r"#).unwrap_err();
    assert!(err.contains("no embedding source"), "{err}");
    let err = run(r#"FOR r IN VECTOR_SEARCH("docs", "odd", "wide", 1) RETURN r"#).unwrap_err();
    assert!(err.contains("5 dimensions"), "{err}");
    // HYBRID_SEARCH: null embeds the text query itself.
    docs.create_vector_index(
        VectorIndexConfig::new("named".to_string(), "emb".to_string(), 3)
            .with_embedding_source("name".to_string())
            .with_embedding_provider("ollama".to_string()),
    )
    .unwrap();
    let rows = run(r#"RETURN LENGTH(HYBRID_SEARCH("docs", "named", "name", null, "cat"))"#);
    assert!(rows.is_ok(), "{rows:?}");
}
