use super::*;
use whatsapp_rust::{NewsletterMetadata, NewsletterRole, NewsletterState, NewsletterVerification};

fn newsletter(id: &str) -> Result<Jid> {
    let jid = validate::jid(id)?;
    ensure!(jid.is_newsletter(), "newsletter_id must end in @newsletter");
    Ok(jid)
}

fn role(value: Option<NewsletterRole>) -> Option<String> {
    value.map(|role| {
        match role {
            NewsletterRole::Owner => "owner",
            NewsletterRole::Admin => "admin",
            NewsletterRole::Subscriber => "subscriber",
            NewsletterRole::Guest => "guest",
            _ => "unknown",
        }
        .to_string()
    })
}

#[derive(Serialize, JsonSchema)]
pub struct Metadata {
    pub newsletter_id: String,
    pub name: String,
    pub description: Option<String>,
    pub subscriber_count: u64,
    pub verification: String,
    pub state: String,
    pub picture_url: Option<String>,
    pub preview_url: Option<String>,
    pub invite_code: Option<String>,
    pub role: Option<String>,
    pub creation_time: Option<u64>,
    pub muted: Option<bool>,
    pub follower_activity_muted: Option<bool>,
}

impl From<NewsletterMetadata> for Metadata {
    fn from(v: NewsletterMetadata) -> Self {
        Self {
            newsletter_id: v.jid.to_string(),
            name: v.name,
            description: v.description,
            subscriber_count: v.subscriber_count,
            verification: match v.verification {
                NewsletterVerification::Verified => "verified".into(),
                NewsletterVerification::Unverified => "unverified".into(),
                NewsletterVerification::Other(s) => s,
                _ => "unknown".into(),
            },
            state: match v.state {
                NewsletterState::Active => "active".into(),
                NewsletterState::Suspended => "suspended".into(),
                NewsletterState::Geosuspended => "geosuspended".into(),
                NewsletterState::Other(s) => s,
                _ => "unknown".into(),
            },
            picture_url: v.picture_url,
            preview_url: v.preview_url,
            invite_code: v.invite_code,
            role: role(v.role),
            creation_time: v.creation_time,
            muted: v.muted,
            follower_activity_muted: v.follower_activity_muted,
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewsletterRequest {
    pub session_id: String,
    pub newsletter_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub session_id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateRequest {
    pub session_id: String,
    pub newsletter_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PictureRequest {
    pub session_id: String,
    pub newsletter_id: String,
    pub jpeg_base64: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UserRequest {
    pub session_id: String,
    pub newsletter_id: String,
    pub user_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CountRequest {
    pub session_id: String,
    pub newsletter_id: String,
    pub count: u32,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HistoryRequest {
    pub session_id: String,
    pub newsletter_id: String,
    pub count: u32,
    pub before_server_id: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MuteRequest {
    pub session_id: String,
    pub newsletter_id: String,
    pub muted: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InviteRequest {
    pub session_id: String,
    pub invite_code: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReactionRequest {
    pub session_id: String,
    pub newsletter_id: String,
    pub server_id: u64,
    pub emoji: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoteRequest {
    pub session_id: String,
    pub newsletter_id: String,
    pub server_id: u64,
    /// Original option strings; an empty list clears the vote.
    pub option_names: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditRequest {
    pub session_id: String,
    pub newsletter_id: String,
    pub message_id: String,
    pub text: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeRequest {
    pub session_id: String,
    pub newsletter_id: String,
    pub message_id: String,
}

#[derive(Serialize, JsonSchema)]
pub struct Subscription {
    pub duration_seconds: u64,
}

#[derive(Serialize, JsonSchema)]
pub struct Stanza {
    pub stanza_id: String,
}

#[derive(Serialize, JsonSchema)]
pub struct AdminProfile {
    pub id: Option<String>,
    pub name: String,
    pub picture_id: Option<String>,
    pub picture_direct_path: Option<String>,
}

impl From<whatsapp_rust::NewsletterAdminProfile> for AdminProfile {
    fn from(v: whatsapp_rust::NewsletterAdminProfile) -> Self {
        Self {
            id: v.id,
            name: v.name,
            picture_id: v.picture_id,
            picture_direct_path: v.picture_direct_path,
        }
    }
}

#[derive(Serialize, JsonSchema)]
pub struct AdminInfo {
    pub admin_count: Option<u32>,
    pub admin_profile: Option<AdminProfile>,
    pub admin_profiles_enabled: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
pub struct Follower {
    pub jid: String,
    pub phone_jid: Option<String>,
    pub display_name: Option<String>,
    pub username: Option<String>,
    pub role: Option<String>,
    pub follow_time: Option<u64>,
    pub admin_profile: Option<AdminProfile>,
}

#[derive(Serialize, JsonSchema)]
pub struct ReactionCount {
    pub emoji: String,
    pub count: u64,
}

#[derive(Serialize, JsonSchema)]
pub struct VoteCount {
    pub option_hash_base64: String,
    pub count: u64,
}

#[derive(Serialize, JsonSchema)]
pub struct HistoryMessage {
    pub message_id: String,
    pub server_id: u64,
    pub timestamp: u64,
    pub message_type: String,
    pub is_sender: bool,
    /// Native protobuf JSON content; absent on revoked messages.
    pub content: Option<serde_json::Value>,
    pub reactions: Vec<ReactionCount>,
    pub votes: Vec<VoteCount>,
    pub forwards_count: Option<u64>,
    pub views_count: Option<u64>,
    pub responses_count: Option<u64>,
    pub original_timestamp: Option<u64>,
    pub last_edit_timestamp_ms: Option<u64>,
    pub media_type: Option<String>,
    pub poll_type: Option<String>,
    pub admin_profile: Option<AdminProfile>,
}

#[derive(Serialize, JsonSchema)]
pub struct OwnReaction {
    pub emoji: String,
    pub timestamp: u64,
}

#[derive(Serialize, JsonSchema)]
pub struct OwnVote {
    pub timestamp: u64,
    pub option_hashes_base64: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct OwnAddons {
    pub server_id: u64,
    pub reaction: Option<OwnReaction>,
    pub poll_vote: Option<OwnVote>,
}

fn count(count: u32) -> Result<()> {
    ensure!(
        (1..=100).contains(&count),
        "count must be between 1 and 100"
    );
    Ok(())
}

pub(super) fn register(registry: &mut Registry) {
    registry.add::<SessionRequest, Vec<Metadata>, _, _>(
        "whatsapp::newsletters::list",
        "List subscribed channels.",
        |m, r| async move {
            Ok(m.client(&r.session_id)
                .await?
                .newsletter()
                .list_subscribed()
                .await?
                .into_iter()
                .map(Into::into)
                .collect())
        },
    );
    macro_rules! metadata {
        ($id:literal, $description:literal, $method:ident) => {
            registry.add::<NewsletterRequest, Metadata, _, _>(
                $id,
                $description,
                |m, r| async move {
                    let jid = newsletter(&r.newsletter_id)?;
                    Ok(m.client(&r.session_id)
                        .await?
                        .newsletter()
                        .$method(&jid)
                        .await?
                        .into())
                },
            );
        };
    }
    metadata!(
        "whatsapp::newsletters::get",
        "Fetch channel metadata.",
        get_metadata
    );
    metadata!(
        "whatsapp::newsletters::join",
        "Subscribe to a channel.",
        join
    );
    metadata!(
        "whatsapp::newsletters::remove_picture",
        "Remove a channel picture.",
        remove_picture
    );
    macro_rules! action {
        ($id:literal, $description:literal, $method:ident) => {
            registry.add::<NewsletterRequest, Applied, _, _>(
                $id,
                $description,
                |m, r| async move {
                    let jid = newsletter(&r.newsletter_id)?;
                    m.client(&r.session_id)
                        .await?
                        .newsletter()
                        .$method(&jid)
                        .await?;
                    Ok(applied())
                },
            );
        };
    }
    action!(
        "whatsapp::newsletters::leave",
        "Unsubscribe from a channel.",
        leave
    );
    action!(
        "whatsapp::newsletters::delete",
        "Delete a channel owned by this account.",
        delete
    );
    registry.add::<CreateRequest, Metadata, _, _>(
        "whatsapp::newsletters::create",
        "Create a channel with a name and optional description.",
        |m, r| async move {
            validate::text(&r.name, "name", 100)?;
            optional_text(r.description.as_deref(), "description", 2048)?;
            Ok(m.client(&r.session_id)
                .await?
                .newsletter()
                .create(&r.name, r.description.as_deref())
                .await?
                .into())
        },
    );
    registry.add::<UpdateRequest, Metadata, _, _>(
        "whatsapp::newsletters::update",
        "Update a channel's name or description.",
        |m, r| async move {
            let jid = newsletter(&r.newsletter_id)?;
            ensure!(
                r.name.is_some() || r.description.is_some(),
                "provide name or description"
            );
            if let Some(name) = &r.name {
                validate::text(name, "name", 100)?;
            }
            optional_text(r.description.as_deref(), "description", 2048)?;
            Ok(m.client(&r.session_id)
                .await?
                .newsletter()
                .update(&jid, r.name.as_deref(), r.description.as_deref())
                .await?
                .into())
        },
    );
    registry.add::<PictureRequest, Metadata, _, _>(
        "whatsapp::newsletters::set_picture",
        "Set a channel picture from bounded JPEG data.",
        |m, r| async move {
            let jid = newsletter(&r.newsletter_id)?;
            let jpeg = validate::media(
                &r.jpeg_base64,
                m.config().max_media_bytes.min(5 * 1024 * 1024),
            )?;
            ensure!(
                jpeg.starts_with(&[0xff, 0xd8, 0xff]),
                "picture must be encoded JPEG data"
            );
            Ok(m.client(&r.session_id)
                .await?
                .newsletter()
                .set_picture(&jid, &jpeg)
                .await?
                .into())
        },
    );
    macro_rules! user_action {
        ($id:literal, $description:literal, $method:ident) => {
            registry.add::<UserRequest, Applied, _, _>($id, $description, |m, r| async move {
                let jid = newsletter(&r.newsletter_id)?;
                let user = validate::jid(&r.user_id)?;
                ensure!(
                    user.is_pn() || user.is_lid(),
                    "user_id must identify a person"
                );
                m.client(&r.session_id)
                    .await?
                    .newsletter()
                    .$method(&jid, &user)
                    .await?;
                Ok(applied())
            });
        };
    }
    user_action!(
        "whatsapp::newsletters::change_owner",
        "Transfer ownership of a channel.",
        change_owner
    );
    user_action!(
        "whatsapp::newsletters::demote_admin",
        "Remove a channel administrator's role.",
        demote_admin
    );
    registry.add::<NewsletterRequest, AdminInfo, _, _>(
        "whatsapp::newsletters::admin_info",
        "Get channel administrator information visible to this account.",
        |m, r| async move {
            let jid = newsletter(&r.newsletter_id)?;
            let info = m
                .client(&r.session_id)
                .await?
                .newsletter()
                .get_admin_info(&jid)
                .await?;
            Ok(AdminInfo {
                admin_count: info.admin_count,
                admin_profile: info.admin_profile.map(Into::into),
                admin_profiles_enabled: info.admin_profiles_enabled,
            })
        },
    );
    registry.add::<CountRequest, Vec<Follower>, _, _>(
        "whatsapp::newsletters::followers",
        "Fetch up to 100 channel followers visible to this account.",
        |m, r| async move {
            let jid = newsletter(&r.newsletter_id)?;
            count(r.count)?;
            Ok(m.client(&r.session_id)
                .await?
                .newsletter()
                .get_followers(&jid, r.count)
                .await?
                .into_iter()
                .map(|v| Follower {
                    jid: v.jid.to_string(),
                    phone_jid: v.phone_jid.map(|j| j.to_string()),
                    display_name: v.display_name,
                    username: v.username,
                    role: role(v.role),
                    follow_time: v.follow_time,
                    admin_profile: v.admin_profile.map(Into::into),
                })
                .collect())
        },
    );
    macro_rules! mute {
        ($id:literal, $description:literal, $method:ident) => {
            registry.add::<MuteRequest, Applied, _, _>($id, $description, |m, r| async move {
                let jid = newsletter(&r.newsletter_id)?;
                m.client(&r.session_id)
                    .await?
                    .newsletter()
                    .$method(&jid, r.muted)
                    .await?;
                Ok(applied())
            });
        };
    }
    mute!(
        "whatsapp::newsletters::mute_updates",
        "Mute or unmute channel updates for this account.",
        set_admin_mute
    );
    mute!(
        "whatsapp::newsletters::mute_follower_activity",
        "Mute or unmute follower activity notifications for a channel administrator.",
        set_follower_mute
    );
    registry.add::<InviteRequest, Metadata, _, _>(
        "whatsapp::newsletters::get_by_invite",
        "Look up channel metadata using its invite code.",
        |m, r| async move {
            validate::text(&r.invite_code, "invite_code", 256)?;
            Ok(m.client(&r.session_id)
                .await?
                .newsletter()
                .get_metadata_by_invite(&r.invite_code)
                .await?
                .into())
        },
    );
    registry.add::<NewsletterRequest, Subscription, _, _>(
        "whatsapp::newsletters::subscribe_updates",
        "Subscribe to channel counters and return the server's renewal interval.",
        |m, r| async move {
            let jid = newsletter(&r.newsletter_id)?;
            Ok(Subscription {
                duration_seconds: m
                    .client(&r.session_id)
                    .await?
                    .newsletter()
                    .subscribe_live_updates(jid)
                    .await?,
            })
        },
    );
    registry.add::<ReactionRequest, Stanza, _, _>(
        "whatsapp::newsletters::react",
        "Set or remove a channel reaction using its server-assigned message id.",
        |m, r| async move {
            let jid = newsletter(&r.newsletter_id)?;
            ensure!(r.server_id > 0, "server_id must be positive");
            optional_text(Some(&r.emoji), "emoji", 64)?;
            Ok(Stanza {
                stanza_id: m
                    .client(&r.session_id)
                    .await?
                    .newsletter()
                    .send_reaction(&jid, r.server_id, &r.emoji)
                    .await?,
            })
        },
    );
    registry.add::<VoteRequest, Stanza, _, _>(
        "whatsapp::newsletters::vote_poll",
        "Vote for original channel poll options, or clear the selection with an empty list.",
        |m, r| async move {
            let jid = newsletter(&r.newsletter_id)?;
            ensure!(r.server_id > 0, "server_id must be positive");
            ensure!(
                r.option_names.len() <= 12,
                "at most 12 option_names are allowed"
            );
            let mut hashes = Vec::with_capacity(r.option_names.len());
            for option in &r.option_names {
                validate::text(option, "option_name", 100)?;
                hashes.push(wacore::poll::compute_option_hash(option));
            }
            Ok(Stanza {
                stanza_id: m
                    .client(&r.session_id)
                    .await?
                    .newsletter()
                    .send_poll_vote(&jid, r.server_id, &hashes)
                    .await?,
            })
        },
    );
    registry.add::<EditRequest, Applied, _, _>(
        "whatsapp::newsletters::edit",
        "Edit a channel text message using its stanza message id, not server id.",
        |m, r| async move {
            let jid = newsletter(&r.newsletter_id)?;
            message_id(&r.message_id)?;
            let content = text_message(&r.text)?;
            m.client(&r.session_id)
                .await?
                .newsletter()
                .edit_message(&jid, r.message_id, content)
                .await?;
            Ok(applied())
        },
    );
    registry.add::<RevokeRequest, Applied, _, _>(
        "whatsapp::newsletters::revoke",
        "Revoke a channel message using its stanza message id, not server id.",
        |m, r| async move {
            let jid = newsletter(&r.newsletter_id)?;
            message_id(&r.message_id)?;
            m.client(&r.session_id)
                .await?
                .newsletter()
                .revoke_message(&jid, r.message_id)
                .await?;
            Ok(applied())
        },
    );
    registry.add::<HistoryRequest, Vec<HistoryMessage>, _, _>("whatsapp::newsletters::messages", "Fetch a bounded page of channel messages with typed envelope metadata and protobuf JSON content.", |m, r| async move {
        let jid = newsletter(&r.newsletter_id)?;
        count(r.count)?;
        m.client(&r.session_id).await?.newsletter().get_messages(jid, r.count, r.before_server_id).await?.into_iter().map(|v| {
            Ok(HistoryMessage {
                message_id: v.message_id, server_id: v.server_id, timestamp: v.timestamp,
                message_type: v.message_type.as_str().to_string(), is_sender: v.is_sender,
                content: v.message.map(serde_json::to_value).transpose()?,
                reactions: v.reactions.into_iter().map(|r| ReactionCount { emoji: r.code, count: r.count }).collect(),
                votes: v.votes.into_iter().map(|v| VoteCount { option_hash_base64: BASE64.encode(v.option_hash), count: v.count }).collect(),
                forwards_count: v.forwards_count, views_count: v.views_count, responses_count: v.responses_count,
                original_timestamp: v.original_timestamp, last_edit_timestamp_ms: v.last_edit_timestamp_ms,
                media_type: v.media_type.map(|t| t.as_str().to_string()), poll_type: v.poll_type_raw,
                admin_profile: v.admin_profile.map(Into::into),
            })
        }).collect()
    });
    registry.add::<CountRequest, Vec<OwnAddons>, _, _>(
        "whatsapp::newsletters::my_addons",
        "Read this account's reactions and poll selections on a bounded set of channel messages.",
        |m, r| async move {
            let jid = newsletter(&r.newsletter_id)?;
            count(r.count)?;
            Ok(m.client(&r.session_id)
                .await?
                .newsletter()
                .get_my_addons(&jid, r.count)
                .await?
                .into_iter()
                .map(|v| OwnAddons {
                    server_id: v.server_id,
                    reaction: v.reaction.map(|r| OwnReaction {
                        emoji: r.code,
                        timestamp: r.timestamp,
                    }),
                    poll_vote: v.poll_vote.map(|v| OwnVote {
                        timestamp: v.timestamp,
                        option_hashes_base64: v
                            .option_hashes
                            .into_iter()
                            .map(|h| BASE64.encode(h))
                            .collect(),
                    }),
                })
                .collect())
        },
    );
}
