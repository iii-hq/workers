//! A credential value in memory: zeroized on drop, redacted in `Debug`.
use std::fmt;

use schemars::gen::SchemaGenerator;
use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zeroize::Zeroizing;

/// Placeholder every `Debug` impl prints instead of a value.
pub const REDACTED: &str = "[REDACTED]";

/// Serializes as the plain string (only `secrets::resolve` returns one), but
/// never prints it through `Debug`, so a `{:?}` in a log line, a panic
/// message or a derived `Debug` on a request struct cannot leak it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    /// The plaintext. Keep the borrow short; never format it.
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }

    pub fn is_blank(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self::new(value.to_owned())
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl Serialize for SecretString {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.expose())
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

impl JsonSchema for SecretString {
    fn is_referenceable() -> bool {
        false
    }

    fn schema_name() -> String {
        "SecretString".to_owned()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        String::json_schema(generator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_the_value() {
        let secret = SecretString::from("sk-ant-api03-very-secret-9f2c");
        let printed = format!("{secret:?} {secret:#?}");
        assert!(!printed.contains("very-secret"), "{printed}");
        assert!(printed.contains(REDACTED));
    }

    #[test]
    fn serde_round_trips_the_plain_string() {
        let secret: SecretString = serde_json::from_str("\"abc\"").unwrap();
        assert_eq!(secret.expose(), "abc");
        assert_eq!(serde_json::to_string(&secret).unwrap(), "\"abc\"");
    }
}
