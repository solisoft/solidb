//! Custom JWT implementation for scripting

use base64::Engine;
use hmac::Mac;

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub(crate) struct Header {
    pub alg: String,
    pub typ: String,
}

impl Header {
    pub fn default() -> Self {
        Self {
            alg: "HS256".to_string(),
            typ: "JWT".to_string(),
        }
    }
}

/// What `decode` checks beyond the signature.
///
/// This used to be a unit struct and the argument was named `_validation`:
/// the signature was verified but `exp` and `nbf` were not, so a script using
/// `crypto.jwt_decode(token, secret)` to authenticate a session accepted a
/// token that expired months ago. A verified signature says the token was
/// issued by someone holding the secret, not that it is still valid.
#[derive(Debug, Clone)]
pub(crate) struct Validation {
    /// Reject a token whose `exp` has passed. On by default.
    pub validate_exp: bool,
    /// Reject a token whose `nbf` is still in the future. On by default.
    pub validate_nbf: bool,
    /// Clock-skew allowance, in seconds.
    pub leeway: u64,
}

impl Validation {
    pub fn default() -> Self {
        Self {
            validate_exp: true,
            validate_nbf: true,
            leeway: 5,
        }
    }
}

#[derive(Debug)]
pub(crate) struct EncodingKey(pub Vec<u8>);

impl EncodingKey {
    pub fn from_secret(secret: &[u8]) -> Self {
        Self(secret.to_vec())
    }
}

#[derive(Debug)]
pub(crate) struct DecodingKey(pub Vec<u8>);

impl DecodingKey {
    pub fn from_secret(secret: &[u8]) -> Self {
        Self(secret.to_vec())
    }
}

#[derive(Debug)]
pub(crate) struct TokenData<T> {
    #[allow(dead_code)]
    pub header: Header,
    pub claims: T,
}

pub(crate) fn encode<T: serde::Serialize>(
    _header: &Header,
    claims: &T,
    key: &EncodingKey,
) -> Result<String, String> {
    // JWT Header: {"alg":"HS256","typ":"JWT"}
    let header = r#"{"alg":"HS256","typ":"JWT"}"#;

    // Base64url encode header
    let header_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(header);

    // Serialize and encode claims
    let claims_json =
        serde_json::to_string(claims).map_err(|e| format!("JWT encode failed: {}", e))?;
    let claims_b64 =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims_json.as_bytes());

    // Create signing input
    let signing_input = format!("{}.{}", header_b64, claims_b64);

    // Sign with HMAC-SHA256
    let signature = sign_hmac_sha256(&signing_input, &key.0)?;

    // Combine into JWT format: header.claims.signature
    Ok(format!("{}.{}.{}", header_b64, claims_b64, signature))
}

pub(crate) fn decode<T: serde::de::DeserializeOwned>(
    token: &str,
    key: &DecodingKey,
    validation: &Validation,
) -> Result<TokenData<T>, String> {
    // Split JWT into parts
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid JWT format".to_string());
    }

    let (header_b64, claims_b64, signature_b64) = (parts[0], parts[1], parts[2]);

    // Verify header (should be {"alg":"HS256","typ":"JWT"})
    let header_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(header_b64)
        .map_err(|_| "Invalid JWT header".to_string())?;
    let header_str =
        String::from_utf8(header_bytes).map_err(|_| "Invalid JWT header encoding".to_string())?;

    let header_json: serde_json::Value =
        serde_json::from_str(&header_str).map_err(|_| "Invalid JWT header encoding".to_string())?;
    if header_json.get("alg").and_then(|v| v.as_str()) != Some("HS256")
        || header_json.get("typ").and_then(|v| v.as_str()) != Some("JWT")
    {
        return Err("Unsupported JWT algorithm or type".to_string());
    }

    // Verify signature
    let signing_input = format!("{}.{}", header_b64, claims_b64);
    let expected_signature = sign_hmac_sha256(&signing_input, &key.0)?;

    use subtle::ConstantTimeEq;
    if expected_signature
        .as_bytes()
        .ct_eq(signature_b64.as_bytes())
        .unwrap_u8()
        != 1
    {
        return Err("Invalid JWT signature".to_string());
    }

    // Decode claims
    let claims_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(claims_b64)
        .map_err(|_| "Invalid JWT claims".to_string())?;

    // Time-based claims are checked on the raw JSON, before deserializing into
    // `T`: callers pass their own claim types and most do not model `exp`.
    let raw_claims: serde_json::Value = serde_json::from_slice(&claims_bytes)
        .map_err(|_| "Invalid JWT claims format".to_string())?;
    check_time_claims(&raw_claims, validation)?;

    let claims: T = serde_json::from_slice(&claims_bytes)
        .map_err(|_| "Invalid JWT claims format".to_string())?;

    Ok(TokenData {
        header: Header::default(),
        claims,
    })
}

pub(crate) fn sign_hmac_sha256(data: &str, secret: &[u8]) -> Result<String, String> {
    use hmac::Hmac;
    use sha2::Sha256;

    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret).map_err(|e| format!("HMAC init failed: {}", e))?;
    mac.update(data.as_bytes());

    let result = mac.finalize();
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(result.into_bytes()))
}

/// Enforce `exp` / `nbf` against the current clock.
///
/// A token without the claim is accepted: a JWT is not required to carry
/// either, and refusing one would break issuers that deliberately mint
/// non-expiring tokens. What must not happen is *having* an `exp` and
/// ignoring it.
fn check_time_claims(claims: &serde_json::Value, validation: &Validation) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "System clock is before the Unix epoch".to_string())?
        .as_secs();

    if validation.validate_exp {
        if let Some(exp) = claims.get("exp").and_then(numeric_claim) {
            if now > exp.saturating_add(validation.leeway) {
                return Err("JWT has expired".to_string());
            }
        }
    }

    if validation.validate_nbf {
        if let Some(nbf) = claims.get("nbf").and_then(numeric_claim) {
            if now.saturating_add(validation.leeway) < nbf {
                return Err("JWT is not yet valid".to_string());
            }
        }
    }

    Ok(())
}

/// Read a numeric JWT time claim. Accepts the integer form and the float form
/// some issuers emit; anything else is treated as absent rather than as zero,
/// which would make every token look expired.
fn numeric_claim(value: &serde_json::Value) -> Option<u64> {
    if let Some(n) = value.as_u64() {
        return Some(n);
    }
    value.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn round_trip(claims: serde_json::Value) -> Result<TokenData<serde_json::Value>, String> {
        let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(b"k"))?;
        decode::<serde_json::Value>(
            &token,
            &DecodingKey::from_secret(b"k"),
            &Validation::default(),
        )
    }

    #[test]
    fn expired_tokens_are_rejected() {
        let err = round_trip(json!({ "sub": "alice", "exp": now() - 3600 }))
            .expect_err("an expired token must not validate");
        assert!(err.contains("expired"), "unexpected error: {}", err);
    }

    #[test]
    fn future_tokens_are_rejected() {
        let err = round_trip(json!({ "sub": "alice", "nbf": now() + 3600 }))
            .expect_err("a not-yet-valid token must not validate");
        assert!(err.contains("not yet valid"), "unexpected error: {}", err);
    }

    #[test]
    fn live_tokens_are_accepted() {
        let data = round_trip(json!({ "sub": "alice", "exp": now() + 3600 }))
            .expect("a live token must validate");
        assert_eq!(data.claims["sub"], "alice");
    }

    #[test]
    fn tokens_without_time_claims_are_accepted() {
        let data = round_trip(json!({ "sub": "alice" })).expect("no exp is not an error");
        assert_eq!(data.claims["sub"], "alice");
    }

    #[test]
    fn a_bad_signature_still_loses() {
        let token = encode(
            &Header::default(),
            &json!({ "sub": "alice", "exp": now() + 3600 }),
            &EncodingKey::from_secret(b"k"),
        )
        .unwrap();
        assert!(
            decode::<serde_json::Value>(
                &token,
                &DecodingKey::from_secret(b"other"),
                &Validation::default()
            )
            .is_err(),
            "signature check must still run"
        );
    }
}
