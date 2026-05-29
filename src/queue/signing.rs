//! HMAC-SHA256 helpers for signing webhook payloads.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Returns the hex-encoded HMAC-SHA256 of `body` keyed with `secret`.
/// The output is lowercase hex (64 chars) — matches the format the Soli-app
/// `/_jobs/run/:name` handler verifies via constant-time compare.
pub fn sign(body: &[u8], secret: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(body);
    let result = mac.finalize().into_bytes();
    let mut out = String::with_capacity(result.len() * 2);
    for b in result.iter() {
        use std::fmt::Write;
        let _ = write!(out, "{:02x}", b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_known_vector() {
        // RFC 4231 test case 1.
        let key = "Jefe";
        let data = b"what do ya want for nothing?";
        let got = sign(data, key);
        assert_eq!(
            got,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn sign_is_stable() {
        let a = sign(b"hello", "k");
        let b = sign(b"hello", "k");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn sign_differs_by_key() {
        assert_ne!(sign(b"x", "a"), sign(b"x", "b"));
    }
}
