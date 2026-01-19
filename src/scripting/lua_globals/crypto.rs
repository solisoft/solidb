//! Crypto namespace functions for Lua

use crate::error::DbError;
use crate::scripting::conversion::{json_to_lua, lua_to_json_value};
use crate::scripting::jwt::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use base64::Engine;
use hmac::Mac;
use mlua::{Lua, Value as LuaValue};
use sha2::Digest;
use x25519_dalek::{PublicKey, StaticSecret};

/// Setup the crypto namespace with all cryptography functions
pub fn setup_crypto_globals(lua: &Lua) -> Result<(), DbError> {
    let globals = lua.globals();

    let crypto = lua
        .create_table()
        .map_err(|e| DbError::InternalError(format!("Failed to create crypto table: {}", e)))?;

    // md5(data)
    let md5_fn = lua
        .create_function(|_, data: mlua::String| {
            let digest = md5::compute(data.as_bytes());
            Ok(format!("{:x}", digest))
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create md5 function: {}", e)))?;
    crypto
        .set("md5", md5_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set md5: {}", e)))?;

    // sha256(data)
    let sha256_fn = lua
        .create_function(|_, data: mlua::String| {
            let mut hasher = sha2::Sha256::new();
            hasher.update(data.as_bytes());
            Ok(hex::encode(hasher.finalize()))
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create sha256 function: {}", e)))?;
    crypto
        .set("sha256", sha256_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set sha256: {}", e)))?;

    // sha512(data)
    let sha512_fn = lua
        .create_function(|_, data: mlua::String| {
            let mut hasher = sha2::Sha512::new();
            hasher.update(data.as_bytes());
            Ok(hex::encode(hasher.finalize()))
        })
        .map_err(|e| DbError::InternalError(format!("Failed to create sha512 function: {}", e)))?;
    crypto
        .set("sha512", sha512_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set sha512: {}", e)))?;

    // hmac_sha256(key, data)
    let hmac_sha256_fn = lua
        .create_function(|_, (key, data): (mlua::String, mlua::String)| {
            type HmacSha256 = hmac::Hmac<sha2::Sha256>;
            let mut mac = HmacSha256::new_from_slice(&key.as_bytes())
                .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
            mac.update(&data.as_bytes());
            Ok(hex::encode(mac.finalize().into_bytes()))
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create hmac_sha256 function: {}", e))
        })?;
    crypto
        .set("hmac_sha256", hmac_sha256_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set hmac_sha256: {}", e)))?;

    // hmac_sha512(key, data)
    let hmac_sha512_fn = lua
        .create_function(|_, (key, data): (mlua::String, mlua::String)| {
            type HmacSha512 = hmac::Hmac<sha2::Sha512>;
            let mut mac = HmacSha512::new_from_slice(&key.as_bytes())
                .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
            mac.update(&data.as_bytes());
            Ok(hex::encode(mac.finalize().into_bytes()))
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create hmac_sha512 function: {}", e))
        })?;
    crypto
        .set("hmac_sha512", hmac_sha512_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set hmac_sha512: {}", e)))?;

    // base64_encode(data)
    let base64_encode_fn = lua
        .create_function(|_, data: mlua::String| {
            Ok(base64::engine::general_purpose::STANDARD.encode(data.as_bytes()))
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create base64_encode function: {}", e))
        })?;
    crypto
        .set("base64_encode", base64_encode_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set base64_encode: {}", e)))?;

    // base64_decode(data)
    let base64_decode_fn = lua
        .create_function(|lua, data: String| {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
            lua.create_string(&bytes)
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create base64_decode function: {}", e))
        })?;
    crypto
        .set("base64_decode", base64_decode_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set base64_decode: {}", e)))?;

    // base32_encode(data)
    let base32_encode_fn = lua
        .create_function(|_, data: mlua::String| {
            let encoded = base32::encode(
                base32::Alphabet::RFC4648 { padding: true },
                &data.as_bytes(),
            );
            Ok(encoded)
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create base32_encode function: {}", e))
        })?;
    crypto
        .set("base32_encode", base32_encode_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set base32_encode: {}", e)))?;

    // base32_decode(data)
    let base32_decode_fn = lua
        .create_function(|lua, data: String| {
            let bytes = base32::decode(base32::Alphabet::RFC4648 { padding: true }, &data)
                .ok_or_else(|| mlua::Error::RuntimeError("Invalid base32".to_string()))?;
            lua.create_string(&bytes)
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create base32_decode function: {}", e))
        })?;
    crypto
        .set("base32_decode", base32_decode_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set base32_decode: {}", e)))?;

    // hex_encode(data)
    let hex_encode_fn = lua
        .create_function(|_, data: String| Ok(hex::encode(data)))
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create hex_encode function: {}", e))
        })?;
    crypto
        .set("hex_encode", hex_encode_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set hex_encode: {}", e)))?;

    // hex_decode(data)
    let hex_decode_fn = lua
        .create_function(|lua, data: String| {
            let bytes = hex::decode(data).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
            lua.create_string(&bytes)
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create hex_decode function: {}", e))
        })?;
    crypto
        .set("hex_decode", hex_decode_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set hex_decode: {}", e)))?;

    // uuid()
    let uuid_fn = lua
        .create_function(|_, ()| Ok(uuid::Uuid::new_v4().to_string()))
        .map_err(|e| DbError::InternalError(format!("Failed to create uuid function: {}", e)))?;
    crypto
        .set("uuid", uuid_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set uuid: {}", e)))?;

    // uuid_v7()
    let uuid_v7_fn = lua
        .create_function(|_, ()| Ok(uuid::Uuid::now_v7().to_string()))
        .map_err(|e| DbError::InternalError(format!("Failed to create uuid_v7 function: {}", e)))?;
    crypto
        .set("uuid_v7", uuid_v7_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set uuid_v7: {}", e)))?;

    // random_bytes(n)
    let random_bytes_fn = lua
        .create_function(|lua, n: usize| {
            use rand::RngCore;
            let mut bytes = vec![0u8; n];
            rand::thread_rng().fill_bytes(&mut bytes);
            lua.create_string(&bytes)
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create random_bytes function: {}", e))
        })?;
    crypto
        .set("random_bytes", random_bytes_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set random_bytes: {}", e)))?;

    // random_string(n)
    let random_string_fn = lua
        .create_function(|_, n: usize| {
            use rand::Rng;
            const CHARSET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
            let mut rng = rand::thread_rng();
            let s: String = (0..n)
                .map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char)
                .collect();
            Ok(s)
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create random_string function: {}", e))
        })?;
    crypto
        .set("random_string", random_string_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set random_string: {}", e)))?;

    // hash_password(password)
    let hash_password_fn = lua
        .create_async_function(|_, password: String| async move {
            tokio::task::spawn_blocking(move || {
                let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
                let argon2 = Argon2::default();
                argon2
                    .hash_password(password.as_bytes(), &salt)
                    .map(|h| h.to_string())
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            })
            .await
            .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create hash_password function: {}", e))
        })?;
    crypto
        .set("hash_password", hash_password_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set hash_password: {}", e)))?;

    // verify_password(hash, password)
    let verify_password_fn = lua
        .create_async_function(|_, (hash, password): (String, String)| async move {
            tokio::task::spawn_blocking(move || {
                let parsed_hash = PasswordHash::new(&hash)
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
                Ok(Argon2::default()
                    .verify_password(password.as_bytes(), &parsed_hash)
                    .is_ok())
            })
            .await
            .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create verify_password function: {}", e))
        })?;
    crypto
        .set("verify_password", verify_password_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set verify_password: {}", e)))?;

    // jwt_encode(claims, secret)
    let jwt_encode_fn = lua
        .create_function(
            move |lua, (claims, secret): (LuaValue, String)| -> Result<String, mlua::Error> {
                let json_claims = lua_to_json_value(lua, claims)?;
                let token = encode(
                    &Header::default(),
                    &json_claims,
                    &EncodingKey::from_secret(secret.as_bytes()),
                )
                .map_err(|e| mlua::Error::RuntimeError(format!("JWT encode error: {}", e)))?;
                Ok(token)
            },
        )
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create jwt_encode function: {}", e))
        })?;
    crypto
        .set("jwt_encode", jwt_encode_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set jwt_encode: {}", e)))?;

    // jwt_decode(token, secret)
    let jwt_decode_fn = lua
        .create_function(
            move |lua, (token, secret): (String, String)| -> Result<mlua::Value, mlua::Error> {
                let token_data = decode::<serde_json::Value>(
                    &token,
                    &DecodingKey::from_secret(secret.as_bytes()),
                    &Validation::default(),
                )
                .map_err(|e| mlua::Error::RuntimeError(format!("JWT decode error: {}", e)))?;
                json_to_lua(lua, &token_data.claims)
            },
        )
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create jwt_decode function: {}", e))
        })?;
    crypto
        .set("jwt_decode", jwt_decode_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set jwt_decode: {}", e)))?;

    // curve25519(secret, peer_public) - X25519 key agreement
    // If peer_public is empty, returns the public key for the given secret
    // Otherwise, performs ECDH and returns the shared secret
    let curve25519_fn = lua
        .create_function(|lua, (secret, peer_public): (mlua::String, mlua::String)| {
            let secret_bytes = secret.as_bytes();
            if secret_bytes.len() != 32 {
                return Err(mlua::Error::RuntimeError(
                    "Secret key must be exactly 32 bytes".to_string(),
                ));
            }

            let mut secret_arr = [0u8; 32];
            secret_arr.copy_from_slice(&secret_bytes);
            let static_secret = StaticSecret::from(secret_arr);

            let peer_bytes = peer_public.as_bytes();
            if peer_bytes.is_empty() {
                // Generate public key from secret
                let public = PublicKey::from(&static_secret);
                lua.create_string(public.as_bytes())
            } else {
                // Perform key exchange
                if peer_bytes.len() != 32 {
                    return Err(mlua::Error::RuntimeError(
                        "Peer public key must be exactly 32 bytes".to_string(),
                    ));
                }
                let mut peer_arr = [0u8; 32];
                peer_arr.copy_from_slice(&peer_bytes);
                let peer_public_key = PublicKey::from(peer_arr);
                let shared = static_secret.diffie_hellman(&peer_public_key);
                lua.create_string(shared.as_bytes())
            }
        })
        .map_err(|e| {
            DbError::InternalError(format!("Failed to create curve25519 function: {}", e))
        })?;
    crypto
        .set("curve25519", curve25519_fn)
        .map_err(|e| DbError::InternalError(format!("Failed to set curve25519: {}", e)))?;

    globals
        .set("crypto", crypto)
        .map_err(|e| DbError::InternalError(format!("Failed to set crypto global: {}", e)))?;

    Ok(())
}
