use serde_json::{json, Value};

pub fn build_manifest() -> Value {
    json!({
        "name": "email",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Email worker — SMTP send + real-time IMAP read with IDLE push.",
        "functions": [
            { "id": "email::send",              "stability": "beta" },
            { "id": "email::accounts::list",    "stability": "stable" },
            { "id": "email::list",              "stability": "beta" },
            { "id": "email::get",               "stability": "beta" },
            { "id": "email::search",            "stability": "beta", "streams": "ndjson" },
            { "id": "email::flag",              "stability": "beta" },
            { "id": "email::move",              "stability": "beta" },
            { "id": "email::attachment::get",   "stability": "beta", "streams": "bytes" }
        ],
        "trigger_types": [
            {
                "name": "email::new-mail",
                "stability": "beta",
                "description": "Fires when IMAP IDLE pushes a new message to a configured (account, folder)."
            }
        ],
        "default_config": {
            "limits": {
                "max_attachment_bytes": 26214400,
                "max_recipients": 100,
                "send_timeout_ms": 30000,
                "imap_connect_timeout_ms": 15000
            }
        }
    })
}
