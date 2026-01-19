//! Lua Crypto Integration Tests
//!
//! Tests for the cryptographic functions exposed to Lua:
//! - Hashing (MD5, SHA256, SHA512)
//! - HMAC
//! - Encoding (Base64, Hex)
//! - JWT
//! - Password Hashing (Argon2)
//! - Key Exchange (Curve25519)

use serde_json::json;
use solidb::scripting::{Script, ScriptContext, ScriptEngine, ScriptStats, ScriptUser};
use solidb::storage::StorageEngine;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

fn create_test_env() -> (Arc<StorageEngine>, ScriptEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = Arc::new(
        StorageEngine::new(tmp_dir.path().to_str().unwrap())
            .expect("Failed to create storage engine"),
    );

    // Create DB
    engine.create_database("testdb".to_string()).unwrap();

    let stats = Arc::new(ScriptStats::default());
    let script_engine = ScriptEngine::new(engine.clone(), stats);

    (engine, script_engine, tmp_dir)
}

fn create_context() -> ScriptContext {
    ScriptContext {
        method: "POST".to_string(),
        path: "/test".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: Some(json!({})),
        is_websocket: false,
        user: ScriptUser::anonymous(),
    }
}

fn create_script(code: &str) -> Script {
    Script {
        key: "crypto_test".to_string(),
        name: "Crypto Test".to_string(),
        methods: vec!["POST".to_string()],
        path: "/crypto".to_string(),
        database: "testdb".to_string(),
        collection: None,
        code: code.to_string(),
        description: None,
        created_at: "".to_string(),
        updated_at: "".to_string(),
    }
}

#[tokio::test]
async fn test_lua_crypto_hashing() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        return {
            md5 = crypto.md5("hello"),
            sha256 = crypto.sha256("hello"),
            sha512 = crypto.sha512("hello")
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(
        body.get("md5").unwrap().as_str().unwrap(),
        "5d41402abc4b2a76b9719d911017c592"
    );
    assert_eq!(
        body.get("sha256").unwrap().as_str().unwrap(),
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}

#[tokio::test]
async fn test_lua_crypto_encoding() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local original = "Hello World"
        local b64 = crypto.base64_encode(original)
        local hex = crypto.hex_encode(original)

        return {
            b64 = b64,
            decoded_b64 = crypto.base64_decode(b64),
            hex = hex,
            decoded_hex = crypto.hex_decode(hex)
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(
        body.get("b64").unwrap().as_str().unwrap(),
        "SGVsbG8gV29ybGQ="
    );
    assert_eq!(
        body.get("decoded_b64").unwrap().as_str().unwrap(),
        "Hello World"
    );
    assert_eq!(
        body.get("decoded_hex").unwrap().as_str().unwrap(),
        "Hello World"
    );
}

#[tokio::test]
async fn test_lua_crypto_password() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local password = "my_secret_password"
        local hash = crypto.hash_password(password)
        local valid = crypto.verify_password(hash, password)
        local invalid = crypto.verify_password(hash, "wrong_password")

        return {
            hash = hash,
            valid = valid,
            invalid = invalid
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert!(body
        .get("hash")
        .unwrap()
        .as_str()
        .unwrap()
        .starts_with("$argon2"));
    assert_eq!(body.get("valid").unwrap().as_bool().unwrap(), true);
    assert_eq!(body.get("invalid").unwrap().as_bool().unwrap(), false);
}

#[tokio::test]
async fn test_lua_crypto_jwt() {
    let (_engine, script_engine, _tmp) = create_test_env();

    let code = r#"
        local claims = { sub = "123", name = "John Doe", admin = true }
        local secret = "my_jwt_secret"

        local token = crypto.jwt_encode(claims, secret)
        local decoded = crypto.jwt_decode(token, secret)

        -- Default alg is HS256
        return {
            token = token,
            decoded_sub = decoded.sub,
            decoded_admin = decoded.admin
        }
    "#;

    let script = create_script(code);
    let ctx = create_context();

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    let token = body.get("token").unwrap().as_str().unwrap();
    assert!(token.split('.').count() == 3);

    assert_eq!(body.get("decoded_sub").unwrap().as_str().unwrap(), "123");
    assert_eq!(body.get("decoded_admin").unwrap().as_bool().unwrap(), true);
}

#[tokio::test]
async fn test_lua_crypto_curve25519() {
    let (_engine, script_engine, _tmp) = create_test_env();
    let ctx = create_context();

    // We keep logic entirely within Lua to avoid utf8 issues with JSON serialization of raw bytes
    let code = r#"
        -- We'll use mocked bytes to ensure we can return them for debug,
        -- but normally we use crypto.random_bytes(32)
        -- Since JSON requires UTF-8, we'll encode logic in Lua entirely or use base64

        local alice_secret = crypto.random_bytes(32)
        local bob_secret = crypto.random_bytes(32)

        -- 2. Generate public keys
        local alice_pub = crypto.curve25519(alice_secret, "")
        local bob_pub = crypto.curve25519(bob_secret, "")

        -- 3. Calculate shared secrets
        local alice_shared = crypto.curve25519(alice_secret, bob_pub)
        local bob_shared = crypto.curve25519(bob_secret, alice_pub)

        return {
             match = (alice_shared == bob_shared),
             len = #alice_shared
        }
    "#;

    let script = create_script(code);

    let result = script_engine
        .execute(&script, "testdb", &ctx)
        .await
        .unwrap();
    let body = result.body.as_object().unwrap();

    assert_eq!(body.get("match").unwrap().as_bool().unwrap(), true);
    assert_eq!(body.get("len").unwrap().as_i64().unwrap(), 32);
}
