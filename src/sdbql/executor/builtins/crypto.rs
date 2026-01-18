//! Cryptographic and encoding functions for SDBQL.
//!
//! MD5, SHA256, BASE64, ARGON2, HMAC, etc.

use crate::error::{DbError, DbResult};
use serde_json::Value;

/// Evaluate crypto/encoding functions
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "MD5" => {
            check_args(name, args, 1)?;
            let input = get_string(&args[0], name)?;
            let digest = md5::compute(input.as_bytes());
            Ok(Some(Value::String(format!("{:x}", digest))))
        }
        "SHA256" => {
            check_args(name, args, 1)?;
            let input = get_string(&args[0], name)?;
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(input.as_bytes());
            Ok(Some(Value::String(hex::encode(hasher.finalize()))))
        }
        "SHA512" => {
            check_args(name, args, 1)?;
            let input = get_string(&args[0], name)?;
            use sha2::{Digest, Sha512};
            let mut hasher = Sha512::new();
            hasher.update(input.as_bytes());
            Ok(Some(Value::String(hex::encode(hasher.finalize()))))
        }
        "BASE64_ENCODE" | "TO_BASE64" => {
            check_args(name, args, 1)?;
            let input = get_string(&args[0], name)?;
            use base64::{engine::general_purpose, Engine as _};
            Ok(Some(Value::String(general_purpose::STANDARD.encode(input))))
        }
        "BASE64_DECODE" | "FROM_BASE64" => {
            check_args(name, args, 1)?;
            let input = get_string(&args[0], name)?;
            use base64::{engine::general_purpose, Engine as _};
            match general_purpose::STANDARD.decode(input) {
                Ok(bytes) => {
                    let s = String::from_utf8(bytes).map_err(|_| {
                        DbError::ExecutionError(
                            "BASE64_DECODE: result is not valid utf8".to_string(),
                        )
                    })?;
                    Ok(Some(Value::String(s)))
                }
                Err(_) => Err(DbError::ExecutionError(
                    "BASE64_DECODE: invalid base64".to_string(),
                )),
            }
        }
        "HEX_ENCODE" | "TO_HEX" => {
            check_args(name, args, 1)?;
            let input = get_string(&args[0], name)?;
            Ok(Some(Value::String(hex::encode(input.as_bytes()))))
        }
        "HEX_DECODE" | "FROM_HEX" => {
            check_args(name, args, 1)?;
            let input = get_string(&args[0], name)?;
            match hex::decode(input) {
                Ok(bytes) => {
                    let s = String::from_utf8(bytes).map_err(|_| {
                        DbError::ExecutionError("HEX_DECODE: result is not valid utf8".to_string())
                    })?;
                    Ok(Some(Value::String(s)))
                }
                Err(_) => Err(DbError::ExecutionError(
                    "HEX_DECODE: invalid hex string".to_string(),
                )),
            }
        }
        "ARGON2_HASH" => {
            check_args(name, args, 1)?;
            let password = get_string(&args[0], name)?;
            use argon2::{
                password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
                Argon2,
            };
            let salt = SaltString::generate(&mut OsRng);
            let argon2 = Argon2::default();
            match argon2.hash_password(password.as_bytes(), &salt) {
                Ok(hash) => Ok(Some(Value::String(hash.to_string()))),
                Err(e) => Err(DbError::ExecutionError(format!(
                    "ARGON2_HASH: failed to hash: {}",
                    e
                ))),
            }
        }
        "ARGON2_VERIFY" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "ARGON2_VERIFY requires 2 arguments: hash, password".to_string(),
                ));
            }
            let hash = get_string(&args[0], "ARGON2_VERIFY (hash)")?;
            let password = get_string(&args[1], "ARGON2_VERIFY (password)")?;
            use argon2::{
                password_hash::{PasswordHash, PasswordVerifier},
                Argon2,
            };
            let parsed_hash = PasswordHash::new(&hash).map_err(|_| {
                DbError::ExecutionError("ARGON2_VERIFY: invalid hash format".to_string())
            })?;
            let is_valid = Argon2::default()
                .verify_password(password.as_bytes(), &parsed_hash)
                .is_ok();
            Ok(Some(Value::Bool(is_valid)))
        }
        "HMAC_SHA256" => {
            if args.len() != 2 {
                return Err(DbError::ExecutionError(
                    "HMAC_SHA256 requires 2 arguments: key, message".to_string(),
                ));
            }
            let key = get_string(&args[0], "HMAC_SHA256 (key)")?;
            let message = get_string(&args[1], "HMAC_SHA256 (message)")?;
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            type HmacSha256 = Hmac<Sha256>;
            let mut mac = HmacSha256::new_from_slice(key.as_bytes())
                .map_err(|_| DbError::ExecutionError("HMAC_SHA256: invalid key".to_string()))?;
            mac.update(message.as_bytes());
            let result = mac.finalize();
            Ok(Some(Value::String(hex::encode(result.into_bytes()))))
        }
        _ => Ok(None),
    }
}

fn get_string(v: &Value, func_name: &str) -> DbResult<String> {
    v.as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| DbError::ExecutionError(format!("{}: argument must be a string", func_name)))
}

fn check_args(name: &str, args: &[Value], expected: usize) -> DbResult<()> {
    if args.len() != expected {
        return Err(DbError::ExecutionError(format!(
            "{} requires {} argument(s)",
            name, expected
        )));
    }
    Ok(())
}
