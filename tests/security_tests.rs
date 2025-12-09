//! Security-related tests for SolidB authentication and cluster features

use std::collections::HashMap;

/// Test HMAC-SHA256 authentication response computation
/// This tests the cryptographic primitives used for cluster node authentication
#[test]
fn test_hmac_auth_response() {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    
    type HmacSha256 = Hmac<Sha256>;
    
    // Helper function matching the one in cluster/service.rs
    fn compute_auth_response(challenge: &str, keyfile: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(keyfile.as_bytes())
            .expect("HMAC can accept any key length");
        mac.update(challenge.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }
    
    let keyfile = "my-secret-cluster-key";
    let challenge = "random-challenge-uuid";
    
    // Compute response
    let response = compute_auth_response(challenge, keyfile);
    
    // Response should be 64 hex characters (256 bits = 32 bytes = 64 hex)
    assert_eq!(response.len(), 64, "HMAC-SHA256 should produce 64 hex characters");
    
    // Response should be deterministic
    let response2 = compute_auth_response(challenge, keyfile);
    assert_eq!(response, response2, "Same inputs should produce same response");
    
    // Different challenge should produce different response
    let response3 = compute_auth_response("different-challenge", keyfile);
    assert_ne!(response, response3, "Different challenge should produce different response");
    
    // Different keyfile should produce different response
    let response4 = compute_auth_response(challenge, "different-key");
    assert_ne!(response, response4, "Different keyfile should produce different response");
}

/// Test that HMAC responses are cryptographically distinct
#[test]
fn test_hmac_collision_resistance() {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    
    type HmacSha256 = Hmac<Sha256>;
    
    fn compute_auth_response(challenge: &str, keyfile: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(keyfile.as_bytes())
            .expect("HMAC can accept any key length");
        mac.update(challenge.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }
    
    let keyfile = "test-key";
    let mut responses = HashMap::new();
    
    // Generate 1000 responses with different challenges
    for i in 0..1000 {
        let challenge = format!("challenge-{}", i);
        let response = compute_auth_response(&challenge, keyfile);
        
        // Ensure no collisions
        assert!(
            !responses.contains_key(&response),
            "Found collision at iteration {}", i
        );
        responses.insert(response, i);
    }
}

/// Test password hashing with Argon2
#[test]
fn test_password_hashing() {
    use argon2::{
        password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
        Argon2,
    };
    use rand_core::OsRng;
    
    let password = "secure-password-123";
    
    // Hash password
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .expect("Hashing should succeed")
        .to_string();
    
    // Verify correct password
    let parsed_hash = PasswordHash::new(&hash).expect("Hash should be parseable");
    assert!(
        argon2.verify_password(password.as_bytes(), &parsed_hash).is_ok(),
        "Correct password should verify"
    );
    
    // Verify wrong password fails
    assert!(
        argon2.verify_password(b"wrong-password", &parsed_hash).is_err(),
        "Wrong password should not verify"
    );
}

/// Test that generated passwords have sufficient entropy
#[test]
fn test_generated_password_entropy() {
    use rand_core::{OsRng, RngCore};
    
    // Generate password the same way as AuthService::init
    let mut password_bytes = [0u8; 16];
    OsRng.fill_bytes(&mut password_bytes);
    let generated_password = hex::encode(password_bytes);
    
    // Should be 32 hex characters (16 bytes = 128 bits of entropy)
    assert_eq!(generated_password.len(), 32, "Generated password should be 32 hex chars");
    
    // Generate another password - should be different
    let mut password_bytes2 = [0u8; 16];
    OsRng.fill_bytes(&mut password_bytes2);
    let generated_password2 = hex::encode(password_bytes2);
    
    assert_ne!(
        generated_password, generated_password2,
        "Generated passwords should be unique"
    );
}

/// Test JWT secret generation
#[test]
fn test_jwt_secret_generation() {
    use rand_core::{OsRng, RngCore};
    
    // Generate secret the same way as done in auth.rs when JWT_SECRET is not set
    let mut key_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut key_bytes);
    let generated = hex::encode(key_bytes);
    
    // Should be 64 hex characters (32 bytes = 256 bits)
    assert_eq!(generated.len(), 64, "Generated JWT secret should be 64 hex chars");
    
    // All characters should be valid hex
    assert!(
        generated.chars().all(|c| c.is_ascii_hexdigit()),
        "All characters should be hex digits"
    );
}

/// Test API key generation and hashing
#[test]
fn test_api_key_generation() {
    use rand_core::{OsRng, RngCore};
    use sha2::{Sha256, Digest};
    
    // Generate API key the same way as AuthService::generate_api_key
    let mut key_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut key_bytes);
    let raw_key = format!("sk_{}", hex::encode(key_bytes));
    
    // Hash with SHA-256
    let mut hasher = Sha256::new();
    hasher.update(raw_key.as_bytes());
    let key_hash = hex::encode(hasher.finalize());
    
    // Raw key should start with "sk_" and be 67 chars (3 + 64)
    assert!(raw_key.starts_with("sk_"), "API key should start with sk_");
    assert_eq!(raw_key.len(), 67, "API key should be 67 characters");
    
    // Hash should be 64 hex chars
    assert_eq!(key_hash.len(), 64, "Hash should be 64 hex characters");
    
    // Same key should produce same hash
    let mut hasher2 = Sha256::new();
    hasher2.update(raw_key.as_bytes());
    let key_hash2 = hex::encode(hasher2.finalize());
    assert_eq!(key_hash, key_hash2, "Same key should produce same hash");
}

/// Test constant-time comparison (timing attack prevention)
#[test]
fn test_constant_time_comparison() {
    fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        a.iter().zip(b.iter()).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
    }
    
    let a = b"secret-key-hash-12345";
    let b = b"secret-key-hash-12345";
    let c = b"secret-key-hash-12346";
    let d = b"different-length";
    
    assert!(constant_time_eq(a, b), "Equal values should match");
    assert!(!constant_time_eq(a, c), "Different values should not match");
    assert!(!constant_time_eq(a, d), "Different lengths should not match");
}
