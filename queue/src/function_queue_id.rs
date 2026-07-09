use sha2::{Digest, Sha256};

const PHYSICAL_PREFIX: &str = "fnq";

/// Convert a logical function ID into a stable, readable broker-safe name.
///
/// The digest is part of the name even when the readable prefixes differ so
/// IDs that sanitize to the same text cannot alias one another.
pub fn physical_function_queue_name(function_id: &str) -> String {
    let readable = sanitize_prefix(function_id);
    let digest = Sha256::digest(function_id.as_bytes());
    let short_hash = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{PHYSICAL_PREFIX}.{readable}.{short_hash}")
}

/// Tagged adapter key used by generic stats/DLQ operations to distinguish a
/// function queue from an ordinary topic. The tag is stripped before broker
/// resource names are built and is never exposed in public results.
pub fn function_queue_adapter_key(function_id: &str) -> String {
    format!("__fn_queue::{}", physical_function_queue_name(function_id))
}

fn sanitize_prefix(function_id: &str) -> String {
    let mut result = String::with_capacity(48);
    let mut separator = false;
    for character in function_id.chars() {
        let safe = character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_');
        if safe {
            result.push(character);
            separator = false;
        } else if !separator && !result.is_empty() {
            result.push('-');
            separator = true;
        }
        if result.len() >= 48 {
            break;
        }
    }
    while result.ends_with('-') {
        result.pop();
    }
    if result.is_empty() {
        "function".to_string()
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_names_are_stable_safe_and_bounded() {
        let function_id = "orders::create/with a very long suffix";
        let first = physical_function_queue_name(function_id);
        assert_eq!(first, physical_function_queue_name(function_id));
        assert!(first.len() < 100);
        assert!(first
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character)));
        assert!(first.starts_with("fnq.orders-create-with-a-very-long-suffix."));
    }

    #[test]
    fn sanitization_collisions_still_have_distinct_names() {
        assert_ne!(
            physical_function_queue_name("orders::create"),
            physical_function_queue_name("orders/create")
        );
    }

    #[test]
    fn adapter_key_is_tagged_without_exposing_the_physical_name() {
        let key = function_queue_adapter_key("orders::create");
        assert!(key.starts_with("__fn_queue::fnq.orders-create."));
        assert_eq!(key, function_queue_adapter_key("orders::create"));
    }
}
