use serde_json::{Value, json};
use whatsapp::{config::Config, messaging, registry::Registry, sessions::SessionManager};

fn manager() -> (tempfile::TempDir, std::sync::Arc<SessionManager>) {
    let directory = tempfile::tempdir().unwrap();
    let manager = SessionManager::open(Config {
        data_dir: directory.path().join("sessions"),
        ..Default::default()
    })
    .unwrap();
    (directory, manager)
}

#[tokio::test]
async fn group_reactions_require_sender_even_when_authored_by_this_account() {
    let (_directory, manager) = manager();
    let mut registry = Registry::default();
    messaging::register(&mut registry);
    let error = registry
        .invoke(
            "whatsapp::messages::react",
            manager,
            json!({
                "session_id": "absent", "emoji": "👍", "target": {
                    "chat_id": "123456-789@g.us", "message_id": "MESSAGE", "from_me": true
                }
            }),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("participant_id"), "{error:#}");
}

#[tokio::test]
async fn prepared_voice_status_validates_before_requiring_connected_session() {
    let (_directory, manager) = manager();
    let mut registry = Registry::default();
    messaging::register(&mut registry);
    let error = registry
        .invoke(
            "whatsapp::status::send_media",
            manager,
            json!({
                "session_id": "absent", "recipients": ["15551234567"], "media": {
                    "media_type": "audio", "data_base64": "T2dnUw==", "voice_note": true,
                    "mimetype": "audio/ogg; codecs=opus"
                }
            }),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unknown session"), "{error:#}");
}

#[tokio::test]
async fn encrypted_poll_votes_cannot_use_the_channel_message_path() {
    let (_directory, manager) = manager();
    let mut registry = Registry::default();
    messaging::register(&mut registry);
    let error = registry.invoke("whatsapp::messages::vote_poll", manager, json!({
        "session_id": "absent", "chat_id": "123456@newsletter", "message_id": "ORIGINAL",
        "creator_id": "15551234567", "message_secret_base64": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "option_names": ["A"]
    })).await.unwrap_err();
    assert!(error.to_string().contains("direct or group"), "{error:#}");
}

#[tokio::test]
async fn message_requests_reject_unknown_fields_and_empty_status_audiences() {
    let (_directory, manager) = manager();
    let mut registry = Registry::default();
    messaging::register(&mut registry);
    let error = registry
        .invoke(
            "whatsapp::messages::send_text",
            manager.clone(),
            json!({
                "session_id": "absent", "chat_id": "15551234567", "text": "Hello", "typo": true
            }),
        )
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("unknown field"));
    let error = registry.invoke("whatsapp::status::send_text", manager, json!({
        "session_id": "absent", "text": "Hello", "recipients": [], "font": "system", "background_argb": 0
    })).await.unwrap_err();
    assert!(error.to_string().contains("recipients"));
}

#[tokio::test]
async fn media_rejects_url_inputs_and_oversize_data_before_network() {
    let (_directory, manager) = manager();
    let mut registry = Registry::default();
    messaging::register(&mut registry);
    for (route, payload) in [
        (
            "whatsapp::media::send",
            json!({ "chat_id": "15551234567", "media": {
                "media_type": "image", "data_base64": "http://localhost/private"
            }}),
        ),
        (
            "whatsapp::media::upload",
            json!({ "media_type": "not_media", "data_base64": "YWJj" }),
        ),
        (
            "whatsapp::media::download",
            json!({ "media_type": "image", "direct_path": "http://localhost/private",
                "file_sha256_base64": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", "file_length": 8
            }),
        ),
    ] {
        let mut payload = payload;
        payload["session_id"] = json!("absent");
        let error = registry
            .invoke(route, manager.clone(), payload)
            .await
            .unwrap_err();
        assert!(
            !error.to_string().contains("unknown session"),
            "{route}: {error:#}"
        );
    }
    let mut payload = json!({ "session_id": "absent", "media_type": "image", "data_base64": "" });
    payload["data_base64"] = Value::String("A".repeat(manager.config().max_media_bytes * 2));
    let error = registry
        .invoke("whatsapp::media::upload", manager, payload)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("byte limit"));
}

// Every family must reject disconnected sessions before upload, IQ, or sends.
// These are valid operation inputs so validation cannot hide a missing guard.
fn valid_request(id: &str) -> Value {
    let chat = "15551234567";
    let channel = "123456@newsletter";
    let key = json!({"chat_id": chat, "message_id": "ORIGINAL", "from_me": true});
    let media = json!({"media_type": "image", "data_base64": "/9j/"});
    match id.strip_prefix("whatsapp::").unwrap() {
        "messages::send_text" => json!({"chat_id": chat, "text": "Hello"}),
        "messages::reply" => {
            json!({"chat_id": chat, "text": "Reply", "message_id": "ORIGINAL", "sender_id": chat, "quoted_text": "Hello"})
        }
        "messages::edit" => json!({"chat_id": chat, "message_id": "ORIGINAL", "text": "Updated"}),
        "messages::delete" => json!({"chat_id": chat, "message_id": "ORIGINAL"}),
        "messages::react" => json!({"target": key, "emoji": "👍"}),
        "messages::send_location" => json!({"chat_id": chat, "latitude": 0.0, "longitude": 0.0}),
        "messages::send_contact" => {
            json!({"chat_id": chat, "display_name": "Name", "vcard": "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Name\r\nEND:VCARD"})
        }
        "messages::send_poll" => {
            json!({"chat_id": chat, "name": "Choose", "options": ["A", "B"], "selectable_count": 1})
        }
        "messages::vote_poll" => {
            json!({"chat_id": chat, "message_id": "ORIGINAL", "creator_id": chat, "message_secret_base64": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", "option_names": ["A"]})
        }
        "messages::mark_read" | "messages::mark_played" => {
            json!({"chat_id": chat, "message_ids": ["ORIGINAL"]})
        }
        "messages::star" | "messages::unstar" | "messages::unpin" => json!({"target": key}),
        "messages::pin" => json!({"target": key, "duration": "days7"}),
        "messages::keep" => json!({"target": key, "keep": true}),
        "messages::delete_for_me" => json!({"target": key, "delete_media": false}),
        "media::upload" => json!({"media_type": "image", "data_base64": "/9j/"}),
        "media::send" => json!({"chat_id": chat, "media": media}),
        "media::download" => {
            json!({"media_type": "image", "direct_path": "/v/t62.7118-24/test.enc", "file_sha256_base64": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", "file_length": 3})
        }
        "chats::pin"
        | "chats::unpin"
        | "chats::mute"
        | "chats::unmute"
        | "chats::lock"
        | "chats::unlock"
        | "chats::archive"
        | "chats::unarchive"
        | "chats::subscribe_presence"
        | "chats::unsubscribe_presence" => json!({"chat_id": chat}),
        "chats::mark_read" => json!({"chat_id": chat, "read": true}),
        "chats::delete" => json!({"chat_id": chat, "delete_media": false}),
        "chats::clear" => json!({"chat_id": chat, "delete_media": false, "delete_starred": false}),
        "chats::presence" => json!({"presence": "available"}),
        "chats::state" => json!({"chat_id": chat, "state": "composing"}),
        "status::audience" | "newsletters::list" => json!({}),
        "status::send_text" => {
            json!({"text": "Status", "recipients": [chat], "background_argb": 0xff000000_u32, "font": "system"})
        }
        "status::send_media" => json!({"recipients": [chat], "media": media}),
        "status::revoke" => json!({"recipients": [chat], "message_id": "ORIGINAL"}),
        "labels::set" => json!({"label_id": "1", "name": "Work", "color": 1}),
        "labels::delete" => json!({"label_id": "1"}),
        "labels::add_chat" | "labels::remove_chat" => json!({"label_id": "1", "chat_id": chat}),
        "labels::add_message" | "labels::remove_message" => {
            json!({"label_id": "1", "chat_id": chat, "message_id": "ORIGINAL"})
        }
        "newsletters::get"
        | "newsletters::join"
        | "newsletters::leave"
        | "newsletters::delete"
        | "newsletters::remove_picture"
        | "newsletters::admin_info"
        | "newsletters::subscribe_updates" => json!({"newsletter_id": channel}),
        "newsletters::create" => json!({"name": "Channel"}),
        "newsletters::update" => json!({"newsletter_id": channel, "name": "Updated"}),
        "newsletters::set_picture" => json!({"newsletter_id": channel, "jpeg_base64": "/9j/"}),
        "newsletters::change_owner" | "newsletters::demote_admin" => {
            json!({"newsletter_id": channel, "user_id": chat})
        }
        "newsletters::followers" | "newsletters::messages" | "newsletters::my_addons" => {
            json!({"newsletter_id": channel, "count": 10})
        }
        "newsletters::mute_updates" | "newsletters::mute_follower_activity" => {
            json!({"newsletter_id": channel, "muted": true})
        }
        "newsletters::get_by_invite" => json!({"invite_code": "AbcDef"}),
        "newsletters::react" => json!({"newsletter_id": channel, "server_id": 1, "emoji": "👍"}),
        "newsletters::vote_poll" => {
            json!({"newsletter_id": channel, "server_id": 1, "option_names": ["A"]})
        }
        "newsletters::edit" => {
            json!({"newsletter_id": channel, "message_id": "ORIGINAL", "text": "Edited"})
        }
        "newsletters::revoke" => json!({"newsletter_id": channel, "message_id": "ORIGINAL"}),
        unexpected => panic!("missing valid offline fixture for {unexpected}"),
    }
}

#[tokio::test]
async fn all_messaging_operations_require_an_existing_connected_session() {
    let (_directory, manager) = manager();
    manager.create("stopped").await.unwrap();
    let mut registry = Registry::default();
    messaging::register(&mut registry);
    for operation in registry.catalog() {
        for (session, expected) in [("absent", "unknown session"), ("stopped", "not connected")] {
            let mut payload = valid_request(&operation.id);
            payload["session_id"] = json!(session);
            let error = registry
                .invoke(&operation.id, manager.clone(), payload)
                .await
                .unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "{}: {error:#}",
                operation.id
            );
        }
    }
    manager.shutdown().await;
}

#[tokio::test]
async fn media_send_rejects_broadcast_destinations_before_upload() {
    let (_directory, manager) = manager();
    let mut registry = Registry::default();
    messaging::register(&mut registry);
    for destination in ["status@broadcast", "123456@broadcast", "123456@newsletter"] {
        let error = registry
            .invoke(
                "whatsapp::media::send",
                manager.clone(),
                json!({
                    "session_id": "absent", "chat_id": destination,
                    "media": {"media_type": "image", "data_base64": "/9j/"}
                }),
            )
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("direct or group chat"),
            "{destination}: {error:#}"
        );
    }
}
