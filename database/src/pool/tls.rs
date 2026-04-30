//! Shared TLS connector construction for postgres and mysql pools.
//!
//! Three operator-visible modes (see `crate::config::TlsMode`):
//!
//!   - `disable`     → caller passes `NoTls`; this module never runs.
//!   - `require`     → cert chain validated, hostname NOT verified
//!     (matches libpq `sslmode=require`).
//!   - `verify-full` → cert chain + hostname verified
//!     (matches libpq `sslmode=verify-full`).
//!
//! Trust roots come from the OS-provided trust store
//! (`rustls-native-certs`) by default. An optional `ca_cert` PEM file
//! **replaces** the system store with the operator-supplied certs — useful
//! for self-hosted databases with private CAs.
//!
//! `aws_lc_rs` is the rustls crypto provider; it's the modern default and
//! lets us avoid an OpenSSL system dep. `tls12` and TLS 1.3 are both
//! enabled because real-world managed Postgres still negotiates 1.2.

use crate::config::{TlsConfig, TlsMode};
use crate::error::DbError;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use std::sync::{Arc, Once};

/// Install `aws_lc_rs` as the process-level default rustls `CryptoProvider`.
///
/// rustls 0.23 requires a process default when the dep graph contains more
/// than one provider feature (we have `aws_lc_rs` direct + `aws-lc-rs`
/// transitively via `tokio-postgres-rustls`). Without this, the first
/// rustls user that doesn't take an explicit provider — notably
/// `mysql_async`'s `rustls-tls` path — **panics** on first TLS attempt.
/// The panic happens inside the spawned connection task, where it
/// invisibly crashes the pool's `get_conn()` future and presents to the
/// caller as a multi-second hang rather than an error.
///
/// Idempotent: `install_default` returns Err on second call; `Once`
/// guarantees we only attempt it once per process.
fn ensure_crypto_provider_installed() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        // Best-effort: if some other crate beat us to it, that's fine —
        // either provider works for our verification path.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

/// Build a `tokio_postgres_rustls::MakeRustlsConnect` for the given TLS
/// config. Returns `Ok(None)` when the operator chose `mode: disable` —
/// callers should fall back to `NoTls` in that case.
pub fn make_pg_connector(
    tls: &TlsConfig,
) -> Result<Option<tokio_postgres_rustls::MakeRustlsConnect>, DbError> {
    if matches!(tls.mode, TlsMode::Disable) {
        return Ok(None);
    }
    ensure_crypto_provider_installed();
    let client_config = build_client_config(tls)?;
    Ok(Some(tokio_postgres_rustls::MakeRustlsConnect::new(
        client_config,
    )))
}

/// Build a `mysql_async::SslOpts` for the given TLS config. Returns
/// `Ok(None)` when `mode: disable` — callers must NOT enable any TLS opts
/// in that case.
pub fn make_mysql_ssl_opts(tls: &TlsConfig) -> Result<Option<mysql_async::SslOpts>, DbError> {
    use mysql_async::SslOpts;
    if matches!(tls.mode, TlsMode::Disable) {
        return Ok(None);
    }
    // Mandatory: mysql_async's rustls-tls feature reaches for the
    // process-default provider. Without this install, the first TLS
    // attempt panics inside a spawned task and presents as a 30s pool
    // hang to the caller. See `ensure_crypto_provider_installed` doc.
    ensure_crypto_provider_installed();
    let mut opts = SslOpts::default();
    if let Some(path) = tls.ca_cert.as_deref() {
        // mysql_async accepts a PEM file path directly.
        opts = opts.with_root_certs(vec![std::path::PathBuf::from(path).into()]);
    }
    // libpq-aligned semantics: require = chain only (skip hostname);
    // verify-full = chain + hostname.
    if matches!(tls.mode, TlsMode::Require) {
        opts = opts
            .with_danger_skip_domain_validation(true)
            .with_danger_accept_invalid_certs(false);
    }
    Ok(Some(opts))
}

/// Construct the `rustls::ClientConfig` matching `tls.mode`. Used by the
/// postgres connector; the mysql side has its own knobs and doesn't share
/// this `ClientConfig`.
fn build_client_config(tls: &TlsConfig) -> Result<ClientConfig, DbError> {
    let roots = build_root_store(tls.ca_cert.as_deref())?;
    let provider = Arc::new(default_provider());

    match tls.mode {
        TlsMode::Disable => {
            // Caller should have short-circuited; defensive panic-avoid path.
            Err(DbError::ConfigError {
                message: "internal: build_client_config called with mode=disable".into(),
            })
        }
        TlsMode::VerifyFull => {
            let verifier =
                WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                    .build()
                    .map_err(|e| DbError::ConfigError {
                        message: format!("tls verifier build failed: {e}"),
                    })?;
            let cfg = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| DbError::ConfigError {
                    message: format!("tls protocol negotiation failed: {e}"),
                })?
                .with_webpki_verifier(verifier)
                .with_no_client_auth();
            Ok(cfg)
        }
        TlsMode::Require => {
            // Chain-only verifier. Validates the certificate chain against
            // the trust store but does NOT verify the cert hostname matches
            // the URL host. Same security posture as libpq's
            // `sslmode=require`: catches eavesdropping, doesn't catch a
            // determined MITM with their own valid-chain cert.
            let verifier = Arc::new(ChainOnlyVerifier {
                roots: Arc::new(roots),
                provider: provider.clone(),
            });
            let cfg = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| DbError::ConfigError {
                    message: format!("tls protocol negotiation failed: {e}"),
                })?
                .dangerous()
                .with_custom_certificate_verifier(verifier)
                .with_no_client_auth();
            Ok(cfg)
        }
    }
}

/// Build a `RootCertStore` from either an operator-supplied PEM file or
/// the OS trust store. The `ca_cert` path **replaces** the native store;
/// it is not additive. This matches the typical operator intent: "trust
/// these certs, nothing else."
pub fn build_root_store(ca_cert: Option<&str>) -> Result<RootCertStore, DbError> {
    let mut store = RootCertStore::empty();
    if let Some(path) = ca_cert {
        let pem = std::fs::read(path).map_err(|e| DbError::ConfigError {
            message: format!("ca_cert read `{path}`: {e}"),
        })?;
        let mut cursor = std::io::Cursor::new(pem);
        let mut added = 0usize;
        for item in rustls_pemfile::certs(&mut cursor) {
            let cert = item.map_err(|e| DbError::ConfigError {
                message: format!("ca_cert parse `{path}`: {e}"),
            })?;
            store
                .add(cert)
                .map_err(|e| DbError::ConfigError {
                    message: format!("ca_cert add `{path}`: {e}"),
                })?;
            added += 1;
        }
        if added == 0 {
            return Err(DbError::ConfigError {
                message: format!("ca_cert `{path}`: no PEM CERTIFICATE blocks found"),
            });
        }
        return Ok(store);
    }
    // Native trust store. `load_native_certs` returns errors as a Vec
    // alongside the certs — non-fatal (one bad cert in the store
    // shouldn't block startup if the rest are usable).
    let result = rustls_native_certs::load_native_certs();
    if result.certs.is_empty() {
        return Err(DbError::ConfigError {
            message: format!(
                "no native CA certificates loaded ({} errors); set `tls.ca_cert` to provide them",
                result.errors.len()
            ),
        });
    }
    for cert in result.certs {
        // Ignore individual `add` failures: bad cert in the OS store
        // shouldn't fail the whole worker if other certs work.
        let _ = store.add(cert);
    }
    Ok(store)
}

fn default_provider() -> CryptoProvider {
    rustls::crypto::aws_lc_rs::default_provider()
}

/// Custom verifier that performs certificate chain validation against the
/// trust store but skips hostname verification. Used for `mode: require`
/// to match libpq's `sslmode=require` semantics.
#[derive(Debug)]
struct ChainOnlyVerifier {
    roots: Arc<RootCertStore>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for ChainOnlyVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let cert = webpki::EndEntityCert::try_from(end_entity)
            .map_err(|e| rustls::Error::General(format!("cert parse: {e}")))?;
        let trust_anchors: Vec<_> = self.roots.roots.to_vec();
        let revocation: Option<webpki::RevocationOptions<'_>> = None;
        cert.verify_for_usage(
            self.provider.signature_verification_algorithms.all,
            &trust_anchors,
            intermediates,
            now,
            webpki::KeyUsage::server_auth(),
            revocation,
            None,
        )
        .map_err(|e| rustls::Error::General(format!("cert chain: {e}")))?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn build_root_store_loads_native_certs() {
        // System trust store should have at least a few CAs on a normal
        // dev machine; if this fails the dev environment is unusual.
        let store = build_root_store(None).expect("native certs");
        assert!(!store.roots.is_empty(), "native trust store is empty");
    }

    #[test]
    fn build_root_store_rejects_missing_file() {
        let err = build_root_store(Some("/no/such/path/ca.pem")).unwrap_err();
        let body = serde_json::to_string(&err).unwrap();
        assert!(body.contains("ca_cert read"), "got: {body}");
    }

    #[test]
    fn build_root_store_rejects_pem_with_no_certs() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        // Write a PEM-shaped block that's not a certificate.
        f.write_all(b"-----BEGIN PRIVATE KEY-----\nMIIBVQ==\n-----END PRIVATE KEY-----\n")
            .unwrap();
        let err = build_root_store(Some(f.path().to_str().unwrap())).unwrap_err();
        let body = serde_json::to_string(&err).unwrap();
        assert!(body.contains("no PEM CERTIFICATE"), "got: {body}");
    }

    #[test]
    fn make_pg_connector_disable_returns_none() {
        let tls = TlsConfig {
            mode: TlsMode::Disable,
            ca_cert: None,
        };
        assert!(make_pg_connector(&tls).unwrap().is_none());
    }

    #[test]
    fn make_pg_connector_require_returns_some() {
        let tls = TlsConfig {
            mode: TlsMode::Require,
            ca_cert: None,
        };
        let conn = make_pg_connector(&tls).expect("require mode builds");
        assert!(conn.is_some());
    }

    #[test]
    fn make_pg_connector_verify_full_returns_some() {
        let tls = TlsConfig {
            mode: TlsMode::VerifyFull,
            ca_cert: None,
        };
        let conn = make_pg_connector(&tls).expect("verify-full mode builds");
        assert!(conn.is_some());
    }

    #[test]
    fn make_mysql_ssl_opts_disable_returns_none() {
        let tls = TlsConfig {
            mode: TlsMode::Disable,
            ca_cert: None,
        };
        assert!(make_mysql_ssl_opts(&tls).unwrap().is_none());
    }

    #[test]
    fn make_mysql_ssl_opts_require_skips_domain_validation() {
        let tls = TlsConfig {
            mode: TlsMode::Require,
            ca_cert: None,
        };
        let opts = make_mysql_ssl_opts(&tls).unwrap().unwrap();
        // SslOpts doesn't expose getters for its danger-flags directly,
        // but Debug output captures the configuration. Use it as a
        // proxy for the test contract: `require` mode disables domain
        // validation, `verify-full` keeps it on.
        let dbg = format!("{opts:?}");
        assert!(
            dbg.contains("skip_domain_validation: true")
                || dbg.contains("DangerSkipDomainValidation: true"),
            "expected domain validation off in require mode; got: {dbg}"
        );
    }
}
