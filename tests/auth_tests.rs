//! Auth Service Coverage Tests
//!
//! Comprehensive tests for server/auth.rs including:
//! - Password hashing and verification
//! - JWT token creation and validation
//! - API key generation and validation
//! - Rate limiting

use solidb::storage::StorageEngine;
use solidb::server::auth::AuthService;
use std::sync::Arc;
use tempfile::TempDir;

fn create_test_engine() -> (Arc<StorageEngine>, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (Arc::new(engine), tmp_dir)
}

// ============================================================================
// Password Hashing Tests
// ============================================================================

#[test]
fn test_password_hash_and_verify() {
    let password = "mysecretpassword123";
    
    // Hash password
    let hash = AuthService::hash_password(password).expect("Hashing should succeed");
    
    // Hash should not be the same as password
    assert_ne!(hash, password);
    
    // Verify correct password
    assert!(AuthService::verify_password(password, &hash), "Correct password should verify");
    
    // Verify wrong password
    assert!(!AuthService::verify_password("wrongpassword", &hash), "Wrong password should not verify");
}

#[test]
fn test_password_hash_is_unique() {
    let password = "samepassword";
    
    // Hash the same password twice
    let hash1 = AuthService::hash_password(password).unwrap();
    let hash2 = AuthService::hash_password(password).unwrap();
    
    // Due to salting, hashes should be different
    assert_ne!(hash1, hash2, "Same password should produce different hashes (salted)");
    
    // But both should verify
    assert!(AuthService::verify_password(password, &hash1));
    assert!(AuthService::verify_password(password, &hash2));
}

#[test]
fn test_password_hash_empty() {
    let password = "";
    
    // Should still work (even if empty password is not recommended)
    let result = AuthService::hash_password(password);
    assert!(result.is_ok());
}

#[test]
fn test_password_verify_invalid_hash() {
    // Verify against invalid hash format
    let result = AuthService::verify_password("password", "not_a_valid_hash");
    assert!(!result, "Invalid hash format should return false");
}

// ============================================================================
// JWT Tests
// ============================================================================

#[test]
fn test_create_and_validate_jwt() {
    let username = "testuser";
    
    // Create JWT
    let token = AuthService::create_jwt(username).expect("JWT creation should succeed");
    
    // Token should be non-empty
    assert!(!token.is_empty());
    
    // Validate token
    let claims = AuthService::validate_token(&token).expect("Token validation should succeed");
    assert_eq!(claims.sub, username);
}

#[test]
fn test_jwt_invalid_token() {
    let result = AuthService::validate_token("invalid.jwt.token");
    assert!(result.is_err(), "Invalid token should fail validation");
}

#[test]
fn test_jwt_different_users() {
    let token1 = AuthService::create_jwt("user1").unwrap();
    let token2 = AuthService::create_jwt("user2").unwrap();
    
    // Tokens should be different
    assert_ne!(token1, token2);
    
    // Each should validate to correct user
    let claims1 = AuthService::validate_token(&token1).unwrap();
    let claims2 = AuthService::validate_token(&token2).unwrap();
    
    assert_eq!(claims1.sub, "user1");
    assert_eq!(claims2.sub, "user2");
}

#[test]
fn test_create_livequery_jwt() {
    // Create livequery token
    let token = AuthService::create_livequery_jwt().expect("Livequery JWT creation should succeed");
    
    // Token should be non-empty
    assert!(!token.is_empty());
    
    // Should validate
    let claims = AuthService::validate_token(&token);
    assert!(claims.is_ok());
}

// ============================================================================
// API Key Tests
// ============================================================================

#[test]
fn test_generate_api_key() {
    let (raw_key, hash) = AuthService::generate_api_key();
    
    // Raw key should have format prefix_key
    assert!(raw_key.contains('_'), "API key should contain underscore separator");
    
    // Hash should be different from raw key
    assert_ne!(raw_key, hash);
    
    // Hash should be hex string
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

// API key hashing is private, so we test generate_api_key which produces consistent hashes internally

#[test]
fn test_validate_api_key_without_keys() {
    let (engine, _tmp) = create_test_engine();
    
    // Initialize auth (creates _system database)
    let _ = AuthService::init(&engine, None);
    
    // Try to validate a non-existent key
    let result = AuthService::validate_api_key(&engine, "sdb_nonexistent_key");
    assert!(result.is_err(), "Non-existent key should fail validation");
}

// ============================================================================
// Auth Initialization Tests
// ============================================================================

#[test]
fn test_auth_init() {
    let (engine, _tmp) = create_test_engine();
    
    // Create _system database first
    engine.create_database("_system".to_string()).unwrap();
    
    // Initialize auth
    let result = AuthService::init(&engine, None);
    assert!(result.is_ok(), "Auth init should succeed: {:?}", result.err());
}

#[test]
fn test_auth_init_idempotent() {
    let (engine, _tmp) = create_test_engine();
    
    // Create _system database first
    engine.create_database("_system".to_string()).unwrap();
    
    // Initialize multiple times
    let result1 = AuthService::init(&engine, None);
    let result2 = AuthService::init(&engine, None);
    let result3 = AuthService::init(&engine, None);
    
    assert!(result1.is_ok());
    assert!(result2.is_ok());
    assert!(result3.is_ok());
}


// ============================================================================
// Claims Tests
// ============================================================================

#[test]
fn test_jwt_contains_expiry() {
    let token = AuthService::create_jwt("testuser").unwrap();
    let claims = AuthService::validate_token(&token).unwrap();
    
    // Claims should have expiry
    assert!(claims.exp > 0, "JWT should have expiry timestamp");
}

#[test]
fn test_jwt_subject_preserved() {
    // Test various usernames
    let usernames = vec!["admin", "user@domain.com", "user-123", "CamelCaseUser"];
    
    for username in usernames {
        let token = AuthService::create_jwt(username).unwrap();
        let claims = AuthService::validate_token(&token).unwrap();
        assert_eq!(claims.sub, username, "Subject should match for '{}'", username);
    }
}

// ============================================================================
// Edge Cases Tests
// ============================================================================

#[test]
fn test_long_password() {
    let password = "a".repeat(1000);
    
    let hash = AuthService::hash_password(&password).unwrap();
    assert!(AuthService::verify_password(&password, &hash));
}

#[test]
fn test_unicode_password() {
    let password = "密码🔐пароль";
    
    let hash = AuthService::hash_password(password).unwrap();
    assert!(AuthService::verify_password(password, &hash));
}

#[test]
fn test_special_chars_password() {
    let password = "p@$$w0rd!#$%^&*(){}|:<>?";
    
    let hash = AuthService::hash_password(password).unwrap();
    assert!(AuthService::verify_password(password, &hash));
}
