//! `session::mention::search` / `session::mention::get` — the
//! `@session(id="<session_id>")` chat mention provider.
//!
//! Search feeds the console's `@session:` menu; get turns one id into the
//! pill, the preview card and the agent's one-line summary. The get
//! function carries the provider descriptor in its metadata (see
//! `mention-contract`); agents read the session through `session::get` and
//! its transcript through `session::messages-tail`.

use mention_contract::{
    color, tone, MentionField, MentionGetRequest, MentionItem, MentionOpen, MentionProvider,
    MentionSearchRequest, MentionSearchResponse, MentionView,
};
use serde_json::Value;

use super::Deps;
use crate::error::SessionError;
use crate::types::{SessionKind, SessionMeta, SessionStatus};

pub const SEARCH_FN: &str = "session::mention::search";
pub const GET_FN: &str = "session::mention::get";
pub const MENTION_NAME: &str = "session";
pub const ICON: &str = "session";

pub fn provider() -> MentionProvider {
    MentionProvider::new(MENTION_NAME, "Sessions", SEARCH_FN)
        .description(
            "A chat session, by session id; its transcript reads through session::messages-tail",
        )
        .icon(ICON)
        .color(color::NEUTRAL)
        .details("session::get", "session_id")
}

pub async fn search(
    deps: &Deps,
    req: MentionSearchRequest,
) -> Result<MentionSearchResponse, SessionError> {
    let metas = deps.service.all_metas().await?;
    let exclude = req
        .context
        .as_ref()
        .and_then(|context| context.session_id.as_deref());
    Ok(MentionSearchResponse {
        items: search_metas(&metas, req.trimmed_query(), exclude, req.effective_limit())
            .into_iter()
            .map(item)
            .collect(),
    })
}

pub async fn get(deps: &Deps, req: MentionGetRequest) -> Result<Option<MentionView>, SessionError> {
    let Some(meta) = deps.service.get_meta(&req.id).await? else {
        return Ok(None);
    };
    Ok(Some(view(&meta)))
}

/// How well a session matches a lowercase query; lower is better.
fn search_tier(meta: &SessionMeta, query: &str) -> Option<u8> {
    let id = meta.session_id.to_lowercase();
    let title = meta.title.to_lowercase();
    if id == query || title == query || title.starts_with(query) {
        return Some(0);
    }
    if query.len() >= 4 && id.starts_with(query) {
        return Some(1);
    }
    let words: Vec<&str> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let mut terms = query.split_whitespace().peekable();
    if terms.peek().is_some() && terms.all(|term| words.iter().any(|w| w.starts_with(term))) {
        return Some(1);
    }
    if title.contains(query) {
        return Some(2);
    }
    if meta.description.to_lowercase().contains(query) {
        return Some(3);
    }
    None
}

/// Sessions matching `query` (case-insensitive), best first: top-level
/// chats before sub-agents, then the most recently updated. e2e sessions
/// and the session the search was asked from never match. An empty query
/// lists the most recently updated sessions.
pub fn search_metas<'a>(
    metas: &'a [SessionMeta],
    query: &str,
    exclude: Option<&str>,
    limit: usize,
) -> Vec<&'a SessionMeta> {
    let query = query.to_lowercase();
    let mut hits: Vec<(u8, bool, &SessionMeta)> = metas
        .iter()
        .filter(|meta| meta.kind != SessionKind::E2e)
        .filter(|meta| exclude != Some(meta.session_id.as_str()))
        .filter_map(|meta| {
            let tier = if query.is_empty() {
                Some(0)
            } else {
                search_tier(meta, &query)
            };
            tier.map(|tier| (tier, parent_session(meta).is_some(), meta))
        })
        .collect();
    hits.sort_by(|(a_tier, a_child, a), (b_tier, b_child, b)| {
        a_tier
            .cmp(b_tier)
            .then_with(|| a_child.cmp(b_child))
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    hits.truncate(limit);
    hits.into_iter().map(|(_, _, meta)| meta).collect()
}

fn item(meta: &SessionMeta) -> MentionItem {
    MentionItem {
        id: meta.session_id.clone(),
        label: label(meta),
        hint: parent_session(meta).map(|_| "sub-agent".to_string()),
        description: Some(description(meta)),
        icon: None,
        color: Some(status_color(meta.status).to_string()),
    }
}

/// The view of one session.
pub fn view(meta: &SessionMeta) -> MentionView {
    let mut fields = vec![
        MentionField::new("Status", status_label(meta.status)).tone(status_tone(meta.status)),
        MentionField::new("Messages", meta.message_count.to_string()),
        MentionField::new("Updated", rfc3339_from_ms(meta.updated_at)),
    ];
    if let Some(reason) = meta.status_reason.as_deref().filter(|r| !r.is_empty()) {
        fields.push(MentionField::new("Reason", reason));
    }
    if let Some(agent) = metadata_str(meta, "agent_profile") {
        fields.push(MentionField::new("Agent", agent));
    }
    if let Some(parent) = parent_session(meta) {
        fields.push(MentionField::new("Parent", parent));
    }
    if meta.kind != SessionKind::User {
        fields.push(MentionField::new("Kind", kind_label(meta.kind)));
    }
    MentionView {
        id: meta.session_id.clone(),
        label: label(meta),
        hint: parent_session(meta).map(|_| "sub-agent".to_string()),
        description: Some(description(meta)),
        icon: Some(ICON.to_string()),
        color: Some(status_color(meta.status).to_string()),
        fields,
        open: Some(MentionOpen::Session {
            session: meta.session_id.clone(),
        }),
        summary: Some(summary(meta)),
        data: public_meta(meta),
        updated_at: Some(rfc3339_from_ms(meta.updated_at)),
    }
}

fn label(meta: &SessionMeta) -> String {
    let title = meta.title.trim();
    if title.is_empty() {
        "Untitled session".to_string()
    } else {
        title.to_string()
    }
}

fn description(meta: &SessionMeta) -> String {
    let messages = match meta.message_count {
        1 => "1 message".to_string(),
        n => format!("{n} messages"),
    };
    format!("{} · {messages}", status_label(meta.status))
}

fn summary(meta: &SessionMeta) -> String {
    let title = serde_json::to_string(&label(meta)).unwrap_or_default();
    let mut line = format!(
        "Chat session {title} ({}) · status: {} · {} message(s) · updated {}",
        meta.session_id,
        status_label(meta.status).to_lowercase(),
        meta.message_count,
        rfc3339_from_ms(meta.updated_at),
    );
    if let Some(agent) = metadata_str(meta, "agent_profile") {
        line.push_str(&format!(" · agent: {agent}"));
    }
    if let Some(parent) = parent_session(meta) {
        line.push_str(&format!(" · sub-agent of {parent}"));
    }
    line
}

/// The metadata record without the parked composer draft, which is the
/// session owner's unsent input and not part of what a mention shares.
fn public_meta(meta: &SessionMeta) -> Option<Value> {
    let mut value = serde_json::to_value(meta).ok()?;
    if let Some(object) = value.as_object_mut() {
        object.remove("draft");
        object.remove("draft_attachments");
    }
    Some(value)
}

fn metadata_str<'a>(meta: &'a SessionMeta, key: &str) -> Option<&'a str> {
    meta.metadata
        .as_ref()?
        .get(key)?
        .as_str()
        .filter(|value| !value.is_empty())
}

fn parent_session(meta: &SessionMeta) -> Option<&str> {
    metadata_str(meta, "parent_session_id")
}

fn status_label(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Idle => "Idle",
        SessionStatus::Working => "Working",
        SessionStatus::Done => "Done",
        SessionStatus::Error => "Error",
    }
}

fn status_color(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Idle => color::NEUTRAL,
        SessionStatus::Working => color::BLUE,
        SessionStatus::Done => color::GREEN,
        SessionStatus::Error => color::ROSE,
    }
}

fn status_tone(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Idle => tone::NEUTRAL,
        SessionStatus::Working => tone::INFO,
        SessionStatus::Done => tone::SUCCESS,
        SessionStatus::Error => tone::DANGER,
    }
}

fn kind_label(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::User => "user",
        SessionKind::Automation => "automation",
        SessionKind::E2e => "e2e",
    }
}

/// RFC 3339 (UTC, millisecond precision) for a milliseconds-since-epoch
/// timestamp.
pub fn rfc3339_from_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let day_secs = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        day_secs / 3600,
        (day_secs % 3600) / 60,
        day_secs % 60,
    )
}

/// Proleptic Gregorian (year, month, day) for days since 1970-01-01
/// (H. Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta(id: &str, title: &str, updated_at: i64) -> SessionMeta {
        SessionMeta {
            session_id: id.into(),
            title: title.into(),
            description: String::new(),
            status: SessionStatus::Idle,
            status_reason: None,
            kind: SessionKind::User,
            metadata: None,
            forked_from: None,
            draft: None,
            draft_attachments: None,
            created_at: 0,
            updated_at,
            message_count: 0,
        }
    }

    fn child(mut meta: SessionMeta, parent: &str) -> SessionMeta {
        meta.metadata = json!({ "parent_session_id": parent }).as_object().cloned();
        meta
    }

    fn ids(metas: &[SessionMeta], query: &str, exclude: Option<&str>) -> Vec<String> {
        search_metas(metas, query, exclude, 10)
            .into_iter()
            .map(|meta| meta.session_id.clone())
            .collect()
    }

    fn sample() -> Vec<SessionMeta> {
        let mut e2e = meta("s_e2e", "login e2e run", 9);
        e2e.kind = SessionKind::E2e;
        let mut described = meta("s_desc", "database work", 3);
        described.description = "about the login bug".into();
        vec![
            meta("s_old", "fix login redirect", 1),
            meta("s_new", "login page polish", 2),
            child(meta("s_child", "login sub-task", 5), "s_new"),
            described,
            e2e,
        ]
    }

    #[test]
    fn search_ranks_title_prefix_before_words_and_description() {
        assert_eq!(
            ids(&sample(), "login", None),
            ["s_new", "s_child", "s_old", "s_desc"]
        );
        assert_eq!(ids(&sample(), "redirect fix", None), ["s_old"]);
    }

    #[test]
    fn search_matches_session_ids_and_skips_e2e_and_the_asking_session() {
        assert_eq!(ids(&sample(), "s_old", None), ["s_old"]);
        assert_eq!(ids(&sample(), "s_ch", None), ["s_child"]);
        assert!(!ids(&sample(), "", None).contains(&"s_e2e".to_string()));
        assert!(!ids(&sample(), "login", Some("s_new")).contains(&"s_new".to_string()));
    }

    #[test]
    fn an_empty_query_lists_top_level_sessions_first_then_recent() {
        assert_eq!(
            ids(&sample(), "", None),
            ["s_desc", "s_new", "s_old", "s_child"]
        );
    }

    #[test]
    fn view_carries_status_parent_and_hides_the_draft() {
        let mut session = child(meta("s_child", "", 1_791_244_800_000), "s_new");
        session.status = SessionStatus::Error;
        session.message_count = 3;
        session.draft = Some("unsent words".into());
        let view = view(&session);
        assert_eq!(view.label, "Untitled session");
        assert_eq!(view.hint.as_deref(), Some("sub-agent"));
        assert_eq!(view.description.as_deref(), Some("Error · 3 messages"));
        assert_eq!(view.color.as_deref(), Some(color::ROSE));
        assert_eq!(
            view.open,
            Some(MentionOpen::Session {
                session: "s_child".into()
            })
        );
        assert!(view
            .fields
            .iter()
            .any(|f| f.label == "Parent" && f.value == "s_new"));
        assert_eq!(
            view.summary.as_deref(),
            Some(
                r#"Chat session "Untitled session" (s_child) · status: error · 3 message(s) · updated 2026-10-06T00:00:00.000Z · sub-agent of s_new"#
            )
        );
        let data = view.data.expect("data");
        assert!(data.get("draft").is_none());
        assert_eq!(data["session_id"], json!("s_child"));
    }

    #[test]
    fn the_get_function_declares_the_provider() {
        let provider =
            MentionProvider::from_metadata(&provider().metadata()).expect("descriptor validates");
        assert_eq!(provider.name, "session");
        assert_eq!(provider.search, SEARCH_FN);
        let details = provider.details.expect("details");
        assert_eq!(details.function_id, "session::get");
        assert_eq!(details.payload("s_1"), json!({ "session_id": "s_1" }));
    }

    #[test]
    fn rfc3339_formats_epoch_and_leap_days() {
        assert_eq!(rfc3339_from_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339_from_ms(951_782_400_123), "2000-02-29T00:00:00.123Z");
        assert_eq!(
            rfc3339_from_ms(1_791_244_800_000),
            "2026-10-06T00:00:00.000Z"
        );
        assert_eq!(rfc3339_from_ms(-1), "1969-12-31T23:59:59.999Z");
    }
}
