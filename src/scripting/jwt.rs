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

#[derive(Debug)]
pub(crate) struct Validation;

impl Validation {
    pub fn default() -> Self {
        Self
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
    _validation: &Validation,
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

    if !header_str.contains(r#""alg":"HS256""#) || !header_str.contains(r#""typ":"JWT""#) {
        return Err("Unsupported JWT algorithm or type".to_string());
    }

    // Verify signature
    let signing_input = format!("{}.{}", header_b64, claims_b64);
    let expected_signature = sign_hmac_sha256(&signing_input, &key.0)?;

    if expected_signature != signature_b64 {
        return Err("Invalid JWT signature".to_string());
    }

    // Decode claims
    let claims_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(claims_b64)
        .map_err(|_| "Invalid JWT claims".to_string())?;

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
