use whatsapp::{config::Config, validate};

#[test]
fn phone_and_openwa_ids_normalize_without_reinterpreting_lids() {
    for (input, expected) in [
        ("+15550000001", "15550000001@s.whatsapp.net"),
        ("15550000001@c.us", "15550000001@s.whatsapp.net"),
        ("987654321@lid", "987654321@lid"),
        ("987654321:4@lid", "987654321:4@lid"),
        (
            "15550000001:2@s.whatsapp.net",
            "15550000001:2@s.whatsapp.net",
        ),
        ("120363000000000001@g.us", "120363000000000001@g.us"),
    ] {
        assert_eq!(validate::jid(input).unwrap().to_string(), expected);
    }
    for input in [
        "",
        "../session",
        "12",
        "123@unknown",
        "123@lid@c.us",
        "1555 0000001",
        "123:x@lid",
        "123:2:3@lid",
        "-@g.us",
        "12--34@g.us",
        "123-@g.us",
    ] {
        assert!(validate::jid(input).is_err(), "accepted {input:?}");
    }
}

#[test]
fn bounded_base64_rejects_oversized_or_malformed_media() {
    assert_eq!(validate::media("aGVsbG8=", 5).unwrap(), b"hello");
    assert!(validate::media("aGVsbG8=", 4).is_err());
    assert!(validate::media("not base64!", 100).is_err());
    assert!(validate::media("", 100).is_err());
}

#[test]
fn invalid_limits_and_unknown_config_fields_fail() {
    assert!(Config::default().validate().is_ok());
    for input in [
        "max_sessions: 0",
        "max_events: 0",
        "operation_timeout_secs: 0",
        "unexpected: true",
    ] {
        assert!(Config::from_yaml(input).is_err(), "accepted {input}");
    }
}

#[test]
fn text_limits_count_utf8_bytes_and_reject_blank_payloads() {
    assert!(validate::text("á", "text", 2).is_ok());
    assert!(validate::text("á", "text", 1).is_err());
    assert!(validate::text("  ", "text", 10).is_err());
}
