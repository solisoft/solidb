use solidb::scripting::{ScriptEngine, Script, ScriptContext};
use solidb::storage::StorageEngine;
use std::sync::Arc;
use std::collections::HashMap;

#[tokio::test]
async fn verify_lua_crypto() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(StorageEngine::new(tmp_dir.path()).unwrap());
    let engine = ScriptEngine::new(storage);

    // Crypto Test Script
    // We confirm correct values for standard algorithms and sanity check others
    let script_code = r#"
        local res = {}
        
        -- Hashing
        res.md5 = crypto.md5("hello")
        res.sha256 = crypto.sha256("hello")
        
        -- HMAC
        res.hmac256 = crypto.hmac_sha256("secret", "message")
        res.hmac512 = crypto.hmac_sha512("secret", "message")
        
        -- Encoding
        res.b64_enc = crypto.base64_encode("hello")
        res.b64_dec = crypto.base64_decode(res.b64_enc)
        res.b32_enc = crypto.base32_encode("hello")
        res.b32_dec = crypto.base32_decode(res.b32_enc)
        
        res.hex_enc = crypto.hex_encode("hello")
        res.hex_dec = crypto.hex_decode(res.hex_enc)
        
        -- UUID
        res.uuid = crypto.uuid()
        res.uuid7 = crypto.uuid_v7()
        
        -- Random
        local rnd = crypto.random_bytes(10)
        res.rand_len = #rnd
        
        -- Password (Async check)
        -- Note: mlua eval_async handles async function calls from Lua by suspending/resuming
        local hash = crypto.hash_password("s3cr3t")
        res.pw_verify_true = crypto.verify_password(hash, "s3cr3t")
        res.pw_verify_false = crypto.verify_password(hash, "wrong")
        
        -- Curve25519
        local alice_priv = crypto.random_bytes(32)
        local alice_pub = crypto.curve25519(alice_priv, "\9")
        local bob_priv = crypto.random_bytes(32)
        local bob_pub = crypto.curve25519(bob_priv, "\9")
        
        local shared_alice = crypto.curve25519(alice_priv, bob_pub)
        local shared_bob = crypto.curve25519(bob_priv, alice_pub)
        
        res.curve_match = (shared_alice == shared_bob)
        res.curve_len = #shared_alice
        res.pub_len = #alice_pub
        
        return res
    "#;

    let script = Script {
        key: "test_crypto".to_string(),
        name: "Test Crypto".to_string(),
        methods: vec!["GET".to_string()],
        path: "crypto".to_string(),
        database: "_system".to_string(),
        collection: None,
        code: script_code.to_string(),
        description: None,
        created_at: "now".to_string(),
        updated_at: "now".to_string(),
    };

    let context = ScriptContext {
        method: "GET".to_string(),
        path: "crypto".to_string(),
        query_params: HashMap::new(),
        params: HashMap::new(),
        headers: HashMap::new(),
        body: None,
    };

    let result = engine.execute(&script, "_system", &context).await.unwrap();
    let body = &result.body;
    
    // Assertions
    // MD5("hello") = 5d41402abc4b2a76b9719d911017c592
    assert_eq!(body["md5"], "5d41402abc4b2a76b9719d911017c592");
    
    // SHA256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
    assert_eq!(body["sha256"], "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    
    // Base64("hello") = "aGVsbG8="
    assert_eq!(body["b64_enc"], "aGVsbG8=");
    assert_eq!(body["b64_dec"], "hello");
    
    // Base32("hello") = "NBSWY3DP"
    assert_eq!(body["b32_enc"], "NBSWY3DP");
    assert_eq!(body["b32_dec"], "hello");
    
    // Hex("hello") = "68656c6c6f"
    assert_eq!(body["hex_enc"], "68656c6c6f");
    assert_eq!(body["hex_dec"], "hello");
    
    // UUID
    assert!(body["uuid"].is_string());
    assert_eq!(body["uuid"].as_str().unwrap().len(), 36);
    
    assert!(body["uuid7"].is_string());
    assert_eq!(body["uuid7"].as_str().unwrap().len(), 36);
    
    // Random
    assert_eq!(body["rand_len"], 10);
    
    // Password
    assert_eq!(body["pw_verify_true"], true);
    assert_eq!(body["pw_verify_false"], false);
    
    // Curve25519
    assert_eq!(body["curve_match"], true);
    assert_eq!(body["curve_len"], 32);
    assert_eq!(body["pub_len"], 32);
}
