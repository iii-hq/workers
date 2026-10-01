use super::*;
use whatsapp_rust::{StatusPrivacySetting, StatusSendOptions};

pub(super) fn recipients(ids: &[String]) -> Result<Vec<Jid>> {
    ensure!(
        !ids.is_empty() && ids.len() <= 1024,
        "status recipients must contain 1 to 1024 people"
    );
    let mut seen = std::collections::HashSet::new();
    ids.iter()
        .map(|id| {
            let jid = validate::jid(id)?;
            ensure!(
                (jid.is_pn() || jid.is_lid()) && jid.device == 0,
                "status recipients must identify individual people"
            );
            ensure!(
                seen.insert(jid.to_string()),
                "status recipients must be unique"
            );
            Ok(jid)
        })
        .collect()
}

fn options() -> StatusSendOptions {
    StatusSendOptions {
        privacy: StatusPrivacySetting::AllowList,
        ..Default::default()
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatusFont {
    System,
    SystemText,
    Script,
    Bold,
    MorningBreeze,
    Calistoga,
    Exo,
    Courier,
}

impl StatusFont {
    fn native(self) -> wa::message::extended_text_message::FontType {
        use wa::message::extended_text_message::FontType as F;
        match self {
            Self::System => F::SYSTEM,
            Self::SystemText => F::SYSTEM_TEXT,
            Self::Script => F::FB_SCRIPT,
            Self::Bold => F::SYSTEM_BOLD,
            Self::MorningBreeze => F::MORNINGBREEZE_REGULAR,
            Self::Calistoga => F::CALISTOGA_REGULAR,
            Self::Exo => F::EXO2_EXTRABOLD,
            Self::Courier => F::COURIERPRIME_BOLD,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusTextRequest {
    pub session_id: String,
    pub text: String,
    pub recipients: Vec<String>,
    pub background_argb: u32,
    pub font: StatusFont,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusMediaRequest {
    pub session_id: String,
    pub recipients: Vec<String>,
    pub media: media::MediaContent,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeStatusRequest {
    pub session_id: String,
    pub message_id: String,
    /// Supply the same audience used when publishing this status.
    pub recipients: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct StatusAudience {
    /// Latest native privacy action; null means the linked phone has not synced an audience.
    pub audience: Option<serde_json::Value>,
}

pub(super) fn register(registry: &mut Registry) {
    registry.add::<SessionRequest, StatusAudience, _, _>(
        "whatsapp::status::audience",
        "Read the last synced status privacy action; it does not calculate recipients.",
        |m, r| async move {
            let client = m.client(&r.session_id).await?;
            let audience = client
                .status()
                .audience()
                .map(|v| serde_json::to_value(v.as_ref()))
                .transpose()?;
            Ok(StatusAudience { audience })
        },
    );
    registry.add::<StatusTextRequest, SentMessage, _, _>(
        "whatsapp::status::send_text",
        "Publish a text status to an explicit allowlist of people.",
        |m, r| async move {
            validate::text(&r.text, "text", 700)?;
            let audience = recipients(&r.recipients)?;
            Ok(m.client(&r.session_id)
                .await?
                .status()
                .send_text(
                    &r.text,
                    r.background_argb,
                    r.font.native(),
                    &audience,
                    options(),
                )
                .await?
                .into())
        },
    );
    registry.add::<StatusMediaRequest, SentMessage, _, _>(
        "whatsapp::status::send_media",
        "Publish encoded image, video or voice audio status to an explicit allowlist.",
        |m, r| async move {
            let audience = recipients(&r.recipients)?;
            ensure!(
                matches!(
                    r.media.media_type,
                    media::MediaKind::Image | media::MediaKind::Video | media::MediaKind::Audio
                ),
                "status media must be image, video or voice audio"
            );
            ensure!(
                !matches!(r.media.media_type, media::MediaKind::Audio) || r.media.voice_note,
                "audio status requires voice_note=true and prepared voice audio"
            );
            let (data, thumbnail) = r.media.validate(m.config().max_media_bytes)?;
            let client = m.client(&r.session_id).await?;
            let upload = client
                .upload(data, r.media.media_type.native(), Default::default())
                .await?;
            Ok(client
                .status()
                .send_raw(r.media.message(upload, thumbnail), &audience, options())
                .await?
                .into())
        },
    );
    registry.add::<RevokeStatusRequest, SentMessage, _, _>(
        "whatsapp::status::revoke",
        "Revoke a status for the same explicit audience used when publishing it.",
        |m, r| async move {
            message_id(&r.message_id)?;
            let audience = recipients(&r.recipients)?;
            Ok(m.client(&r.session_id)
                .await?
                .status()
                .revoke(r.message_id, &audience, options())
                .await?
                .into())
        },
    );
}
