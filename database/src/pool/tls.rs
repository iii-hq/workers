//! Shared TLS connector construction for postgres and mysql pools.
//!
//! Five operator-visible modes (see `crate::config::TlsMode`):
//!
//!   - `disable`     → caller passes `NoTls`; this module never runs.
//!   - `prefer`      → TLS when the server offers it, chain validated like
//!     `require`; plaintext only when the server offers no TLS at all.
//!   - `require` / `verify-ca` → cert chain validated, hostname NOT verified
//!     (matches libpq `sslmode=require`; libpq's `verify-ca` is the same
//!     check).
//!   - `verify-full` → cert chain + hostname verified
//!     (matches libpq `sslmode=verify-full`).
//!
//! Trust roots come from the OS-provided trust store
//! (`rustls-native-certs`) by default. An optional `ca_cert` **extends**
//! it (or replaces it with `trust_native: false`). `client_cert` +
//! `client_key` add mutual TLS. Every certificate/key arrives here already
//! read ([`ResolvedTls`]): from a path, inline PEM, or a resolved
//! `secret://` / `env://` reference.
//!
//! `aws_lc_rs` is the rustls crypto provider; it's the modern default and
//! lets us avoid an OpenSSL system dep. `tls12` and TLS 1.3 are both
//! enabled because real-world managed Postgres still negotiates 1.2.

use crate::config::TlsMode;
use crate::error::DbError;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};
use std::sync::{Arc, Once};

/// TLS settings with every certificate and key already loaded.
#[derive(Clone, Default)]
pub struct ResolvedTls {
    pub mode: TlsMode,
    pub trust_native: bool,
    /// PEM bundle of extra trust anchors (`tls.ca_cert`).
    pub ca_pem: Option<Vec<u8>>,
    /// PEM client certificate chain (`tls.client_cert`).
    pub client_cert_pem: Option<Vec<u8>>,
    /// PEM client private key (`tls.client_key`). Wiped on drop.
    pub client_key_pem: Option<crate::secrets::SecretValue>,
}

impl std::fmt::Debug for ResolvedTls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedTls")
            .field("mode", &self.mode)
            .field("trust_native", &self.trust_native)
            .field("ca_pem", &self.ca_pem.is_some())
            .field("client_cert_pem", &self.client_cert_pem.is_some())
            .field("client_key_pem", &self.client_key_pem.is_some())
            .finish()
    }
}

impl ResolvedTls {
    /// A mode with the default trust posture and no extra material.
    pub fn mode(mode: TlsMode) -> Self {
        Self {
            mode,
            trust_native: true,
            ..Default::default()
        }
    }

    fn client_identity(
        &self,
    ) -> Result<Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>, DbError> {
        let (Some(cert_pem), Some(key_pem)) = (&self.client_cert_pem, &self.client_key_pem) else {
            return Ok(None);
        };
        let mut cursor = std::io::Cursor::new(cert_pem.as_slice());
        let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cursor)
            .collect::<Result<_, _>>()
            .map_err(|e| DbError::ConfigError {
                message: format!("tls.client_cert: not a PEM certificate: {e}"),
            })?;
        if certs.is_empty() {
            return Err(DbError::ConfigError {
                message: "tls.client_cert: no PEM CERTIFICATE blocks found".into(),
            });
        }
        let mut cursor = std::io::Cursor::new(key_pem.expose().as_bytes());
        let key = rustls_pemfile::private_key(&mut cursor)
            .map_err(|_| DbError::ConfigError {
                message: "tls.client_key: unreadable PEM private key".into(),
            })?
            .ok_or_else(|| DbError::ConfigError {
                message: "tls.client_key: no PEM private key found (expected PKCS#8, PKCS#1 or \
                          SEC1)"
                    .into(),
            })?;
        Ok(Some((certs, key)))
    }
}

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
    tls: &ResolvedTls,
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
/// in that case. `hostname` overrides the name the server certificate is
/// checked against (an SSH tunnel connects to 127.0.0.1 but the certificate
/// names the database host).
pub fn make_mysql_ssl_opts(
    tls: &ResolvedTls,
    hostname: Option<&str>,
) -> Result<Option<mysql_async::SslOpts>, DbError> {
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
    if let Some(pem) = &tls.ca_pem {
        // `with_root_certs` adds to mysql_async's bundled webpki roots;
        // there is no public knob to suppress them on mysql_async 0.34's
        // rustls path, but the server cert must still validate against a
        // trusted root.
        opts = opts.with_root_certs(vec![pem.clone().into()]);
    }
    if let (Some(cert), Some(key)) = (&tls.client_cert_pem, &tls.client_key_pem) {
        // mysql_async reads only PKCS#1 RSA keys from PEM; hand it the DER
        // instead, which it takes as-is.
        let der = mysql_client_key_der(key.expose())?;
        opts = opts.with_client_identity(Some(mysql_async::ClientIdentity::new(
            cert.clone().into(),
            der.into(),
        )));
    }
    // libpq-aligned semantics: require/verify-ca/prefer = chain only (skip
    // hostname); verify-full = chain + hostname.
    if !matches!(tls.mode, TlsMode::VerifyFull) {
        opts = opts
            .with_danger_skip_domain_validation(true)
            .with_danger_accept_invalid_certs(false);
    }
    if let Some(host) = hostname {
        opts = opts.with_danger_tls_hostname_override(Some(host.to_string()));
    }
    Ok(Some(opts))
}

/// The PKCS#1 DER of an RSA client key given as PEM (PKCS#1 or PKCS#8).
/// mysql_async's rustls client identity is RSA-only.
fn mysql_client_key_der(pem: &str) -> Result<Vec<u8>, DbError> {
    let unsupported = || DbError::ConfigError {
        message: "tls.client_key: mysql mutual TLS needs an RSA key (PKCS#1 or PKCS#8 PEM); \
                  mysql_async cannot present EC or Ed25519 client keys"
            .into(),
    };
    let mut cursor = std::io::Cursor::new(pem.as_bytes());
    match rustls_pemfile::private_key(&mut cursor) {
        Ok(Some(PrivateKeyDer::Pkcs1(k))) => Ok(k.secret_pkcs1_der().to_vec()),
        Ok(Some(PrivateKeyDer::Pkcs8(k))) => {
            pkcs8_rsa_inner(k.secret_pkcs8_der()).ok_or_else(unsupported)
        }
        Ok(Some(_)) => Err(unsupported()),
        _ => Err(DbError::ConfigError {
            message: "tls.client_key: no PEM private key found".into(),
        }),
    }
}

/// `PrivateKeyInfo ::= SEQUENCE { version, AlgorithmIdentifier, OCTET STRING }`:
/// the OCTET STRING when the algorithm is rsaEncryption, else `None`.
fn pkcs8_rsa_inner(der: &[u8]) -> Option<Vec<u8>> {
    const RSA_OID: &[u8] = &[
        0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01,
    ];
    fn tlv(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
        let (&tag, rest) = input.split_first()?;
        let (&first, rest) = rest.split_first()?;
        let (len, rest) = if first < 0x80 {
            (first as usize, rest)
        } else {
            let n = (first & 0x7f) as usize;
            if n == 0 || n > 4 || rest.len() < n {
                return None;
            }
            let len = rest[..n]
                .iter()
                .fold(0usize, |acc, b| (acc << 8) | *b as usize);
            (len, &rest[n..])
        };
        if rest.len() < len {
            return None;
        }
        Some((tag, &rest[..len], &rest[len..]))
    }
    let (0x30, body, _) = tlv(der)? else {
        return None;
    };
    let (0x02, _version, rest) = tlv(body)? else {
        return None;
    };
    let (0x30, algorithm, rest) = tlv(rest)? else {
        return None;
    };
    if !algorithm.starts_with(RSA_OID) {
        return None;
    }
    let (0x04, key, _) = tlv(rest)? else {
        return None;
    };
    Some(key.to_vec())
}

/// Construct the `rustls::ClientConfig` matching `tls.mode`. Used by the
/// postgres connector; the mysql side has its own knobs and doesn't share
/// this `ClientConfig`.
fn build_client_config(tls: &ResolvedTls) -> Result<ClientConfig, DbError> {
    let roots = build_root_store(tls.ca_pem.as_deref(), tls.trust_native)?;
    let provider = Arc::new(default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| DbError::ConfigError {
            message: format!("tls protocol negotiation failed: {e}"),
        })?;

    let builder = match tls.mode {
        TlsMode::Disable => {
            // Caller should have short-circuited; defensive panic-avoid path.
            return Err(DbError::ConfigError {
                message: "internal: build_client_config called with mode=disable".into(),
            });
        }
        TlsMode::VerifyFull => {
            let verifier =
                WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
                    .build()
                    .map_err(|e| DbError::ConfigError {
                        message: format!("tls verifier build failed: {e}"),
                    })?;
            builder.with_webpki_verifier(verifier)
        }
        TlsMode::Prefer | TlsMode::Require | TlsMode::VerifyCa => {
            // Chain-only verifier. Validates the certificate chain against
            // the trust store but does NOT verify the cert hostname matches
            // the server host. Same security posture as libpq's
            // `sslmode=require`: catches eavesdropping, doesn't catch a
            // determined MITM with their own valid-chain cert.
            let verifier = Arc::new(ChainOnlyVerifier {
                roots: Arc::new(roots),
                provider: provider.clone(),
            });
            builder
                .dangerous()
                .with_custom_certificate_verifier(verifier)
        }
    };
    match tls.client_identity()? {
        Some((certs, key)) => {
            builder
                .with_client_auth_cert(certs, key)
                .map_err(|e| DbError::ConfigError {
                    message: format!(
                        "tls.client_cert/client_key: the key does not fit the certificate or is \
                         unsupported: {e}"
                    ),
                })
        }
        None => Ok(builder.with_no_client_auth()),
    }
}

/// Build a `RootCertStore` from the OS trust store and/or an operator-
/// supplied PEM bundle.
///
/// Behavior:
///   - `trust_native = true`  (default): load `rustls_native_certs` and,
///     if `ca_pem` is set, extend with its certs. This is the recommended
///     posture: managed providers whose certs already chain to a public
///     root keep working, and operators only need to point `ca_cert` at
///     a private CA when they have one (e.g. Supabase Intermediate 2021).
///   - `trust_native = false`: trust only the certs in `ca_pem`. Used
///     for strict-isolation deployments that explicitly do not want the
///     public web PKI accepted. With both `trust_native = false` *and*
///     no `ca_pem`, the result is a config error (no trust anchors).
///
/// Per-cert `RootCertStore::add` failures are non-fatal for the native
/// store (a bad cert in the OS store shouldn't block the whole worker if
/// the rest are usable) but are fatal for `ca_cert` (operator-supplied
/// certs are intentional and we want loud failures).
pub fn build_root_store(
    ca_pem: Option<&[u8]>,
    trust_native: bool,
) -> Result<RootCertStore, DbError> {
    let mut store = RootCertStore::empty();

    if trust_native {
        let result = rustls_native_certs::load_native_certs();
        if result.certs.is_empty() && ca_pem.is_none() {
            return Err(DbError::ConfigError {
                message: format!(
                    "no native CA certificates loaded ({} errors); \
                     set `tls.ca_cert` to provide them",
                    result.errors.len()
                ),
            });
        }
        for cert in result.certs {
            let _ = store.add(cert);
        }
    }

    if let Some(pem) = ca_pem {
        let mut cursor = std::io::Cursor::new(pem);
        let mut added = 0usize;
        for item in rustls_pemfile::certs(&mut cursor) {
            let cert = item.map_err(|e| DbError::ConfigError {
                message: format!("tls.ca_cert: unreadable PEM: {e}"),
            })?;
            store.add(cert).map_err(|e| DbError::ConfigError {
                message: format!("tls.ca_cert: certificate rejected: {e}"),
            })?;
            added += 1;
        }
        if added == 0 {
            return Err(DbError::ConfigError {
                message: "tls.ca_cert: no PEM CERTIFICATE blocks found".into(),
            });
        }
    }

    if !trust_native && ca_pem.is_none() {
        return Err(DbError::ConfigError {
            message: "tls.trust_native is false and tls.ca_cert is unset; \
                      no trust anchors available"
                .into(),
        });
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

    /// ISRG Root X1 (Let's Encrypt). Real, publicly-known, stable trust
    /// anchor that doesn't expire until 2035-06-04. We use it as a
    /// fixture so the additive-vs-replace tests have a concrete cert to
    /// load through `rustls_pemfile::certs` and `RootCertStore::add`
    /// without needing a cert-gen dev-dep.
    const FIXTURE_CA_PEM: &[u8] = b"\
-----BEGIN CERTIFICATE-----\n\
MIIFazCCA1OgAwIBAgIRAIIQz7DSQONZRGPgu2OCiwAwDQYJKoZIhvcNAQELBQAw\n\
TzELMAkGA1UEBhMCVVMxKTAnBgNVBAoTIEludGVybmV0IFNlY3VyaXR5IFJlc2Vh\n\
cmNoIEdyb3VwMRUwEwYDVQQDEwxJU1JHIFJvb3QgWDEwHhcNMTUwNjA0MTEwNDM4\n\
WhcNMzUwNjA0MTEwNDM4WjBPMQswCQYDVQQGEwJVUzEpMCcGA1UEChMgSW50ZXJu\n\
ZXQgU2VjdXJpdHkgUmVzZWFyY2ggR3JvdXAxFTATBgNVBAMTDElTUkcgUm9vdCBY\n\
MTCCAiIwDQYJKoZIhvcNAQEBBQADggIPADCCAgoCggIBAK3oJHP0FDfzm54rVygc\n\
h77ct984kIxuPOZXoHj3dcKi/vVqbvYATyjb3miGbESTtrFj/RQSa78f0uoxmyF+\n\
0TM8ukj13Xnfs7j/EvEhmkvBioZxaUpmZmyPfjxwv60pIgbz5MDmgK7iS4+3mX6U\n\
A5/TR5d8mUgjU+g4rk8Kb4Mu0UlXjIB0ttov0DiNewNwIRt18jA8+o+u3dpjq+sW\n\
T8KOEUt+zwvo/7V3LvSye0rgTBIlDHCNAymg4VMk7BPZ7hm/ELNKjD+Jo2FR3qyH\n\
B5T0Y3HsLuJvW5iB4YlcNHlsdu87kGJ55tukmi8mxdAQ4Q7e2RCOFvu396j3x+UC\n\
B5iPNgiV5+I3lg02dZ77DnKxHZu8A/lJBdiB3QW0KtZB6awBdpUKD9jf1b0SHzUv\n\
KBds0pjBqAlkd25HN7rOrFleaJ1/ctaJxQZBKT5ZPt0m9STJEadao0xAH0ahmbWn\n\
OlFuhjuefXKnEgV4We0+UXgVCwOPjdAvBbI+e0ocS3MFEvzG6uBQE3xDk3SzynTn\n\
jh8BCNAw1FtxNrQHusEwMFxIt4I7mKZ9YIqioymCzLq9gwQbooMDQaHWBfEbwrbw\n\
qHyGO0aoSCqI3Haadr8faqU9GY/rOPNk3sgrDQoo//fb4hVC1CLQJ13hef4Y53CI\n\
rU7m2Ys6xt0nUW7/vGT1M0NPAgMBAAGjQjBAMA4GA1UdDwEB/wQEAwIBBjAPBgNV\n\
HRMBAf8EBTADAQH/MB0GA1UdDgQWBBR5tFnme7bl5AFzgAiIyBpY9umbbjANBgkq\n\
hkiG9w0BAQsFAAOCAgEAVR9YqbyyqFDQDLHYGmkgJykIrGF1XIpu+ILlaS/V9lZL\n\
ubhzEFnTIZd+50xx+7LSYK05qAvqFyFWhfFQDlnrzuBZ6brJFe+GnY+EgPbk6ZGQ\n\
3BebYhtF8GaV0nxvwuo77x/Py9auJ/GpsMiu/X1+mvoiBOv/2X/qkSsisRcOj/KK\n\
NFtY2PwByVS5uCbMiogziUwthDyC3+6WVwW6LLv3xLfHTjuCvjHIInNzktHCgKQ5\n\
ORAzI4JMPJ+GslWYHb4phowim57iaztXOoJwTdwJx4nLCgdNbOhdjsnvzqvHu7Ur\n\
TkXWStAmzOVyyghqpZXjFaH3pO3JLF+l+/+sKAIuvtd7u+Nxe5AW0wdeRlN8NwdC\n\
jNPElpzVmbUq4JUagEiuTDkHzsxHpFKVK7q4+63SM1N95R1NbdWhscdCb+ZAJzVc\n\
oyi3B43njTOQ5yOf+1CceWxG1bQVs5ZufpsMljq4Ui0/1lvh+wjChP4kqKOJ2qxq\n\
4RgqsahDYVvTH9w7jXbyLeiNdd8XM2w9U/t7y0Ff/9yi0GE44Za4rF2LN9d11TPA\n\
mRGunUHBcnWEvgJBQl9nJEiU0Zsnvgc/ubhPgXRR4Xq37Z0j4r7g1SgEEzwxA57d\n\
emyPxgcYxn/eR44/KJ4EBs+lVDR3veyJm+kXQ99b21/+jh5Xos1AnX5iItreGCc=\n\
-----END CERTIFICATE-----\n";

    #[test]
    fn build_root_store_loads_native_certs() {
        // System trust store should have at least a few CAs on a normal
        // dev machine; if this fails the dev environment is unusual.
        let store = build_root_store(None, true).expect("native certs");
        assert!(!store.roots.is_empty(), "native trust store is empty");
    }

    #[test]
    fn build_root_store_rejects_pem_with_no_certs() {
        // A PEM-shaped block that's not a certificate.
        let pem = b"-----BEGIN PRIVATE KEY-----\nMIIBVQ==\n-----END PRIVATE KEY-----\n";
        let err = build_root_store(Some(pem), true).unwrap_err();
        let body = serde_json::to_string(&err).unwrap();
        assert!(body.contains("no PEM CERTIFICATE"), "got: {body}");
    }

    /// Additive default: with `trust_native = true` and a `ca_cert` set,
    /// the store contains both the native count *and* the supplied cert.
    /// This is the regression test for the Supabase use case — operators
    /// can supply Supabase Intermediate 2021 without forfeiting the public
    /// web PKI for other databases sharing the same TlsConfig surface.
    #[test]
    fn build_root_store_extends_native_when_trust_native_true() {
        let native_only = build_root_store(None, true).expect("native certs");
        let native_count = native_only.roots.len();
        let extended = build_root_store(Some(FIXTURE_CA_PEM), true).expect("native + fixture");
        assert_eq!(
            extended.roots.len(),
            native_count + 1,
            "expected native_count + 1 fixture cert; native={native_count} extended={}",
            extended.roots.len()
        );
    }

    /// Strict-isolation: `trust_native = false` + ca_cert provided yields
    /// exactly the supplied cert(s), nothing more. Preserves the old
    /// "trust only my CA" posture from before the additive default.
    #[test]
    fn build_root_store_only_ca_cert_when_trust_native_false() {
        let store = build_root_store(Some(FIXTURE_CA_PEM), false).expect("ca_cert only");
        assert_eq!(store.roots.len(), 1, "expected exactly the fixture cert");
    }

    /// Both flags off is a misconfiguration: there are no trust anchors,
    /// so any TLS handshake would fail. Catch it at config-build time
    /// with a clear `CONFIG_ERROR`, not later as an opaque pool failure.
    #[test]
    fn build_root_store_errors_when_trust_native_false_and_no_ca_cert() {
        let err = build_root_store(None, false).unwrap_err();
        let body = serde_json::to_string(&err).unwrap();
        assert!(
            body.contains("no trust anchors"),
            "expected explicit no-trust-anchors message; got: {body}"
        );
    }

    #[test]
    fn make_pg_connector_disable_returns_none() {
        assert!(make_pg_connector(&ResolvedTls::mode(TlsMode::Disable))
            .unwrap()
            .is_none());
    }

    #[test]
    fn make_pg_connector_builds_for_every_tls_mode() {
        for mode in [
            TlsMode::Prefer,
            TlsMode::Require,
            TlsMode::VerifyCa,
            TlsMode::VerifyFull,
        ] {
            let conn = make_pg_connector(&ResolvedTls::mode(mode))
                .unwrap_or_else(|e| panic!("{mode:?} builds: {e}"));
            assert!(conn.is_some(), "{mode:?}");
        }
    }

    #[test]
    fn make_mysql_ssl_opts_disable_returns_none() {
        assert!(
            make_mysql_ssl_opts(&ResolvedTls::mode(TlsMode::Disable), None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn make_mysql_ssl_opts_skips_domain_validation_except_verify_full() {
        for (mode, skip) in [
            (TlsMode::Prefer, true),
            (TlsMode::Require, true),
            (TlsMode::VerifyCa, true),
            (TlsMode::VerifyFull, false),
        ] {
            let opts = make_mysql_ssl_opts(&ResolvedTls::mode(mode), None)
                .unwrap()
                .unwrap();
            assert_eq!(opts.skip_domain_validation(), skip, "{mode:?}");
            assert!(
                !opts.accept_invalid_certs(),
                "{mode:?} must validate the chain"
            );
            assert_eq!(opts.tls_hostname_override(), None);
        }
        let tunneled =
            make_mysql_ssl_opts(&ResolvedTls::mode(TlsMode::VerifyFull), Some("db.internal"))
                .unwrap()
                .unwrap();
        assert_eq!(tunneled.tls_hostname_override(), Some("db.internal"));
    }

    /// A throwaway self-signed RSA client certificate and its key in both
    /// PEM encodings, generated by the openssl CLI (skips without it).
    fn rsa_client_identity() -> Option<(Vec<u8>, String, String)> {
        let dir = tempfile::tempdir().ok()?;
        let key8 = dir.path().join("k8.pem");
        let key1 = dir.path().join("k1.pem");
        let cert = dir.path().join("c.pem");
        let ok = std::process::Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "2",
                "-subj",
                "/CN=client",
            ])
            .arg("-keyout")
            .arg(&key8)
            .arg("-out")
            .arg(&cert)
            .output()
            .ok()?
            .status
            .success()
            && std::process::Command::new("openssl")
                .args(["rsa", "-traditional", "-in"])
                .arg(&key8)
                .arg("-out")
                .arg(&key1)
                .output()
                .ok()?
                .status
                .success();
        if !ok {
            return None;
        }
        Some((
            std::fs::read(cert).ok()?,
            std::fs::read_to_string(key8).ok()?,
            std::fs::read_to_string(key1).ok()?,
        ))
    }

    #[test]
    fn mutual_tls_identity_builds_for_postgres_and_mysql() {
        let Some((cert, key8, key1)) = rsa_client_identity() else {
            eprintln!("skipping: openssl CLI not available");
            return;
        };
        for key in [&key8, &key1] {
            let tls = ResolvedTls {
                mode: TlsMode::Require,
                trust_native: true,
                ca_pem: None,
                client_cert_pem: Some(cert.clone()),
                client_key_pem: Some(crate::secrets::SecretValue::new(key.clone())),
            };
            assert!(make_pg_connector(&tls).unwrap().is_some());
            let opts = make_mysql_ssl_opts(&tls, None).unwrap().unwrap();
            assert!(opts.client_identity().is_some());
            // The DER handed to mysql_async is PKCS#1 whatever the PEM was.
            let der = mysql_client_key_der(key).unwrap();
            assert_eq!(der[0], 0x30);
        }
        // A key that does not belong to the certificate is refused up front.
        let Some((_, other_key, _)) = rsa_client_identity() else {
            return;
        };
        let tls = ResolvedTls {
            mode: TlsMode::Require,
            trust_native: true,
            ca_pem: None,
            client_cert_pem: Some(cert),
            client_key_pem: Some(crate::secrets::SecretValue::new(other_key)),
        };
        let Err(err) = make_pg_connector(&tls) else {
            panic!("a key that does not fit the certificate must be refused");
        };
        assert!(format!("{err}").contains("does not fit"), "{err}");
    }

    #[test]
    fn mysql_mutual_tls_refuses_a_non_rsa_key_with_a_clear_message() {
        let Ok(out) = std::process::Command::new("openssl")
            .args([
                "genpkey",
                "-algorithm",
                "EC",
                "-pkeyopt",
                "ec_paramgen_curve:P-256",
            ])
            .output()
        else {
            eprintln!("skipping: openssl CLI not available");
            return;
        };
        let pem = String::from_utf8(out.stdout).unwrap();
        if !pem.contains("PRIVATE KEY") {
            return;
        }
        let err = mysql_client_key_der(&pem).unwrap_err();
        assert!(format!("{err}").contains("needs an RSA key"), "{err}");
        assert!(!format!("{err}").contains(&pem[30..60]));
    }

    #[test]
    fn debug_never_prints_the_client_key() {
        let tls = ResolvedTls {
            client_key_pem: Some(crate::secrets::SecretValue::new("TOPSECRETKEY".into())),
            ..ResolvedTls::mode(TlsMode::Require)
        };
        assert!(!format!("{tls:?}").contains("TOPSECRETKEY"));
    }
}
