//! Native group, contact, profile, privacy and community functions.

use std::collections::HashSet;

use anyhow::{Result, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use whatsapp_rust::{Jid, Server, features::*, privacy_settings::*};

use crate::{registry::Registry, validate};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SessionRequest {
    session_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupRequest {
    pub session_id: String,
    pub group_id: String,
}

impl GroupRequest {
    pub fn group(&self) -> Result<Jid> {
        group_jid(&self.group_id)
    }
}

fn group_jid(value: &str) -> Result<Jid> {
    let jid = validate::jid(value)?;
    ensure!(
        jid.server == Server::Group,
        "group_id must be a group JID ending in @g.us"
    );
    Ok(jid)
}

fn contact_jid(value: &str) -> Result<Jid> {
    let jid = validate::jid(value)?;
    ensure!(
        matches!(jid.server, Server::Pn | Server::Lid),
        "contact must be a phone number, PN JID or LID JID"
    );
    Ok(jid)
}

fn addressbook_jid(value: &str) -> Result<Jid> {
    let jid = contact_jid(value)?;
    ensure!(
        jid.server == Server::Pn && jid.device == 0,
        "address-book mutations require a bare phone-number JID"
    );
    Ok(jid)
}

fn subject(value: String) -> Result<GroupSubject> {
    ensure!(!value.trim().is_empty(), "group name must not be blank");
    GroupSubject::new(value)
}

fn jpeg(value: &str, max_bytes: usize) -> Result<Vec<u8>> {
    let bytes = validate::media(value, max_bytes)?;
    ensure!(
        bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "picture must be encoded JPEG data"
    );
    Ok(bytes)
}

fn contacts(values: &[String], allow_empty: bool) -> Result<Vec<Jid>> {
    ensure!(
        allow_empty || !values.is_empty(),
        "participants must not be empty"
    );
    ensure!(
        values.len() <= 1024,
        "at most 1024 participants are allowed"
    );
    let mut seen = HashSet::new();
    values
        .iter()
        .map(|value| {
            let jid = contact_jid(value)?;
            ensure!(seen.insert(jid.to_string()), "duplicate participant");
            Ok(jid)
        })
        .collect()
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticipantsRequest {
    pub session_id: String,
    pub group_id: String,
    pub participants: Vec<String>,
}

impl ParticipantsRequest {
    pub fn targets(&self) -> Result<Vec<Jid>> {
        contacts(&self.participants, false)
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    session_id: String,
    subject: String,
    #[serde(default)]
    participants: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SubjectRequest {
    session_id: String,
    group_id: String,
    subject: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DescriptionRequest {
    session_id: String,
    group_id: String,
    description: Option<String>,
    /// An existing description version; omit to fetch the current version.
    previous_description_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InviteRequest {
    session_id: String,
    code: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GroupBooleanRequest {
    session_id: String,
    group_id: String,
    enabled: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GroupDurationRequest {
    session_id: String,
    group_id: String,
    expiration_seconds: u32,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GroupMemberModeRequest {
    session_id: String,
    group_id: String,
    /// False permits admins only; true permits all members.
    all_members: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PictureRequest {
    session_id: String,
    jid: String,
    #[serde(default)]
    preview: bool,
    existing_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GroupPictureRequest {
    session_id: String,
    group_id: String,
    jpeg_base64: String,
}

#[derive(Serialize, JsonSchema)]
struct PictureId {
    id: String,
}

#[derive(Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PictureView {
    Found {
        id: String,
        url: String,
        direct_path: Option<String>,
        hash: Option<String>,
    },
    Unchanged,
    NotFound,
    NotAuthorized,
    RateOverlimit,
    Unknown,
}

impl From<ProfilePictureLookup> for PictureView {
    fn from(value: ProfilePictureLookup) -> Self {
        match value {
            ProfilePictureLookup::Found(p) => Self::Found {
                id: p.id,
                url: p.url,
                direct_path: p.direct_path,
                hash: p.hash,
            },
            ProfilePictureLookup::Unchanged => Self::Unchanged,
            ProfilePictureLookup::NotFound => Self::NotFound,
            ProfilePictureLookup::NotAuthorized => Self::NotAuthorized,
            ProfilePictureLookup::RateOverlimit => Self::RateOverlimit,
            _ => Self::Unknown,
        }
    }
}

#[derive(Serialize, JsonSchema)]
pub struct ParticipantView {
    jid: String,
    phone_number: Option<String>,
    lid: Option<String>,
    username: Option<String>,
    participant_type: String,
}

impl From<GroupParticipant> for ParticipantView {
    fn from(p: GroupParticipant) -> Self {
        Self {
            jid: p.jid.to_string(),
            phone_number: p.phone_number.map(|v| v.to_string()),
            lid: p.lid.map(|v| v.to_string()),
            username: p.username.map(|v| v.to_string()),
            participant_type: p.participant_type.as_str().to_owned(),
        }
    }
}

#[derive(Serialize, JsonSchema)]
pub struct GroupView {
    id: String,
    subject: Option<String>,
    description: Option<String>,
    description_id: Option<String>,
    creator: Option<String>,
    creation_time: Option<u64>,
    participants: Vec<ParticipantView>,
    locked: bool,
    announcement: bool,
    ephemeral_expiration_seconds: Option<u32>,
    membership_approval: bool,
    member_add_mode: Option<String>,
    member_link_mode: Option<String>,
    member_share_history_mode: Option<String>,
    participant_count: Option<u32>,
    is_community: bool,
    parent_group_id: Option<String>,
    is_announcement_subgroup: bool,
    is_general_chat: bool,
    no_frequently_forwarded: bool,
    allow_admin_reports: bool,
    group_history_enabled: bool,
    limit_sharing_enabled: bool,
}

impl From<GroupMetadata> for GroupView {
    fn from(m: GroupMetadata) -> Self {
        Self {
            id: m.id.to_string(),
            subject: m.subject,
            description: m.description,
            description_id: m.description_id,
            creator: m.creator.map(|v| v.to_string()),
            creation_time: m.creation_time,
            participants: m.participants.into_iter().map(Into::into).collect(),
            locked: m.is_locked,
            announcement: m.is_announcement,
            ephemeral_expiration_seconds: m.ephemeral.and_then(|v| v.expiration),
            membership_approval: m.membership_approval,
            member_add_mode: m.member_add_mode.map(|v| v.as_str().to_owned()),
            member_link_mode: m.member_link_mode.map(|v| v.as_str().to_owned()),
            member_share_history_mode: m.member_share_history_mode.map(|v| v.as_str().to_owned()),
            participant_count: m.size,
            is_community: m.is_parent_group,
            parent_group_id: m.parent_group_jid.map(|v| v.to_string()),
            is_announcement_subgroup: m.is_default_sub_group,
            is_general_chat: m.is_general_chat,
            no_frequently_forwarded: m.no_frequently_forwarded,
            allow_admin_reports: m.allow_admin_reports,
            group_history_enabled: m.has_group_history,
            limit_sharing_enabled: m.is_limit_sharing_enabled,
        }
    }
}

#[derive(Serialize, JsonSchema)]
struct OverviewView {
    id: String,
    subject: Option<String>,
    participant_count: Option<u32>,
    is_community: bool,
    parent_group_id: Option<String>,
}

impl From<GroupOverview> for OverviewView {
    fn from(m: GroupOverview) -> Self {
        let is_community = m.is_parent_group();
        let parent_group_id = m.parent_group_jid().map(|v| v.to_string());
        Self {
            id: m.id.to_string(),
            subject: m.subject,
            participant_count: m.participant_count,
            is_community,
            parent_group_id,
        }
    }
}

#[derive(Serialize, JsonSchema)]
struct ParticipantChange {
    jid: String,
    status: Option<String>,
    error: Option<String>,
    phone_number: Option<String>,
    username: Option<String>,
    invite_code: Option<String>,
    invite_expiration: Option<u64>,
}

impl From<ParticipantChangeResponse> for ParticipantChange {
    fn from(v: ParticipantChangeResponse) -> Self {
        Self {
            jid: v.jid.to_string(),
            status: v.status,
            error: v.error,
            phone_number: v.phone_number.map(|v| v.to_string()),
            username: v.username,
            invite_code: v.add_request.as_ref().map(|v| v.code.clone()),
            invite_expiration: v.add_request.map(|v| v.expiration),
        }
    }
}

#[derive(Serialize, JsonSchema)]
struct JoinView {
    group_id: String,
    pending_approval: bool,
}

#[derive(Serialize, JsonSchema)]
struct InviteView {
    url: String,
}

#[derive(Serialize, JsonSchema)]
struct MembershipView {
    jid: String,
    request_time: Option<u64>,
}

pub fn register(registry: &mut Registry) {
    register_groups(registry);
    register_contacts(registry);
    register_profile_privacy(registry);
    register_communities(registry);
}

fn register_groups(registry: &mut Registry) {
    registry.add::<SessionRequest, Vec<OverviewView>, _, _>(
        "whatsapp::groups::list",
        "List participating WhatsApp groups",
        |manager, req| async move {
            let client = manager.client(&req.session_id).await?;
            Ok(client
                .groups()
                .list_participating()
                .await?
                .into_iter()
                .map(Into::into)
                .collect())
        },
    );
    registry.add::<GroupRequest, GroupView, _, _>(
        "whatsapp::groups::get",
        "Fetch group metadata and settings",
        |manager, req| async move {
            let jid = req.group()?;
            let client = manager.client(&req.session_id).await?;
            Ok(client.groups().fetch_metadata(&jid).await?.into())
        },
    );
    registry.add::<CreateRequest, GroupView, _, _>(
        "whatsapp::groups::create",
        "Create a WhatsApp group",
        |manager, req| async move {
            let subject = subject(req.subject)?;
            let targets = contacts(&req.participants, true)?;
            let options = GroupCreateOptions::new(subject.into_string()).with_participants(
                targets
                    .into_iter()
                    .map(GroupParticipantOptions::new)
                    .collect(),
            );
            let client = manager.client(&req.session_id).await?;
            Ok(client.groups().create_group(options).await?.metadata.into())
        },
    );
    registry.add::<SubjectRequest, (), _, _>(
        "whatsapp::groups::set_subject",
        "Set a group's subject",
        |manager, req| async move {
            let jid = group_jid(&req.group_id)?;
            let subject = subject(req.subject)?;
            manager
                .client(&req.session_id)
                .await?
                .groups()
                .set_subject(jid, subject)
                .await?;
            Ok(())
        },
    );
    registry.add::<DescriptionRequest, (), _, _>(
        "whatsapp::groups::set_description",
        "Set or remove a group description with version checking",
        |manager, req| async move {
            let jid = group_jid(&req.group_id)?;
            let description = req.description.map(GroupDescription::new).transpose()?;
            let previous = req
                .previous_description_id
                .as_deref()
                .map(PreviousDescription::Id)
                .unwrap_or(PreviousDescription::Resolve);
            manager
                .client(&req.session_id)
                .await?
                .groups()
                .set_description(jid, description, previous)
                .await?;
            Ok(())
        },
    );
    registry.add::<GroupRequest, (), _, _>(
        "whatsapp::groups::leave",
        "Leave a group",
        |manager, req| async move {
            let jid = req.group()?;
            manager
                .client(&req.session_id)
                .await?
                .groups()
                .leave(jid)
                .await?;
            Ok(())
        },
    );
    macro_rules! participant_operation {
        ($id:literal, $description:literal, $method:ident) => {
            registry.add::<ParticipantsRequest, Vec<ParticipantChange>, _, _>(
                $id,
                $description,
                |manager, req| async move {
                    let jid = group_jid(&req.group_id)?;
                    let targets = req.targets()?;
                    let client = manager.client(&req.session_id).await?;
                    Ok(client
                        .groups()
                        .$method(jid, &targets)
                        .await?
                        .into_iter()
                        .map(Into::into)
                        .collect())
                },
            );
        };
    }
    participant_operation!(
        "whatsapp::groups::add_participants",
        "Add participants; preserves individual failures",
        add_participants
    );
    participant_operation!(
        "whatsapp::groups::remove_participants",
        "Remove group participants",
        remove_participants
    );
    participant_operation!(
        "whatsapp::groups::remove_linked_participants",
        "Remove participants including linked groups",
        remove_participants_including_linked_groups
    );
    participant_operation!(
        "whatsapp::groups::promote_participants",
        "Promote participants to admin",
        promote_participants
    );
    participant_operation!(
        "whatsapp::groups::demote_participants",
        "Demote group admins",
        demote_participants
    );
    participant_operation!(
        "whatsapp::groups::approve_requests",
        "Approve group membership requests",
        approve_membership_requests
    );
    participant_operation!(
        "whatsapp::groups::reject_requests",
        "Reject group membership requests",
        reject_membership_requests
    );
    participant_operation!(
        "whatsapp::groups::cancel_requests",
        "Cancel pending group membership requests",
        cancel_membership_requests
    );
    participant_operation!(
        "whatsapp::groups::revoke_request_codes",
        "Revoke participant request codes",
        revoke_request_code
    );
    registry.add::<GroupRequest, Vec<MembershipView>, _, _>(
        "whatsapp::groups::membership_requests",
        "List pending group membership requests",
        |manager, req| async move {
            let jid = req.group()?;
            let client = manager.client(&req.session_id).await?;
            Ok(client
                .groups()
                .get_membership_requests(jid)
                .await?
                .into_iter()
                .map(|v| MembershipView {
                    jid: v.jid.to_string(),
                    request_time: v.request_time,
                })
                .collect())
        },
    );
    registry.add::<InviteRequest, GroupView, _, _>(
        "whatsapp::groups::invite_info",
        "Fetch group metadata for an invite code",
        |manager, req| async move {
            validate::text(&req.code, "code", 256)?;
            Ok(manager
                .client(&req.session_id)
                .await?
                .groups()
                .get_invite_info(&req.code)
                .await?
                .into())
        },
    );
    registry.add::<InviteRequest, JoinView, _, _>(
        "whatsapp::groups::join",
        "Join using an invite code; may require approval",
        |manager, req| async move {
            validate::text(&req.code, "code", 256)?;
            let result = manager
                .client(&req.session_id)
                .await?
                .groups()
                .join_with_invite_code(&req.code)
                .await?;
            Ok(JoinView {
                group_id: result.group_jid().to_string(),
                pending_approval: matches!(result, JoinGroupResult::PendingApproval(_)),
            })
        },
    );
    for (id, reset) in [
        ("whatsapp::groups::invite_link", false),
        ("whatsapp::groups::revoke_invite", true),
    ] {
        registry.add::<GroupRequest, InviteView, _, _>(
            id,
            "Get or reset the group's invite link",
            move |manager, req| async move {
                let jid = req.group()?;
                Ok(InviteView {
                    url: manager
                        .client(&req.session_id)
                        .await?
                        .groups()
                        .get_invite_link(jid, reset)
                        .await?,
                })
            },
        );
    }
    macro_rules! boolean_setting {
        ($id:literal, $description:literal, $method:ident) => {
            registry.add::<GroupBooleanRequest, (), _, _>(
                $id,
                $description,
                |manager, req| async move {
                    let jid = group_jid(&req.group_id)?;
                    manager
                        .client(&req.session_id)
                        .await?
                        .groups()
                        .$method(jid, req.enabled)
                        .await?;
                    Ok(())
                },
            );
        };
    }
    boolean_setting!(
        "whatsapp::groups::set_locked",
        "Restrict group info editing to admins",
        set_locked
    );
    boolean_setting!(
        "whatsapp::groups::set_announcement",
        "Restrict group messages to admins",
        set_announce
    );
    boolean_setting!(
        "whatsapp::groups::set_history",
        "Enable or disable group history sharing",
        set_group_history
    );
    boolean_setting!(
        "whatsapp::groups::set_forwarding_restriction",
        "Restrict frequently forwarded messages",
        set_no_frequently_forwarded
    );
    boolean_setting!(
        "whatsapp::groups::set_admin_reports",
        "Allow or disable reports to group admins",
        set_allow_admin_reports
    );
    registry.add::<GroupBooleanRequest, (), _, _>(
        "whatsapp::groups::set_limit_sharing",
        "Set group advanced sharing privacy",
        |manager, req| async move {
            let jid = group_jid(&req.group_id)?;
            manager
                .client(&req.session_id)
                .await?
                .groups()
                .set_limit_sharing(&jid, req.enabled)
                .await?;
            Ok(())
        },
    );
    registry.add::<GroupBooleanRequest, (), _, _>(
        "whatsapp::groups::set_membership_approval",
        "Require approval for new members",
        |manager, req| async move {
            let jid = group_jid(&req.group_id)?;
            let mode = if req.enabled {
                MembershipApprovalMode::On
            } else {
                MembershipApprovalMode::Off
            };
            manager
                .client(&req.session_id)
                .await?
                .groups()
                .set_membership_approval(jid, mode)
                .await?;
            Ok(())
        },
    );
    registry.add::<GroupDurationRequest, (), _, _>(
        "whatsapp::groups::set_ephemeral",
        "Set group disappearing-message duration in seconds",
        |manager, req| async move {
            let jid = group_jid(&req.group_id)?;
            manager
                .client(&req.session_id)
                .await?
                .groups()
                .set_ephemeral(jid, req.expiration_seconds)
                .await?;
            Ok(())
        },
    );
    registry.add::<GroupMemberModeRequest, (), _, _>(
        "whatsapp::groups::set_member_add_mode",
        "Choose whether all members may add people",
        |manager, req| async move {
            let jid = group_jid(&req.group_id)?;
            let mode = if req.all_members {
                MemberAddMode::AllMemberAdd
            } else {
                MemberAddMode::AdminAdd
            };
            manager
                .client(&req.session_id)
                .await?
                .groups()
                .set_member_add_mode(jid, mode)
                .await?;
            Ok(())
        },
    );
    registry.add::<GroupMemberModeRequest, (), _, _>(
        "whatsapp::groups::set_member_link_mode",
        "Choose who may use group invite links",
        |manager, req| async move {
            let jid = group_jid(&req.group_id)?;
            let mode = if req.all_members {
                MemberLinkMode::AllMemberLink
            } else {
                MemberLinkMode::AdminLink
            };
            manager
                .client(&req.session_id)
                .await?
                .groups()
                .set_member_link_mode(&jid, mode)
                .await?;
            Ok(())
        },
    );
    registry.add::<GroupMemberModeRequest, (), _, _>(
        "whatsapp::groups::set_member_history_mode",
        "Choose who may share group history",
        |manager, req| async move {
            let jid = group_jid(&req.group_id)?;
            let mode = if req.all_members {
                MemberShareHistoryMode::AllMemberShare
            } else {
                MemberShareHistoryMode::AdminShare
            };
            manager
                .client(&req.session_id)
                .await?
                .groups()
                .set_member_share_history_mode(&jid, mode)
                .await?;
            Ok(())
        },
    );
    registry.add::<PictureRequest, PictureView, _, _>(
        "whatsapp::groups::picture",
        "Look up a group profile picture and preserve privacy/rate-limit outcomes",
        |manager, req| async move {
            let jid = group_jid(&req.jid)?;
            Ok(manager
                .client(&req.session_id)
                .await?
                .groups()
                .lookup_profile_picture(&jid, req.preview, req.existing_id.as_deref())
                .await?
                .into())
        },
    );
    registry.add::<GroupPictureRequest, PictureId, _, _>(
        "whatsapp::groups::set_picture",
        "Set a group picture from prepared JPEG bytes",
        |manager, req| async move {
            let jid = group_jid(&req.group_id)?;
            let bytes = jpeg(&req.jpeg_base64, manager.config().max_media_bytes)?;
            Ok(PictureId {
                id: manager
                    .client(&req.session_id)
                    .await?
                    .groups()
                    .set_profile_picture(jid, bytes)
                    .await?
                    .id,
            })
        },
    );
    registry.add::<GroupRequest, PictureId, _, _>(
        "whatsapp::groups::remove_picture",
        "Remove a group's picture",
        |manager, req| async move {
            let jid = req.group()?;
            Ok(PictureId {
                id: manager
                    .client(&req.session_id)
                    .await?
                    .groups()
                    .remove_profile_picture(jid)
                    .await?
                    .id,
            })
        },
    );
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ContactRequest {
    session_id: String,
    jid: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ContactsRequest {
    session_id: String,
    jids: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SaveContactRequest {
    session_id: String,
    jid: String,
    full_name: Option<String>,
    first_name: Option<String>,
    #[serde(default)]
    save_on_primary_addressbook: bool,
}

#[derive(Serialize, JsonSchema)]
struct SubprotocolErrorView {
    code: Option<u16>,
    text: Option<String>,
    backoff: Option<u32>,
}

impl From<UsyncSubprotocolError> for SubprotocolErrorView {
    fn from(v: UsyncSubprotocolError) -> Self {
        Self {
            code: v.code,
            text: v.text,
            backoff: v.backoff,
        }
    }
}

#[derive(Serialize, JsonSchema)]
struct ContactCheck {
    jid: String,
    lid: Option<String>,
    phone_number: Option<String>,
    registered: bool,
    is_business: bool,
    username: Option<String>,
    contact_error: Option<SubprotocolErrorView>,
    lid_error: Option<SubprotocolErrorView>,
    business_error: Option<SubprotocolErrorView>,
    username_error: Option<SubprotocolErrorView>,
}

impl From<IsOnWhatsAppResult> for ContactCheck {
    fn from(v: IsOnWhatsAppResult) -> Self {
        Self {
            jid: v.jid.to_string(),
            lid: v.lid.map(|v| v.to_string()),
            phone_number: v.pn_jid.map(|v| v.to_string()),
            registered: v.is_registered,
            is_business: v.is_business,
            username: v.username.map(|v| v.to_string()),
            contact_error: v.contact_error.map(Into::into),
            lid_error: v.lid_error.map(Into::into),
            business_error: v.business_error.map(Into::into),
            username_error: v.username_error.map(Into::into),
        }
    }
}

#[derive(Serialize, JsonSchema)]
struct ContactInfo {
    jid: String,
    lid: Option<String>,
    about: Option<String>,
    picture_id: Option<String>,
    is_business: bool,
    username: Option<String>,
    devices: Vec<u16>,
    lid_error: Option<SubprotocolErrorView>,
    status_error: Option<SubprotocolErrorView>,
    picture_error: Option<SubprotocolErrorView>,
    business_error: Option<SubprotocolErrorView>,
    username_error: Option<SubprotocolErrorView>,
    devices_error: Option<SubprotocolErrorView>,
}

impl From<UserInfo> for ContactInfo {
    fn from(v: UserInfo) -> Self {
        Self {
            jid: v.jid.to_string(),
            lid: v.lid.map(|v| v.to_string()),
            about: v.status,
            picture_id: v.picture_id,
            is_business: v.is_business,
            username: v.username.map(|v| v.to_string()),
            devices: v.devices,
            lid_error: v.lid_error.map(Into::into),
            status_error: v.status_error.map(Into::into),
            picture_error: v.picture_error.map(Into::into),
            business_error: v.business_error.map(Into::into),
            username_error: v.username_error.map(Into::into),
            devices_error: v.devices_error.map(Into::into),
        }
    }
}

#[derive(Serialize, JsonSchema)]
struct BlockedView {
    jid: String,
    timestamp: Option<u64>,
}

#[derive(Serialize, JsonSchema)]
struct BlockedStatus {
    blocked: bool,
}

#[derive(Serialize, JsonSchema)]
struct ContactPicture {
    jid: String,
    picture: Option<PictureView>,
    error: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct ResolvedPhone {
    jid: String,
    phone_number: Option<String>,
}

fn register_contacts(registry: &mut Registry) {
    registry.add::<ContactsRequest, Vec<ContactPicture>, _, _>(
        "whatsapp::contacts::pictures",
        "Fetch up to 50 contact photos, retaining per-contact lookup errors",
        |manager, req| async move {
            ensure!(
                req.jids.len() <= 50,
                "at most 50 contact photos per request"
            );
            let jids = contacts(&req.jids, false)?;
            let client = manager.client(&req.session_id).await?;
            let mut pictures = Vec::with_capacity(jids.len());
            for jid in jids {
                let (picture, error) = match client
                    .contacts()
                    .lookup_profile_picture(&jid, false, None)
                    .await
                {
                    Ok(picture) => (Some(picture.into()), None),
                    Err(error) => (None, Some(error.to_string())),
                };
                pictures.push(ContactPicture {
                    jid: jid.to_string(),
                    picture,
                    error,
                });
            }
            Ok(pictures)
        },
    );
    registry.add::<ContactRequest, ResolvedPhone, _, _>(
        "whatsapp::contacts::resolve_phone",
        "Resolve a PN or LID contact to phone digits when a mapping is available",
        |manager, req| async move {
            let jid = contact_jid(&req.jid)?;
            let client = manager.client(&req.session_id).await?;
            let phone_number = if jid.server == Server::Pn {
                Some(jid.user.to_string())
            } else {
                client
                    .contacts()
                    .is_on_whatsapp(std::slice::from_ref(&jid))
                    .await?
                    .into_iter()
                    .find_map(|result| {
                        if let Some(pn) = result.pn_jid.filter(|pn| pn.server == Server::Pn) {
                            Some(pn.user.to_string())
                        } else if result.jid.server == Server::Pn {
                            Some(result.jid.user.to_string())
                        } else {
                            None
                        }
                    })
            };
            Ok(ResolvedPhone {
                jid: jid.to_string(),
                phone_number,
            })
        },
    );
    registry.add::<ContactsRequest, Vec<ContactCheck>, _, _>(
        "whatsapp::contacts::check",
        "Check WhatsApp registration and PN/LID mappings",
        |manager, req| async move {
            let jids = contacts(&req.jids, false)?;
            let client = manager.client(&req.session_id).await?;
            Ok(client
                .contacts()
                .is_on_whatsapp(&jids)
                .await?
                .into_iter()
                .map(Into::into)
                .collect())
        },
    );
    registry.add::<ContactsRequest, Vec<ContactInfo>, _, _>(
        "whatsapp::contacts::info",
        "Fetch server contact metadata for supplied JIDs",
        |manager, req| async move {
            let jids = contacts(&req.jids, false)?;
            let client = manager.client(&req.session_id).await?;
            let mut result: Vec<ContactInfo> = client
                .contacts()
                .get_user_info(&jids)
                .await?
                .into_values()
                .map(Into::into)
                .collect();
            result.sort_by(|a, b| a.jid.cmp(&b.jid));
            Ok(result)
        },
    );
    registry.add::<PictureRequest, PictureView, _, _>(
        "whatsapp::contacts::picture",
        "Look up a profile photo with explicit privacy and rate-limit outcomes",
        |manager, req| async move {
            let jid = contact_jid(&req.jid)?;
            Ok(manager
                .client(&req.session_id)
                .await?
                .contacts()
                .lookup_profile_picture(&jid, req.preview, req.existing_id.as_deref())
                .await?
                .into())
        },
    );
    registry.add::<SaveContactRequest, (), _, _>(
        "whatsapp::contacts::save",
        "Save a contact through WhatsApp app-state sync",
        |manager, req| async move {
            let jid = addressbook_jid(&req.jid)?;
            ensure!(
                req.full_name.is_some() || req.first_name.is_some(),
                "full_name or first_name is required"
            );
            for name in [&req.full_name, &req.first_name].into_iter().flatten() {
                validate::text(name, "contact name", 1024)?;
            }
            manager
                .client(&req.session_id)
                .await?
                .chat_actions()
                .save_contact(
                    &jid,
                    req.full_name,
                    req.first_name,
                    req.save_on_primary_addressbook,
                )
                .await?;
            Ok(())
        },
    );
    registry.add::<ContactRequest, (), _, _>(
        "whatsapp::contacts::remove",
        "Remove a saved contact",
        |manager, req| async move {
            let jid = addressbook_jid(&req.jid)?;
            manager
                .client(&req.session_id)
                .await?
                .chat_actions()
                .remove_contact(&jid)
                .await?;
            Ok(())
        },
    );
    registry.add::<SessionRequest, Vec<BlockedView>, _, _>(
        "whatsapp::contacts::blocked",
        "List blocked contacts",
        |manager, req| async move {
            Ok(manager
                .client(&req.session_id)
                .await?
                .blocking()
                .get_blocklist()
                .await?
                .into_iter()
                .map(|v| BlockedView {
                    jid: v.jid.to_string(),
                    timestamp: v.timestamp,
                })
                .collect())
        },
    );
    registry.add::<ContactRequest, BlockedStatus, _, _>(
        "whatsapp::contacts::is_blocked",
        "Check whether a contact is blocked",
        |manager, req| async move {
            let jid = contact_jid(&req.jid)?;
            Ok(BlockedStatus {
                blocked: manager
                    .client(&req.session_id)
                    .await?
                    .blocking()
                    .is_blocked(&jid)
                    .await?,
            })
        },
    );
    for (id, blocked) in [
        ("whatsapp::contacts::block", true),
        ("whatsapp::contacts::unblock", false),
    ] {
        registry.add::<ContactRequest, (), _, _>(
            id,
            "Update a contact's block state",
            move |manager, req| async move {
                let jid = contact_jid(&req.jid)?;
                let client = manager.client(&req.session_id).await?;
                if blocked {
                    client.blocking().block(&jid).await?;
                } else {
                    client.blocking().unblock(&jid).await?;
                }
                Ok(())
            },
        );
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProfileTextRequest {
    session_id: String,
    text: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProfilePictureRequest {
    session_id: String,
    jpeg_base64: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrivacyRequest {
    pub session_id: String,
    pub category: String,
    pub value: String,
}

impl PrivacyRequest {
    pub fn setting(&self) -> Result<(PrivacyCategory, PrivacyValue)> {
        let category = privacy_category(&self.category)?;
        let value = PrivacyValue::from(self.value.as_str());
        ensure!(
            !matches!(value, PrivacyValue::Other(_)),
            "unknown privacy value"
        );
        ensure!(
            category.is_valid_value(&value),
            "invalid value for privacy category"
        );
        Ok((category, value))
    }
}

fn privacy_category(value: &str) -> Result<PrivacyCategory> {
    let category = PrivacyCategory::from(value);
    ensure!(
        !matches!(category, PrivacyCategory::Other(_)),
        "unknown privacy category"
    );
    Ok(category)
}

#[derive(Serialize, JsonSchema)]
struct PrivacyView {
    category: String,
    value: String,
}

#[derive(Serialize, JsonSchema)]
struct PrivacyUpdateView {
    dhash: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DefaultDurationRequest {
    session_id: String,
    expiration_seconds: u32,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DisallowedRequest {
    session_id: String,
    category: String,
    dhash: String,
    users: Vec<DisallowedUser>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DisallowedUser {
    jid: String,
    phone_number: Option<String>,
    action: DisallowedAction,
}

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum DisallowedAction {
    Add,
    Remove,
}

fn register_profile_privacy(registry: &mut Registry) {
    registry.add::<ProfileTextRequest, (), _, _>(
        "whatsapp::profile::set_name",
        "Set the account's display name",
        |manager, req| async move {
            validate::text(&req.text, "text", 256)?;
            manager
                .client(&req.session_id)
                .await?
                .profile()
                .set_push_name(&req.text)
                .await?;
            Ok(())
        },
    );
    registry.add::<ProfileTextRequest, (), _, _>(
        "whatsapp::profile::set_about",
        "Set profile About text, not a story/status post",
        |manager, req| async move {
            ensure!(req.text.len() <= 4096, "about text exceeds 4096 bytes");
            manager
                .client(&req.session_id)
                .await?
                .profile()
                .set_status_text(&req.text)
                .await?;
            Ok(())
        },
    );
    registry.add::<ProfilePictureRequest, PictureId, _, _>(
        "whatsapp::profile::set_picture",
        "Set the account photo from prepared JPEG bytes",
        |manager, req| async move {
            let bytes = jpeg(&req.jpeg_base64, manager.config().max_media_bytes)?;
            Ok(PictureId {
                id: manager
                    .client(&req.session_id)
                    .await?
                    .profile()
                    .set_profile_picture(bytes)
                    .await?
                    .id,
            })
        },
    );
    registry.add::<SessionRequest, PictureId, _, _>(
        "whatsapp::profile::remove_picture",
        "Remove the account's profile photo",
        |manager, req| async move {
            Ok(PictureId {
                id: manager
                    .client(&req.session_id)
                    .await?
                    .profile()
                    .remove_profile_picture()
                    .await?
                    .id,
            })
        },
    );
    registry.add::<SessionRequest, Vec<PrivacyView>, _, _>(
        "whatsapp::privacy::get",
        "Fetch current account privacy settings",
        |manager, req| async move {
            Ok(manager
                .client(&req.session_id)
                .await?
                .fetch_privacy_settings()
                .await?
                .settings
                .into_iter()
                .map(|v| PrivacyView {
                    category: v.category.as_str().to_owned(),
                    value: v.value.as_str().to_owned(),
                })
                .collect())
        },
    );
    registry.add::<PrivacyRequest, PrivacyUpdateView, _, _>(
        "whatsapp::privacy::set",
        "Set a validated account privacy category",
        |manager, req| async move {
            let (category, value) = req.setting()?;
            Ok(PrivacyUpdateView {
                dhash: manager
                    .client(&req.session_id)
                    .await?
                    .set_privacy_setting(category, value)
                    .await?
                    .dhash,
            })
        },
    );
    registry.add::<DisallowedRequest, PrivacyUpdateView, _, _>(
        "whatsapp::privacy::update_disallowed",
        "Update a privacy exclusion list using its current hash",
        |manager, req| async move {
            let category = privacy_category(&req.category)?;
            ensure!(
                category.supports_disallowed_list(),
                "category does not support exclusion lists"
            );
            ensure!(
                !req.users.is_empty() && req.users.len() <= 1024,
                "users must contain 1 to 1024 entries"
            );
            ensure!(req.dhash.len() <= 1024, "dhash exceeds 1024 bytes");
            let mut seen = HashSet::new();
            let users: Vec<DisallowedListUserEntry> = req
                .users
                .into_iter()
                .map(|v| {
                    let jid = contact_jid(&v.jid)?;
                    ensure!(
                        jid.server == Server::Lid,
                        "exclusion list users require LID JIDs"
                    );
                    ensure!(
                        seen.insert(jid.to_string()),
                        "duplicate exclusion list user"
                    );
                    let pn_jid = v.phone_number.map(|v| contact_jid(&v)).transpose()?;
                    ensure!(
                        pn_jid.as_ref().is_none_or(|v| v.server == Server::Pn),
                        "phone_number must be a PN JID"
                    );
                    Ok(DisallowedListUserEntry {
                        jid,
                        pn_jid,
                        action: match v.action {
                            DisallowedAction::Add => DisallowedListAction::Add,
                            DisallowedAction::Remove => DisallowedListAction::Remove,
                        },
                    })
                })
                .collect::<Result<_>>()?;
            let update = DisallowedListUpdate {
                dhash: req.dhash,
                users,
            };
            Ok(PrivacyUpdateView {
                dhash: manager
                    .client(&req.session_id)
                    .await?
                    .set_privacy_disallowed_list(category, update)
                    .await?
                    .dhash,
            })
        },
    );
    registry.add::<DefaultDurationRequest, (), _, _>(
        "whatsapp::privacy::set_disappearing_default",
        "Set the default disappearing-message duration in seconds",
        |manager, req| async move {
            manager
                .client(&req.session_id)
                .await?
                .set_default_disappearing_mode(req.expiration_seconds)
                .await?;
            Ok(())
        },
    );
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateCommunityRequest {
    session_id: String,
    name: String,
    description: Option<String>,
    #[serde(default)]
    closed: bool,
    #[serde(default)]
    allow_non_admin_sub_group_creation: bool,
    #[serde(default = "default_true")]
    create_general_chat: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CommunityRequest {
    session_id: String,
    community_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SubgroupRequest {
    session_id: String,
    community_id: String,
    subgroup_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateSubgroupRequest {
    session_id: String,
    community_id: String,
    name: String,
    #[serde(default)]
    participants: Vec<String>,
    #[serde(default)]
    hidden: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LinkRequest {
    session_id: String,
    community_id: String,
    subgroups: Vec<LinkTarget>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LinkTarget {
    group_id: String,
    #[serde(default)]
    hidden: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UnlinkRequest {
    session_id: String,
    community_id: String,
    group_ids: Vec<String>,
    #[serde(default)]
    remove_orphan_members: bool,
}

#[derive(Serialize, JsonSchema)]
struct LinkView {
    succeeded: Vec<String>,
    failed: Vec<FailedGroup>,
}

#[derive(Serialize, JsonSchema)]
struct FailedGroup {
    group_id: String,
    code: u32,
}

fn link_view(succeeded: Vec<Jid>, failed: Vec<(Jid, u32)>) -> LinkView {
    LinkView {
        succeeded: succeeded.into_iter().map(|v| v.to_string()).collect(),
        failed: failed
            .into_iter()
            .map(|(jid, code)| FailedGroup {
                group_id: jid.to_string(),
                code,
            })
            .collect(),
    }
}

#[derive(Serialize, JsonSchema)]
struct SubgroupView {
    id: String,
    subject: String,
    participant_count: Option<u32>,
    creation_time: Option<u64>,
    owner: Option<String>,
    is_announcement_subgroup: bool,
    is_general_chat: bool,
    hidden: bool,
}

#[derive(Serialize, JsonSchema)]
struct SubgroupCount {
    group_id: String,
    participant_count: u32,
}

fn register_communities(registry: &mut Registry) {
    registry.add::<SessionRequest, Vec<OverviewView>, _, _>(
        "whatsapp::communities::list",
        "List participating communities",
        |manager, req| async move {
            Ok(manager
                .client(&req.session_id)
                .await?
                .community()
                .list_participating()
                .await?
                .into_iter()
                .map(Into::into)
                .collect())
        },
    );
    registry.add::<CreateCommunityRequest, GroupView, _, _>(
        "whatsapp::communities::create",
        "Create a community with optional general chat",
        |manager, req| async move {
            let name = subject(req.name)?.into_string();
            let description = req
                .description
                .map(GroupDescription::new)
                .transpose()?
                .map(GroupDescription::into_string);
            let options = CreateCommunityOptions {
                name,
                description,
                closed: req.closed,
                allow_non_admin_sub_group_creation: req.allow_non_admin_sub_group_creation,
                create_general_chat: req.create_general_chat,
            };
            Ok(manager
                .client(&req.session_id)
                .await?
                .community()
                .create(options)
                .await?
                .metadata
                .into())
        },
    );
    registry.add::<CommunityRequest, (), _, _>(
        "whatsapp::communities::deactivate",
        "Deactivate a community",
        |manager, req| async move {
            let jid = group_jid(&req.community_id)?;
            manager
                .client(&req.session_id)
                .await?
                .community()
                .deactivate(jid)
                .await?;
            Ok(())
        },
    );
    registry.add::<CreateSubgroupRequest, GroupView, _, _>(
        "whatsapp::communities::create_subgroup",
        "Create a group linked to a community",
        |manager, req| async move {
            let parent = group_jid(&req.community_id)?;
            let name = subject(req.name)?.into_string();
            let participants = contacts(&req.participants, true)?;
            let visibility = if req.hidden {
                SubgroupVisibility::Hidden
            } else {
                SubgroupVisibility::Visible
            };
            let options =
                CreateSubgroupOptions::new(name, &participants, parent).with_visibility(visibility);
            Ok(manager
                .client(&req.session_id)
                .await?
                .community()
                .create_subgroup_with_options(options)
                .await?
                .metadata
                .into())
        },
    );
    registry.add::<CommunityRequest, Vec<SubgroupView>, _, _>(
        "whatsapp::communities::subgroups",
        "List a community's linked groups",
        |manager, req| async move {
            let jid = group_jid(&req.community_id)?;
            Ok(manager
                .client(&req.session_id)
                .await?
                .community()
                .get_subgroups(&jid)
                .await?
                .into_iter()
                .map(|v| SubgroupView {
                    id: v.id.to_string(),
                    subject: v.subject,
                    participant_count: v.participant_count,
                    creation_time: v.creation,
                    owner: v.owner.map(|v| v.to_string()),
                    is_announcement_subgroup: v.is_default_sub_group,
                    is_general_chat: v.is_general_chat,
                    hidden: v.is_hidden_group,
                })
                .collect())
        },
    );
    registry.add::<CommunityRequest, Vec<SubgroupCount>, _, _>(
        "whatsapp::communities::subgroup_counts",
        "Fetch linked-group participant counts",
        |manager, req| async move {
            let jid = group_jid(&req.community_id)?;
            Ok(manager
                .client(&req.session_id)
                .await?
                .community()
                .get_subgroup_participant_counts(&jid)
                .await?
                .into_iter()
                .map(|(jid, participant_count)| SubgroupCount {
                    group_id: jid.to_string(),
                    participant_count,
                })
                .collect())
        },
    );
    registry.add::<SubgroupRequest, GroupView, _, _>(
        "whatsapp::communities::get_subgroup",
        "Fetch metadata for a linked group",
        |manager, req| async move {
            let parent = group_jid(&req.community_id)?;
            let subgroup = group_jid(&req.subgroup_id)?;
            Ok(manager
                .client(&req.session_id)
                .await?
                .community()
                .query_linked_group(parent, subgroup)
                .await?
                .into())
        },
    );
    registry.add::<SubgroupRequest, GroupView, _, _>(
        "whatsapp::communities::join_subgroup",
        "Join a community subgroup; approval requirements remain errors",
        |manager, req| async move {
            let parent = group_jid(&req.community_id)?;
            let subgroup = group_jid(&req.subgroup_id)?;
            Ok(manager
                .client(&req.session_id)
                .await?
                .community()
                .join_subgroup(parent, subgroup)
                .await?
                .into())
        },
    );
    registry.add::<LinkRequest, LinkView, _, _>(
        "whatsapp::communities::link_subgroups",
        "Link existing groups with explicit visibility",
        |manager, req| async move {
            let parent = group_jid(&req.community_id)?;
            ensure!(
                !req.subgroups.is_empty() && req.subgroups.len() <= 1024,
                "subgroups must contain 1 to 1024 entries"
            );
            let mut seen = HashSet::new();
            let options = req
                .subgroups
                .into_iter()
                .map(|v| {
                    let jid = group_jid(&v.group_id)?;
                    ensure!(jid != parent, "a community cannot link itself");
                    ensure!(seen.insert(jid.to_string()), "duplicate subgroup");
                    Ok(LinkSubgroupOptions::new(
                        jid,
                        if v.hidden {
                            SubgroupVisibility::Hidden
                        } else {
                            SubgroupVisibility::Visible
                        },
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let result = manager
                .client(&req.session_id)
                .await?
                .community()
                .link_subgroups_with_options(parent, &options)
                .await?;
            Ok(link_view(result.linked_jids, result.failed_groups))
        },
    );
    registry.add::<UnlinkRequest, LinkView, _, _>(
        "whatsapp::communities::unlink_subgroups",
        "Unlink existing groups, optionally removing orphan members",
        |manager, req| async move {
            let parent = group_jid(&req.community_id)?;
            ensure!(
                !req.group_ids.is_empty() && req.group_ids.len() <= 1024,
                "group_ids must contain 1 to 1024 entries"
            );
            let mut seen = HashSet::new();
            let targets = req
                .group_ids
                .iter()
                .map(|v| {
                    let jid = group_jid(v)?;
                    ensure!(jid != parent, "a community cannot unlink itself");
                    ensure!(seen.insert(jid.to_string()), "duplicate subgroup");
                    Ok(jid)
                })
                .collect::<Result<Vec<_>>>()?;
            let result = manager
                .client(&req.session_id)
                .await?
                .community()
                .unlink_subgroups(parent, &targets, req.remove_orphan_members)
                .await?;
            Ok(link_view(result.unlinked_jids, result.failed_groups))
        },
    );
    registry.add::<ParticipantsRequest, Vec<ParticipantChange>, _, _>(
        "whatsapp::communities::remove_participants",
        "Remove members from a community and its groups",
        |manager, req| async move {
            let parent = group_jid(&req.group_id)?;
            let targets = req.targets()?;
            Ok(manager
                .client(&req.session_id)
                .await?
                .community()
                .remove_participants(parent, &targets)
                .await?
                .into_iter()
                .map(Into::into)
                .collect())
        },
    );
    registry.add::<CommunityRequest, Vec<ParticipantView>, _, _>(
        "whatsapp::communities::participants",
        "Fetch linked group participants",
        |manager, req| async move {
            let parent = group_jid(&req.community_id)?;
            Ok(manager
                .client(&req.session_id)
                .await?
                .community()
                .get_linked_groups_participants(parent)
                .await?
                .into_iter()
                .map(Into::into)
                .collect())
        },
    );
}
