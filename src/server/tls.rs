//! Native TLS termination for the API listener.
//!
//! SoliDB speaks plain HTTP on the wire by default; deployments that cannot
//! put a reverse proxy in front of the server can pass `--tls-cert` and
//! `--tls-key` to terminate TLS 1.2/1.3 directly (via rustls, keeping the
//! build OpenSSL-free).
//!
//! On the multiplexed port the listener *sniffs* for a TLS ClientHello and
//! only handshakes when one is offered, so an HTTPS client and a plaintext
//! peer can share the port. That mixed mode is deliberate: none of the
//! shipped SDKs' native driver protocol, nor the sync and cluster
//! transports, speak TLS yet, so unconditional termination would break every
//! driver client and every inter-node connection. Once a connection is
//! decrypted the multiplexer sniffs the plaintext exactly as before, so HTTP
//! *and* the driver protocol work inside the tunnel for clients that offer
//! it. Set `SOLIDB_TLS_REQUIRE=1` to refuse plaintext on that port (only
//! safe on a single node with no native-protocol clients).

use crate::error::{DbError, DbResult};
use std::sync::Arc;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::TlsAcceptor;

/// Load a certificate chain and private key from PEM files and return a
/// ready-to-use TLS acceptor.
///
/// PEM parsing goes through `rustls-pki-types`' `PemObject` API directly:
/// the `rustls-pemfile` crate that used to do this is unmaintained
/// (RUSTSEC-2025-0134) and fails `cargo deny`.
pub fn load_tls_acceptor(cert_path: &str, key_path: &str) -> DbResult<TlsAcceptor> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_path)
        .map_err(|e| {
            DbError::InternalError(format!("Cannot open TLS cert file {}: {}", cert_path, e))
        })?
        .collect::<Result<_, _>>()
        .map_err(|e| {
            DbError::InternalError(format!(
                "Invalid TLS certificate chain in {}: {}",
                cert_path, e
            ))
        })?;

    if certs.is_empty() {
        return Err(DbError::InternalError(format!(
            "No certificates found in {}",
            cert_path
        )));
    }

    let key: PrivateKeyDer<'static> = PrivateKeyDer::from_pem_file(key_path).map_err(|e| {
        DbError::InternalError(format!("Cannot load TLS key from {}: {}", key_path, e))
    })?;

    // Name the provider explicitly. `ServerConfig::builder()` panics when it
    // cannot pick a default from the crate features, and this build enables
    // both (`ring` via tokio-rustls, `aws-lc-rs` via reqwest's rustls stack).
    // Installing a process-wide default instead would also change which
    // provider reqwest picks, so keep the choice local to the listener.
    let config = tokio_rustls::rustls::ServerConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| DbError::InternalError(format!("TLS provider error: {}", e)))?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .map_err(|e| DbError::InternalError(format!("TLS configuration error: {}", e)))?;

    tracing::info!(cert = %cert_path, "Native HTTPS/TLS termination enabled");
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// First byte of a TLS record carrying a handshake message (`ContentType`
/// 22). No plaintext protocol this listener speaks can start with it: HTTP
/// begins with an ASCII method, the driver and sync protocols with their
/// `solidb-` magic, cluster messages with `{`.
pub const TLS_HANDSHAKE_CONTENT_TYPE: u8 = 0x16;

/// Whether plaintext connections must be refused when TLS is configured.
/// Off by default because the native driver protocol and the sync/cluster
/// transports are still plaintext-only.
pub fn tls_required() -> bool {
    matches!(
        std::env::var("SOLIDB_TLS_REQUIRE").as_deref(),
        Ok("1") | Ok("true")
    )
}
