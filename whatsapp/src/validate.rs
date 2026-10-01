use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use whatsapp_rust::Jid;

/// Accept phone numbers and supported user/chat JIDs; preserve opaque LIDs.
pub fn jid(input: &str) -> Result<Jid> {
    ensure!(
        !input.is_empty() && input.len() <= 128,
        "invalid JID length"
    );
    let normalized = if let Some((user, server)) = input.split_once('@') {
        ensure!(!user.is_empty(), "JID user is empty");
        ensure!(
            matches!(
                server,
                "c.us" | "s.whatsapp.net" | "lid" | "g.us" | "newsletter" | "broadcast"
            ),
            "unsupported JID server"
        );
        let valid_user = match server {
            "broadcast" => user == "status" || user.bytes().all(|c| c.is_ascii_digit()),
            "g.us" => {
                let parts: Vec<_> = user.split('-').collect();
                parts.len() <= 2
                    && parts
                        .iter()
                        .all(|part| !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit()))
            }
            "c.us" | "s.whatsapp.net" | "lid" => {
                let (identity, device) = user
                    .split_once(':')
                    .map_or((user, None), |(id, device)| (id, Some(device)));
                !identity.is_empty()
                    && identity.bytes().all(|c| c.is_ascii_digit())
                    && device.is_none_or(|device| {
                        !device.is_empty()
                            && device.bytes().all(|c| c.is_ascii_digit())
                            && device.parse::<u16>().is_ok()
                    })
            }
            _ => user.bytes().all(|c| c.is_ascii_digit()),
        };
        ensure!(valid_user, "invalid JID user");
        format!(
            "{user}@{}",
            if server == "c.us" {
                "s.whatsapp.net"
            } else {
                server
            }
        )
    } else {
        let phone = input.strip_prefix('+').unwrap_or(input);
        ensure!(
            (7..=15).contains(&phone.len()) && phone.bytes().all(|c| c.is_ascii_digit()),
            "phone must contain 7..15 digits"
        );
        format!("{phone}@s.whatsapp.net")
    };
    normalized.parse().context("invalid JID")
}

pub fn text(input: &str, field: &str, max: usize) -> Result<()> {
    ensure!(!input.trim().is_empty(), "{field} must not be blank");
    ensure!(input.len() <= max, "{field} exceeds {max} bytes");
    Ok(())
}

pub fn media(input: &str, max_bytes: usize) -> Result<Vec<u8>> {
    ensure!(!input.is_empty(), "media must not be empty");
    let encoded_limit = max_bytes.div_ceil(3).saturating_mul(4);
    ensure!(
        input.len() <= encoded_limit,
        "media exceeds configured byte limit"
    );
    let decoded = STANDARD
        .decode(input)
        .context("media must be standard base64")?;
    ensure!(
        !decoded.is_empty() && decoded.len() <= max_bytes,
        "media exceeds configured byte limit"
    );
    Ok(decoded)
}
