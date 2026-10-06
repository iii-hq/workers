//! The master key and what it does: XChaCha20-Poly1305 sealing (AAD = the
//! secret name) and the HMAC-SHA256 fingerprint.
use std::fmt;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::error::{codes, SecretsError};
use crate::secret::REDACTED;

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;
/// Stored fingerprints keep the first 16 hex characters (64 bits).
pub const FINGERPRINT_HEX_LEN: usize = 16;
/// Label fingerprinted into the vault header so a wrong key is detected
/// before it seals anything or reports a failed decryption as tampering.
const KEY_CHECK_LABEL: &[u8] = b"iii-secrets/key-check/v1";

/// 32 bytes of unlock material, zeroized on drop and never printed.
#[derive(Clone)]
pub struct MasterKey(Zeroizing<[u8; KEY_LEN]>);

/// One sealed value: a fresh random nonce and the AEAD ciphertext + tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sealed {
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MasterKey({REDACTED})")
    }
}

impl MasterKey {
    pub fn generate() -> Result<Self, SecretsError> {
        let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
        getrandom::getrandom(bytes.as_mut())
            .map_err(|_| SecretsError::key("the operating system random source failed"))?;
        Ok(Self(bytes))
    }

    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Standard base64 (surrounding whitespace ignored) of exactly 32 bytes.
    /// `None` on anything else; the caller words the error for its source.
    pub fn from_base64(text: &str) -> Option<Self> {
        let decoded = Zeroizing::new(STANDARD.decode(text.trim()).ok()?);
        let bytes: [u8; KEY_LEN] = decoded.as_slice().try_into().ok()?;
        Some(Self::from_bytes(bytes))
    }

    pub fn to_base64(&self) -> Zeroizing<String> {
        Zeroizing::new(STANDARD.encode(self.0.as_ref()))
    }

    /// Seal `plaintext` under a fresh nonce, bound to `name` as associated
    /// data: a ciphertext copied onto another record fails to open.
    pub fn seal(&self, name: &str, plaintext: &[u8]) -> Result<Sealed, SecretsError> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::getrandom(&mut nonce)
            .map_err(|_| SecretsError::key("the operating system random source failed"))?;
        let ciphertext = self
            .cipher()
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| SecretsError::vault("sealing the value failed"))?;
        Ok(Sealed { nonce, ciphertext })
    }

    pub fn open(&self, name: &str, sealed: &Sealed) -> Result<Zeroizing<Vec<u8>>, SecretsError> {
        self.cipher()
            .decrypt(
                XNonce::from_slice(&sealed.nonce),
                Payload {
                    msg: &sealed.ciphertext,
                    aad: name.as_bytes(),
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| {
                SecretsError::new(
                    codes::DECRYPT_FAILED,
                    format!("stored value for `{name}` failed authentication"),
                )
            })
    }

    /// `HMAC-SHA256(master_key, value)`, hex, first 16 characters. Stable for
    /// one key and value; changes when either does.
    pub fn fingerprint(&self, value: &[u8]) -> String {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(self.0.as_ref())
            .expect("HMAC accepts any key length");
        mac.update(value);
        let digest = mac.finalize().into_bytes();
        let mut hex = String::with_capacity(FINGERPRINT_HEX_LEN);
        for byte in digest.iter().take(FINGERPRINT_HEX_LEN / 2) {
            hex.push_str(&format!("{byte:02x}"));
        }
        hex
    }

    /// The header value identifying this key without revealing it.
    pub fn key_check(&self) -> String {
        self.fingerprint(KEY_CHECK_LABEL)
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(Key::from_slice(self.0.as_ref()))
    }
}

pub fn encode(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn decode(text: &str) -> Option<Vec<u8>> {
    STANDARD.decode(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> MasterKey {
        MasterKey::from_bytes([byte; KEY_LEN])
    }

    #[test]
    fn seal_open_round_trip() {
        let key = MasterKey::generate().unwrap();
        let sealed = key.seal("OPENAI_API_KEY", b"sk-proj-123456").unwrap();
        assert_ne!(sealed.ciphertext.as_slice(), b"sk-proj-123456");
        let opened = key.open("OPENAI_API_KEY", &sealed).unwrap();
        assert_eq!(opened.as_slice(), b"sk-proj-123456");
    }

    #[test]
    fn nonces_are_fresh_per_seal() {
        let key = key(7);
        let a = key.seal("A", b"same").unwrap();
        let b = key.seal("A", b"same").unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn aad_binds_the_ciphertext_to_its_name() {
        let key = key(1);
        let sealed = key.seal("ANTHROPIC_API_KEY", b"value").unwrap();
        let error = key.open("OPENAI_API_KEY", &sealed).unwrap_err();
        assert_eq!(error.code, codes::DECRYPT_FAILED);
    }

    #[test]
    fn a_different_key_or_a_flipped_bit_fails() {
        let sealed = key(1).seal("A", b"value").unwrap();
        assert!(key(2).open("A", &sealed).is_err());
        let mut tampered = sealed.clone();
        tampered.ciphertext[0] ^= 1;
        assert!(key(1).open("A", &tampered).is_err());
    }

    #[test]
    fn fingerprint_is_stable_and_tracks_value_and_key() {
        let first = key(3).fingerprint(b"sk-old");
        assert_eq!(first.len(), FINGERPRINT_HEX_LEN);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(first, key(3).fingerprint(b"sk-old"));
        // Rotation changes it.
        assert_ne!(first, key(3).fingerprint(b"sk-new"));
        // So does another vault's key.
        assert_ne!(first, key(4).fingerprint(b"sk-old"));
        assert_ne!(key(3).key_check(), key(4).key_check());
    }

    #[test]
    fn base64_keys_must_be_exactly_32_bytes() {
        let key = MasterKey::generate().unwrap();
        let text = key.to_base64();
        let parsed = MasterKey::from_base64(&format!("  {}\n", text.as_str())).unwrap();
        assert_eq!(parsed.key_check(), key.key_check());
        assert!(MasterKey::from_base64(&STANDARD.encode([0u8; 31])).is_none());
        assert!(MasterKey::from_base64("not base64!").is_none());
    }

    #[test]
    fn debug_redacts_the_key() {
        let key = MasterKey::from_bytes([0xAB; KEY_LEN]);
        let printed = format!("{key:?}");
        assert!(printed.contains(REDACTED));
        assert!(!printed.to_lowercase().contains("ab, "), "{printed}");
        assert!(!printed.contains(key.to_base64().as_str()));
    }
}
