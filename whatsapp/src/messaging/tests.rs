use super::*;

#[test]
fn reply_keeps_lid_sender_and_quote_context() {
    let request = ReplyRequest {
        session_id: "test".into(),
        chat_id: "123456-789@g.us".into(),
        message_id: "ORIGINAL".into(),
        sender_id: "987654321@lid".into(),
        quoted_text: "Original text".into(),
        text: "The answer".into(),
    };
    let message = request.message().unwrap();
    let extended = message.extended_text_message.as_option().unwrap();
    assert_eq!(extended.text.as_deref(), Some("The answer"));
    let context = extended.context_info.as_option().unwrap();
    assert_eq!(context.stanza_id.as_deref(), Some("ORIGINAL"));
    assert_eq!(context.participant.as_deref(), Some("987654321@lid"));
    assert_eq!(context.remote_jid.as_deref(), Some("123456-789@g.us"));
    assert_eq!(
        context
            .quoted_message
            .as_option()
            .unwrap()
            .conversation
            .as_deref(),
        Some("Original text")
    );
}

#[test]
fn group_message_key_requires_the_original_sender() {
    let mut key = MessageKeyInput {
        chat_id: "123456-789@g.us".into(),
        message_id: "MESSAGE".into(),
        from_me: false,
        participant_id: None,
    };
    assert!(key.native().is_err());
    key.participant_id = Some("987654321@lid".into());
    let native = key.native().unwrap();
    assert_eq!(native.participant.as_deref(), Some("987654321@lid"));
    assert_eq!(native.from_me, Some(false));
    assert_eq!(native.id.as_deref(), Some("MESSAGE"));
}

#[test]
fn invalid_coordinates_do_not_construct_a_message() {
    let mut request = LocationRequest {
        session_id: "test".into(),
        chat_id: "15551234567".into(),
        latitude: 91.0,
        longitude: 0.0,
        name: None,
        address: None,
    };
    assert!(request.message().is_err());
    request.latitude = -12.4;
    request.longitude = -38.5;
    let message = request.message().unwrap();
    let location = message.location_message.as_option().unwrap();
    assert_eq!(location.degrees_latitude, Some(-12.4));
    assert_eq!(location.degrees_longitude, Some(-38.5));
}

#[test]
fn poll_options_must_be_unique_and_selection_bounded() {
    let mut request = PollRequest {
        session_id: "test".into(),
        chat_id: "15551234567".into(),
        name: "Choose".into(),
        options: vec!["A".into(), "A".into()],
        selectable_count: 1,
    };
    assert!(request.validate().is_err());
    request.options[1] = "B".into();
    request.selectable_count = 3;
    assert!(request.validate().is_err());
    request.selectable_count = 1;
    assert!(request.validate().is_ok());
}

#[test]
fn status_requires_explicit_individual_recipients() {
    assert!(status::recipients(&[]).is_err());
    assert!(status::recipients(&["123456-789@g.us".into()]).is_err());
    let audience = status::recipients(&["15551234567@c.us".into(), "12345@lid".into()]).unwrap();
    assert_eq!(audience[0].to_string(), "15551234567@s.whatsapp.net");
    assert_eq!(audience[1].to_string(), "12345@lid");
}

#[test]
fn media_download_rejects_urls_and_malformed_crypto_metadata() {
    let mut request = media::DownloadRequest {
        session_id: "test".into(),
        media_type: media::MediaKind::Image,
        direct_path: "http://127.0.0.1/secrets".into(),
        media_key_base64: Some(BASE64.encode([0; 32])),
        file_sha256_base64: BASE64.encode([1; 32]),
        file_enc_sha256_base64: Some(BASE64.encode([2; 32])),
        file_length: 8,
    };
    assert!(request.params(1024).is_err());
    request.direct_path = "/v/t62.7118-24/abc.enc?ccb=11".into();
    assert!(request.params(1024).is_ok());
    request.file_sha256_base64 = BASE64.encode([1; 31]);
    assert!(request.params(1024).is_err());
    request.file_sha256_base64 = BASE64.encode([1; 32]);
    request.file_length = 1025;
    assert!(request.params(1024).is_err());
}

#[test]
fn bounded_media_sink_enforces_actual_download_size() {
    use std::io::{Seek, SeekFrom, Write};
    use wacore::download::DownloadWriter;
    let mut sink = media::BoundedWriter::new(4);
    sink.write_all(&[1, 2, 3, 4]).unwrap();
    assert!(sink.write_all(&[5]).is_err());
    assert!(sink.seek(SeekFrom::Start(5)).is_err());
    sink.truncate(0).unwrap();
    sink.seek(SeekFrom::Start(0)).unwrap();
    sink.write_all(&[9, 8]).unwrap();
    assert_eq!(sink.into_bytes(), vec![9, 8]);
}

#[test]
fn chat_range_converts_milliseconds_to_protocol_seconds() {
    let range = chats::MessageRange {
        last_message_timestamp_ms: 1_700_000_001_234,
        last_system_message_timestamp_ms: Some(1_700_000_000_999),
        messages: vec![chats::RangeMessage {
            key: MessageKeyInput {
                chat_id: "15551234567".into(),
                message_id: "MESSAGE".into(),
                from_me: true,
                participant_id: None,
            },
            timestamp_ms: 1_700_000_001_234,
        }],
    }
    .native(&validate::jid("15551234567").unwrap())
    .unwrap();
    assert_eq!(range.last_message_timestamp, Some(1_700_000_001));
    assert_eq!(range.last_system_message_timestamp, Some(1_700_000_000));
    assert_eq!(range.messages[0].timestamp, Some(1_700_000_001));
}
