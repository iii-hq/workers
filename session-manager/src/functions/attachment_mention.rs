//! `session::attachments::mention::search` / `session::attachments::mention::get`
//! — the `@attachment(id="a_…")` chat mention provider.
//!
//! A file attached to a chat is mentioned by its attachment id; the pill shows
//! its name. Search lists only the asking chat's attachments
//! (`context.session_id`; nothing without it), newest first for an empty
//! query, then by name. Get resolves an id alone to the pill, the preview
//! card and the agent's one-line summary, and declares the provider, whose
//! details call is `session::read-attachment`. Both are internal plumbing;
//! agents read a mentioned file through `session::read-attachment`.

use mention_contract::{
    MentionField, MentionGetRequest, MentionItem, MentionProvider, MentionSearchRequest,
    MentionSearchResponse, MentionView, color,
};
use serde_json::json;

use super::mention::rfc3339_from_ms;
use super::{Deps, read_attachment};
use crate::error::SessionError;
use crate::reading::{AttachmentKind, human_size};
use crate::types::AttachmentMeta;

pub const SEARCH_FN: &str = "session::attachments::mention::search";
pub const GET_FN: &str = "session::attachments::mention::get";
pub const MENTION_NAME: &str = "attachment";
pub const ICON: &str = "file";

pub fn provider() -> MentionProvider {
    MentionProvider::new(MENTION_NAME, "Attachments", SEARCH_FN)
        .description(
            "A file attached to this chat, by attachment id (a_…); read it with session::read-attachment",
        )
        .icon(ICON)
        .color(color::TEAL)
        .details(read_attachment::FUNCTION_ID, "attachment_id")
}

pub async fn search(
    deps: &Deps,
    req: MentionSearchRequest,
) -> Result<MentionSearchResponse, SessionError> {
    let Some(session_id) = req
        .context
        .as_ref()
        .and_then(|context| context.session_id.as_deref())
        .filter(|id| !id.is_empty())
    else {
        return Ok(MentionSearchResponse { items: Vec::new() });
    };
    let attachments = deps.service.session_attachments(session_id).await?;
    Ok(MentionSearchResponse {
        items: search_attachments(&attachments, req.trimmed_query(), req.effective_limit())
            .into_iter()
            .map(item)
            .collect(),
    })
}

pub async fn get(deps: &Deps, req: MentionGetRequest) -> Result<Option<MentionView>, SessionError> {
    Ok(deps
        .service
        .find_attachment_meta(&req.id, None)
        .await?
        .map(|meta| view(&meta)))
}

/// How well an attachment matches a lowercase query; lower is better.
fn search_tier(meta: &AttachmentMeta, query: &str) -> Option<u8> {
    let name = meta.name.to_lowercase();
    if name == query || meta.attachment_id.to_lowercase() == query {
        return Some(0);
    }
    if name.starts_with(query) {
        return Some(1);
    }
    let words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let mut terms = query.split_whitespace().peekable();
    if terms.peek().is_some() && terms.all(|term| words.iter().any(|w| w.starts_with(term))) {
        return Some(2);
    }
    if name.contains(query) {
        return Some(3);
    }
    None
}

/// Attachments matching `query` (case-insensitive): exact name (or id), name
/// prefix, every query word starting a name word, then substring; newest
/// first within a tier. An empty query lists them newest first.
pub fn search_attachments<'a>(
    attachments: &'a [AttachmentMeta],
    query: &str,
    limit: usize,
) -> Vec<&'a AttachmentMeta> {
    let query = query.to_lowercase();
    let mut hits: Vec<(u8, &AttachmentMeta)> = attachments
        .iter()
        .filter_map(|meta| {
            let tier = if query.is_empty() {
                Some(0)
            } else {
                search_tier(meta, &query)
            };
            tier.map(|tier| (tier, meta))
        })
        .collect();
    hits.sort_by(|(a_tier, a), (b_tier, b)| {
        a_tier
            .cmp(b_tier)
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| b.attachment_id.cmp(&a.attachment_id))
    });
    hits.truncate(limit);
    hits.into_iter().map(|(_, meta)| meta).collect()
}

/// `PDF · 1.2 MB`: the kind by name and mime (the bytes are not read for a
/// mention), then the size.
fn description(meta: &AttachmentMeta) -> String {
    format!(
        "{} · {}",
        AttachmentKind::of(&meta.name, &meta.mime).label(),
        human_size(meta.size)
    )
}

fn item(meta: &AttachmentMeta) -> MentionItem {
    MentionItem {
        id: meta.attachment_id.clone(),
        label: meta.name.clone(),
        hint: Some(human_size(meta.size)),
        description: Some(description(meta)),
        icon: None,
        color: None,
    }
}

/// The view of one attachment: no `open` (the apps open the bytes
/// themselves), `data` = what a client needs to draw the pill and build the
/// message's `file` block.
pub fn view(meta: &AttachmentMeta) -> MentionView {
    let size = human_size(meta.size);
    let added = rfc3339_from_ms(meta.created_at);
    MentionView {
        id: meta.attachment_id.clone(),
        label: meta.name.clone(),
        hint: Some(size.clone()),
        description: Some(description(meta)),
        icon: None,
        color: None,
        fields: vec![
            MentionField::new("Type", meta.mime.clone()),
            MentionField::new("Size", size.clone()),
            MentionField::new("Added", added.clone()),
        ],
        open: None,
        summary: Some(format!(
            "Attachment {} · {} · {size}",
            serde_json::to_string(&meta.name).unwrap_or_default(),
            meta.mime
        )),
        data: Some(json!({
            "session_id": meta.session_id,
            "name": meta.name,
            "mime": meta.mime,
            "size": meta.size,
            "created_at": meta.created_at,
        })),
        updated_at: Some(added),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, name: &str, created_at: i64) -> AttachmentMeta {
        AttachmentMeta {
            attachment_id: id.into(),
            session_id: "s_1".into(),
            name: name.into(),
            mime: "application/pdf".into(),
            size: 1_234_567,
            sha256: "00".into(),
            created_at,
        }
    }

    fn sample() -> Vec<AttachmentMeta> {
        vec![
            meta("a_old", "report.pdf", 1),
            meta("a_new", "report-final.pdf", 4),
            meta("a_word", "Q3 sales report.pdf", 3),
            meta("a_sub", "myreports.pdf", 2),
            meta("a_other", "budget.xlsx", 5),
        ]
    }

    fn ids(query: &str) -> Vec<String> {
        search_attachments(&sample(), query, 10)
            .into_iter()
            .map(|m| m.attachment_id.clone())
            .collect()
    }

    #[test]
    fn search_ranks_exact_then_prefix_then_word_then_substring() {
        // The exact name first, then the substring hit.
        assert_eq!(ids("report.pdf"), ["a_old", "a_word"]);
        assert_eq!(ids("REPORT"), ["a_new", "a_old", "a_word", "a_sub"]);
        assert_eq!(ids("sales rep"), ["a_word"]);
        assert_eq!(ids("a_other"), ["a_other"]);
        assert!(ids("zzz").is_empty());
    }

    #[test]
    fn an_empty_query_lists_newest_first_up_to_the_limit() {
        assert_eq!(ids(""), ["a_other", "a_new", "a_word", "a_sub", "a_old"]);
        assert_eq!(search_attachments(&sample(), "", 2).len(), 2);
    }

    #[test]
    fn items_and_views_carry_name_size_and_kind() {
        let m = meta("a_1", "report.pdf", 1_791_244_800_000);
        let it = item(&m);
        assert_eq!(it.label, "report.pdf");
        assert_eq!(it.hint.as_deref(), Some("1.2 MB"));
        assert_eq!(it.description.as_deref(), Some("PDF · 1.2 MB"));

        let v = view(&m);
        assert_eq!(v.id, "a_1");
        assert_eq!(v.label, "report.pdf");
        assert_eq!(v.hint.as_deref(), Some("1.2 MB"));
        assert_eq!(
            v.summary.as_deref(),
            Some(r#"Attachment "report.pdf" · application/pdf · 1.2 MB"#)
        );
        let fields: Vec<(&str, &str)> = v
            .fields
            .iter()
            .map(|f| (f.label.as_str(), f.value.as_str()))
            .collect();
        assert_eq!(
            fields,
            [
                ("Type", "application/pdf"),
                ("Size", "1.2 MB"),
                ("Added", "2026-10-06T00:00:00.000Z")
            ]
        );
        assert!(v.open.is_none());
        assert_eq!(
            v.data,
            Some(
                json!({ "session_id": "s_1", "name": "report.pdf", "mime": "application/pdf",
                         "size": 1_234_567, "created_at": 1_791_244_800_000i64 })
            )
        );
    }

    #[test]
    fn the_get_function_declares_the_attachment_provider() {
        let provider =
            MentionProvider::from_metadata(&provider().metadata()).expect("descriptor validates");
        assert_eq!(provider.name, "attachment");
        assert_eq!(provider.label, "Attachments");
        assert_eq!(provider.search, SEARCH_FN);
        assert_eq!(provider.icon.as_deref(), Some("file"));
        assert_eq!(provider.color.as_deref(), Some("teal"));
        let details = provider.details.expect("details");
        assert_eq!(details.function_id, "session::read-attachment");
        assert_eq!(details.payload("a_1"), json!({ "attachment_id": "a_1" }));
    }
}
