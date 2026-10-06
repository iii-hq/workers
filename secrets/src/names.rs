//! Secret names, `secret://NAME` references, consumer lists and the masked hint.
use crate::error::{codes, SecretsError};

/// Scheme of a versionable reference: `secret://NAME`.
pub const REFERENCE_PREFIX: &str = "secret://";
/// `^[A-Za-z_][A-Za-z0-9_.-]{0,127}$`: at most 128 bytes.
pub const MAX_NAME_LEN: usize = 128;
/// The env var that carries the master key. It is unlock material, never a
/// secret: storing, detecting or importing it would hand the key to consumers.
pub const RESERVED_NAMES: [&str; 1] = [crate::keys::KEY_ENV];
const MAX_CONSUMERS: usize = 64;
const MAX_CONSUMER_LEN: usize = 128;
/// Below this many characters a hint shows nothing of the value.
const MASK_MIN_LEN: usize = 12;
const MASK_TAIL: usize = 4;
const MASK_MAX_HEAD: usize = 6;
pub const MASK_HIDDEN: &str = "••••";

/// Whether `name` matches `^[A-Za-z_][A-Za-z0-9_.-]{0,127}$`.
pub fn is_valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let Some((&first, rest)) = bytes.split_first() else {
        return false;
    };
    bytes.len() <= MAX_NAME_LEN
        && (first.is_ascii_alphabetic() || first == b'_')
        && rest
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

pub fn is_reserved(name: &str) -> bool {
    RESERVED_NAMES.contains(&name)
}

/// A name a secret may be stored, detected or imported under. The input is
/// never echoed: a caller that pasted a value into `name` must not see it in
/// an error, a span or a log.
pub fn validate_name(name: &str) -> Result<(), SecretsError> {
    if !is_valid_name(name) {
        return Err(SecretsError::invalid_request(
            "name must match ^[A-Za-z_][A-Za-z0-9_.-]{0,127}$",
        ));
    }
    if is_reserved(name) {
        return Err(SecretsError::invalid_request(format!(
            "`{name}` is reserved for the master key and cannot be stored"
        )));
    }
    Ok(())
}

/// `secret://NAME` or a bare `NAME` to the name. Surrounding whitespace is
/// ignored; anything else that does not match the name pattern is
/// `INVALID_REFERENCE`, without echoing the input.
pub fn parse_reference(reference: &str) -> Result<&str, SecretsError> {
    let trimmed = reference.trim();
    let name = trimmed.strip_prefix(REFERENCE_PREFIX).unwrap_or(trimmed);
    if is_valid_name(name) {
        Ok(name)
    } else {
        Err(SecretsError::new(
            codes::INVALID_REFERENCE,
            "reference must be secret://NAME or NAME, with NAME matching ^[A-Za-z_][A-Za-z0-9_.-]{0,127}$",
        ))
    }
}

/// Whether a configuration value is a `secret://` reference (as opposed to a
/// literal credential); consumers use the same test before resolving.
pub fn is_reference(value: &str) -> bool {
    value.trim().starts_with(REFERENCE_PREFIX)
}

pub fn reference_for(name: &str) -> String {
    format!("{REFERENCE_PREFIX}{name}")
}

/// The display hint: with 12 or more characters, the first `min(6, len/4)`
/// and the last 4 around `…` (`sk-ant…9f2c`); shorter values show nothing.
pub fn mask(value: &str) -> String {
    let len = value.chars().count();
    if len < MASK_MIN_LEN {
        return MASK_HIDDEN.to_owned();
    }
    let head = MASK_MAX_HEAD.min(len / 4);
    let byte_at = |index: usize| {
        value
            .char_indices()
            .nth(index)
            .map_or(value.len(), |(at, _)| at)
    };
    format!(
        "{}…{}",
        &value[..byte_at(head)],
        &value[byte_at(len - MASK_TAIL)..]
    )
}

/// Trim, drop blanks and duplicates (first occurrence wins). Consumers are
/// worker names: no whitespace or control characters, at most 128 bytes.
pub fn normalize_consumers(consumers: Vec<String>) -> Result<Vec<String>, SecretsError> {
    let mut out: Vec<String> = Vec::with_capacity(consumers.len());
    for consumer in consumers {
        let consumer = consumer.trim();
        if consumer.is_empty() {
            continue;
        }
        if consumer.len() > MAX_CONSUMER_LEN
            || consumer
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(SecretsError::invalid_request(
                "consumers must be worker names: no whitespace, at most 128 bytes each",
            ));
        }
        if !out.iter().any(|existing| existing == consumer) {
            out.push(consumer.to_owned());
        }
    }
    if out.len() > MAX_CONSUMERS {
        return Err(SecretsError::invalid_request(format!(
            "at most {MAX_CONSUMERS} consumers per secret"
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_pattern() {
        for valid in [
            "ANTHROPIC_API_KEY",
            "_x",
            "a",
            "a.b-c_d9",
            &"A".repeat(MAX_NAME_LEN),
        ] {
            assert!(is_valid_name(valid), "{valid}");
        }
        for invalid in [
            "",
            "9LIVES",
            "-x",
            ".x",
            "has space",
            "slash/name",
            "colon:name",
            "ünïcode",
            &"A".repeat(MAX_NAME_LEN + 1),
        ] {
            assert!(!is_valid_name(invalid), "{invalid}");
        }
    }

    #[test]
    fn the_master_key_variable_is_reserved() {
        let error = validate_name("III_SECRETS_KEY").unwrap_err();
        assert_eq!(error.code, codes::INVALID_REQUEST);
        assert!(validate_name("OPENAI_API_KEY").is_ok());
    }

    #[test]
    fn invalid_names_are_never_echoed() {
        let pasted = "sk live 1234567890abcdef";
        let error = validate_name(pasted).unwrap_err();
        assert!(!error.message.contains("1234567890"), "{error}");
        let error = parse_reference(pasted).unwrap_err();
        assert!(!error.message.contains("1234567890"), "{error}");
    }

    #[test]
    fn references_parse_with_or_without_the_scheme() {
        assert_eq!(
            parse_reference("secret://ANTHROPIC_API_KEY").unwrap(),
            "ANTHROPIC_API_KEY"
        );
        assert_eq!(parse_reference("OPENAI_API_KEY").unwrap(), "OPENAI_API_KEY");
        assert_eq!(parse_reference("  secret://A.b-c  ").unwrap(), "A.b-c");
        for invalid in [
            "",
            "secret://",
            "secret://9x",
            "env://X",
            "https://example.com",
            "secret://a/b",
            "secret:/X",
        ] {
            let error = parse_reference(invalid).unwrap_err();
            assert_eq!(error.code, codes::INVALID_REFERENCE, "{invalid}");
        }
        assert_eq!(reference_for("X"), "secret://X");
        assert!(is_reference(" secret://X"));
        assert!(!is_reference("sk-ant-123"));
    }

    #[test]
    fn mask_matches_the_contract() {
        // len 28 → head min(6, 7) = 6
        assert_eq!(mask("sk-ant-api03-abcdefghij-9f2c"), "sk-ant…9f2c");
        // len 12 → head 3
        assert_eq!(mask("abcdefghijkl"), "abc…ijkl");
        // len 16 → head 4
        assert_eq!(mask("0123456789abcdef"), "0123…cdef");
        assert_eq!(mask("short"), MASK_HIDDEN);
        assert_eq!(mask("elevenchars"), MASK_HIDDEN);
        assert_eq!(mask(""), MASK_HIDDEN);
        // Character-, not byte-based.
        assert_eq!(mask("ééééééééééééé"), "ééé…éééé");
    }

    #[test]
    fn consumers_are_trimmed_and_deduplicated() {
        let consumers = normalize_consumers(vec![
            " llm-router ".into(),
            "".into(),
            "judge-typesafe".into(),
            "llm-router".into(),
        ])
        .unwrap();
        assert_eq!(consumers, vec!["llm-router", "judge-typesafe"]);
        assert!(normalize_consumers(vec!["two words".into()]).is_err());
        assert!(normalize_consumers(vec!["x".repeat(129)]).is_err());
        assert!(normalize_consumers((0..65).map(|i| format!("w{i}")).collect()).is_err());
    }
}
