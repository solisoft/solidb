//! Cryptographic and encoding functions for SDBQL.
//!
//! MD5, SHA1, SHA256, CRC32, FNV64, BASE64, ARGON2, HMAC, etc.
//!
//! As in AQL, a null input gives null and any other non-string input is
//! hashed or encoded in its string form (numbers, booleans; arrays and
//! objects as JSON).

use super::string::{fnv1a64, text_of};
use crate::error::{DbError, DbResult};
use parking_lot::{Condvar, Mutex};
use serde_json::Value;
use std::borrow::Cow;
use std::cell::Cell;
use std::time::{Duration, Instant};

/// Argon2 parameters `ARGON2_VERIFY` accepts from a stored hash. The hash
/// string is caller-supplied, so without a policy `m=4194304,t=4294967295`
/// is a 4 GiB allocation and a near-endless loop per row.
const ARGON2_MAX_M_COST_KIB: u32 = 64 * 1024;
const ARGON2_MAX_T_COST: u32 = 10;
const ARGON2_MAX_P_COST: u32 = 4;

/// Argon2 calls a single query may make (see [`argon2_budget_take`]).
const ARGON2_MAX_CALLS_PER_QUERY: u32 = 64;
/// When no query boundary has been signalled, the per-thread budget refills
/// after this long, so it bounds one runaway query without failing a thread
/// that serves many small queries.
const ARGON2_BUDGET_WINDOW: Duration = Duration::from_secs(10);
/// How long an Argon2 call waits for a free slot before failing.
const ARGON2_SLOT_WAIT: Duration = Duration::from_secs(5);

thread_local! {
    /// (calls made, when the first of them was made).
    static ARGON2_BUDGET: Cell<(u32, Option<Instant>)> = const { Cell::new((0, None)) };
}

/// Reset this thread's Argon2 budget. The executor calls this when a query
/// starts: a query runs on one thread from start to finish, so a
/// thread-local count is then exactly a per-query count.
// Until the executor calls this at query start, only the time-window
// fallback in `argon2_budget_take` applies.
pub(crate) fn reset_query_budget() {
    ARGON2_BUDGET.with(|b| b.set((0, None)));
}

/// Take one unit of the per-query Argon2 budget.
///
/// A builtin sees only its arguments, not the query it runs in, so the count
/// is kept per thread and reset by [`reset_query_budget`] at query start. As
/// a fallback for call paths that do not reset it, the count also restarts
/// [`ARGON2_BUDGET_WINDOW`] after its first call: a query cannot spend more
/// than [`ARGON2_MAX_CALLS_PER_QUERY`] calls inside one window, which is what
/// stops `FOR i IN 1..100000 RETURN ARGON2_HASH(...)` within a second or two.
fn argon2_budget_take(name: &str) -> DbResult<()> {
    ARGON2_BUDGET.with(|b| {
        let now = Instant::now();
        let (used, since) = b.get();
        let (used, since) = match since {
            Some(t) if now.duration_since(t) < ARGON2_BUDGET_WINDOW => (used, t),
            _ => (0, now),
        };
        if used >= ARGON2_MAX_CALLS_PER_QUERY {
            return Err(DbError::ExecutionError(format!(
                "{}: at most {} Argon2 calls per query",
                name, ARGON2_MAX_CALLS_PER_QUERY
            )));
        }
        b.set((used + 1, Some(since)));
        Ok(())
    })
}

/// Process-wide cap on concurrent SDBQL Argon2 work: each run holds its
/// memory cost (19 MiB by default, up to 64 MiB under the verify policy) for
/// tens of milliseconds, so unbounded fan-out across the blocking pool is
/// gigabytes of RSS. Mirrors `ARGON2_PERMITS` in `server/auth.rs`, but
/// blocking, because builtins run synchronously.
struct Argon2Slots {
    in_use: Mutex<usize>,
    freed: Condvar,
    max: usize,
}

struct Argon2Slot<'a>(&'a Argon2Slots);

impl Drop for Argon2Slot<'_> {
    fn drop(&mut self) {
        *self.0.in_use.lock() -= 1;
        self.0.freed.notify_one();
    }
}

static ARGON2_SLOTS: once_cell::sync::Lazy<Argon2Slots> =
    once_cell::sync::Lazy::new(|| Argon2Slots {
        in_use: Mutex::new(0),
        freed: Condvar::new(),
        max: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4),
    });

fn argon2_slot(name: &str) -> DbResult<Argon2Slot<'static>> {
    let slots = &*ARGON2_SLOTS;
    let deadline = Instant::now() + ARGON2_SLOT_WAIT;
    let mut in_use = slots.in_use.lock();
    while *in_use >= slots.max {
        if slots.freed.wait_until(&mut in_use, deadline).timed_out() && *in_use >= slots.max {
            return Err(DbError::ExecutionError(format!(
                "{}: too many concurrent Argon2 operations, try again",
                name
            )));
        }
    }
    *in_use += 1;
    Ok(Argon2Slot(slots))
}

/// Budget and concurrency admission for one Argon2 call.
fn argon2_admit(name: &str) -> DbResult<Argon2Slot<'static>> {
    argon2_budget_take(name)?;
    argon2_slot(name)
}

/// Uppercase hex without leading zeros, as AQL prints `CRC32` / `FNV64`.
fn hex_upper(n: u64) -> String {
    format!("{:X}", n)
}

/// CRC-32C (Castagnoli, reflected polynomial 0x82F63B78, init and final XOR
/// 0xFFFFFFFF) — the variant AQL's `CRC32` uses.
fn crc32c(bytes: &[u8]) -> u32 {
    static TABLE: once_cell::sync::Lazy<[u32; 256]> = once_cell::sync::Lazy::new(|| {
        let mut table = [0u32; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    (c >> 1) ^ 0x82F6_3B78
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        table
    });
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc = TABLE[((crc ^ u32::from(b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// SHA-1 (FIPS 180-4). Not a security primitive here — `SHA1()` exists for
/// AQL compatibility and content addressing. There is no direct `sha1`
/// dependency, so it is implemented inline.
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = Vec::with_capacity(data.len() + 72);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    let mut w = [0u32; 80];
    for block in msg.as_chunks::<64>().0 {
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (hv, v) in h.iter_mut().zip([a, b, c, d, e]) {
            *hv = hv.wrapping_add(v);
        }
    }
    let mut out = [0u8; 20];
    for (chunk, v) in out.as_chunks_mut::<4>().0.iter_mut().zip(h) {
        chunk.copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// Evaluate crypto/encoding functions
pub fn evaluate(name: &str, args: &[Value]) -> DbResult<Option<Value>> {
    match name {
        "MD5" => {
            check_args(name, args, 1)?;
            let Some(input) = input_text(&args[0]) else {
                return Ok(Some(Value::Null));
            };
            let digest = md5::compute(input.as_bytes());
            Ok(Some(Value::String(format!("{:x}", digest))))
        }
        "SHA1" => {
            check_args(name, args, 1)?;
            let Some(input) = input_text(&args[0]) else {
                return Ok(Some(Value::Null));
            };
            Ok(Some(Value::String(hex::encode(sha1(input.as_bytes())))))
        }
        "SHA256" => {
            check_args(name, args, 1)?;
            let Some(input) = input_text(&args[0]) else {
                return Ok(Some(Value::Null));
            };
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(input.as_bytes());
            Ok(Some(Value::String(hex::encode(hasher.finalize()))))
        }
        "SHA512" => {
            check_args(name, args, 1)?;
            let Some(input) = input_text(&args[0]) else {
                return Ok(Some(Value::Null));
            };
            use sha2::{Digest, Sha512};
            let mut hasher = Sha512::new();
            hasher.update(input.as_bytes());
            Ok(Some(Value::String(hex::encode(hasher.finalize()))))
        }
        "CRC32" => {
            check_args(name, args, 1)?;
            let Some(input) = input_text(&args[0]) else {
                return Ok(Some(Value::Null));
            };
            Ok(Some(Value::String(hex_upper(u64::from(crc32c(
                input.as_bytes(),
            ))))))
        }
        "FNV64" => {
            check_args(name, args, 1)?;
            let Some(input) = input_text(&args[0]) else {
                return Ok(Some(Value::Null));
            };
            Ok(Some(Value::String(hex_upper(fnv1a64(input.as_bytes())))))
        }
        "BASE64_ENCODE" | "TO_BASE64" => {
            check_args(name, args, 1)?;
            let Some(input) = input_text(&args[0]) else {
                return Ok(Some(Value::Null));
            };
            use base64::{engine::general_purpose, Engine as _};
            Ok(Some(Value::String(
                general_purpose::STANDARD.encode(input.as_bytes()),
            )))
        }
        "BASE64_DECODE" | "FROM_BASE64" => {
            check_args(name, args, 1)?;
            let Some(input) = input_text(&args[0]) else {
                return Ok(Some(Value::Null));
            };
            use base64::{engine::general_purpose, Engine as _};
            match general_purpose::STANDARD.decode(input.as_bytes()) {
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
            let Some(input) = input_text(&args[0]) else {
                return Ok(Some(Value::Null));
            };
            Ok(Some(Value::String(hex::encode(input.as_bytes()))))
        }
        "HEX_DECODE" | "FROM_HEX" => {
            check_args(name, args, 1)?;
            let Some(input) = input_text(&args[0]) else {
                return Ok(Some(Value::Null));
            };
            match hex::decode(input.as_bytes()) {
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
            let Some(password) = input_text(&args[0]) else {
                return Ok(Some(Value::Null));
            };
            use argon2::{
                password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
                Argon2,
            };
            let _slot = argon2_admit(name)?;
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
            let (Some(hash), Some(password)) = (input_text(&args[0]), input_text(&args[1])) else {
                return Ok(Some(Value::Null));
            };
            use argon2::{
                password_hash::{PasswordHash, PasswordVerifier},
                Argon2, Params,
            };
            let parsed_hash = PasswordHash::new(&hash).map_err(|_| {
                DbError::ExecutionError("ARGON2_VERIFY: invalid hash format".to_string())
            })?;
            let params = Params::try_from(&parsed_hash).map_err(|_| {
                DbError::ExecutionError("ARGON2_VERIFY: invalid hash parameters".to_string())
            })?;
            if params.m_cost() > ARGON2_MAX_M_COST_KIB
                || params.t_cost() > ARGON2_MAX_T_COST
                || params.p_cost() > ARGON2_MAX_P_COST
            {
                return Err(DbError::ExecutionError(format!(
                    "ARGON2_VERIFY: hash parameters exceed the policy \
                     (m <= {} KiB, t <= {}, p <= {})",
                    ARGON2_MAX_M_COST_KIB, ARGON2_MAX_T_COST, ARGON2_MAX_P_COST
                )));
            }
            let _slot = argon2_admit(name)?;
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
            let (Some(key), Some(message)) = (input_text(&args[0]), input_text(&args[1])) else {
                return Ok(Some(Value::Null));
            };
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

/// `None` for null (the function returns null); otherwise the string form.
fn input_text(v: &Value) -> Option<Cow<'_, str>> {
    if v.is_null() {
        None
    } else {
        Some(text_of(v))
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str, args: &[Value]) -> Value {
        evaluate(name, args).unwrap().unwrap()
    }

    #[test]
    fn sha1_known_vectors() {
        assert_eq!(
            call("SHA1", &[json!("foobar")]),
            json!("8843d7f92416211de9ebb963ff4ce28125932878")
        );
        assert_eq!(
            call("SHA1", &[json!("")]),
            json!("da39a3ee5e6b4b0d3255bfef95601890afd80709")
        );
        // Multi-block input.
        assert_eq!(
            hex::encode(sha1("a".repeat(1000).as_bytes())),
            "291e9a6c66994949b57ba5e650361e98fc36b1ba"
        );
    }

    #[test]
    fn crc32_and_fnv64_match_aql() {
        assert_eq!(call("CRC32", &[json!("foobar")]), json!("D5F5C7F"));
        assert_eq!(call("CRC32", &[json!("")]), json!("0"));
        assert_eq!(call("FNV64", &[json!("foobar")]), json!("85944171F73967E8"));
        assert_eq!(call("FNV64", &[json!("")]), json!("CBF29CE484222325"));
    }

    #[test]
    fn null_in_null_out_and_numbers_stringified() {
        for f in [
            "MD5",
            "SHA1",
            "SHA256",
            "SHA512",
            "CRC32",
            "FNV64",
            "BASE64_ENCODE",
            "HEX_ENCODE",
        ] {
            assert_eq!(call(f, &[Value::Null]), Value::Null, "{}", f);
        }
        assert_eq!(call("MD5", &[json!(1)]), call("MD5", &[json!("1")]));
        assert_eq!(call("MD5", &[json!(2.0)]), call("MD5", &[json!("2")]));
        assert_eq!(
            call("SHA256", &[json!(true)]),
            call("SHA256", &[json!("true")])
        );
        assert_eq!(call("HMAC_SHA256", &[Value::Null, json!("m")]), Value::Null);
        assert_eq!(call("ARGON2_HASH", &[Value::Null]), Value::Null);
        assert_eq!(
            call("ARGON2_VERIFY", &[Value::Null, json!("pw")]),
            Value::Null
        );
    }

    #[test]
    fn argon2_verify_rejects_expensive_params() {
        // m = 4 GiB, t = u32::MAX: rejected before any hashing happens.
        let hostile = "$argon2id$v=19$m=4194304,t=4294967295,p=1$c29tZXNhbHQ$\
                       iWh06vD8Fy27wf9npn6FXWiCX4K6pW6Ue1Bnzz07Z8A";
        assert!(evaluate("ARGON2_VERIFY", &[json!(hostile), json!("pw")]).is_err());
        // 128 MiB: well-formed, but over the 64 MiB policy.
        let big_m = "$argon2id$v=19$m=131072,t=2,p=1$c29tZXNhbHQ$\
                     iWh06vD8Fy27wf9npn6FXWiCX4K6pW6Ue1Bnzz07Z8A";
        let err = evaluate("ARGON2_VERIFY", &[json!(big_m), json!("pw")]).unwrap_err();
        assert!(err.to_string().contains("policy"), "{}", err);
        let too_parallel = "$argon2id$v=19$m=19456,t=2,p=16$c29tZXNhbHQ$\
                            iWh06vD8Fy27wf9npn6FXWiCX4K6pW6Ue1Bnzz07Z8A";
        assert!(evaluate("ARGON2_VERIFY", &[json!(too_parallel), json!("pw")]).is_err());
    }

    #[test]
    fn argon2_roundtrip_within_policy() {
        reset_query_budget();
        let hash = call("ARGON2_HASH", &[json!("secret")]);
        assert_eq!(
            call("ARGON2_VERIFY", &[hash.clone(), json!("secret")]),
            json!(true)
        );
        assert_eq!(call("ARGON2_VERIFY", &[hash, json!("nope")]), json!(false));
    }

    #[test]
    fn argon2_budget_is_enforced_and_resettable() {
        reset_query_budget();
        for _ in 0..ARGON2_MAX_CALLS_PER_QUERY {
            argon2_budget_take("ARGON2_HASH").unwrap();
        }
        assert!(argon2_budget_take("ARGON2_HASH").is_err());
        reset_query_budget();
        assert!(argon2_budget_take("ARGON2_HASH").is_ok());
        reset_query_budget();
    }
}
