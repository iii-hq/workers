//! `github::release::*` — typed wrappers over `gh release …`.

use schemars::JsonSchema;
use serde::Deserialize;

use super::{argv, push_bool, push_opt};

pub const LIST_ID: &str = "github::release::list";
pub const LIST_DESC: &str = "List releases: { repo: \"owner/name\", limit? } -> { value: [{tagName, name, isDraft, isLatest, isPrerelease, createdAt, publishedAt}] }.";
pub const LIST_JSON: &str = "tagName,name,isDraft,isLatest,isPrerelease,createdAt,publishedAt";

pub const VIEW_ID: &str = "github::release::view";
pub const VIEW_DESC: &str =
    "View one release including body and assets: { repo: \"owner/name\", tag } -> { value }.";
pub const VIEW_JSON: &str =
    "tagName,name,body,isDraft,isPrerelease,author,assets,targetCommitish,url,createdAt,publishedAt";

pub const CREATE_ID: &str = "github::release::create";
pub const CREATE_DESC: &str = "Create a release: { repo, tag, title?, notes?, generate_notes?, draft?, prerelease?, target? } -> { output: <release url> }. generate_notes autogenerates from merged PRs.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Maximum number of results (gh default: 30).
    pub limit: Option<u32>,
}

pub fn list_args(r: &ListRequest) -> Vec<String> {
    let mut a = argv([
        "release",
        "list",
        "-R",
        r.repo.as_str(),
        "--json",
        LIST_JSON,
    ]);
    push_opt(&mut a, "--limit", r.limit);
    a
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ViewRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Release tag (e.g. "v1.2.0").
    pub tag: String,
}

pub fn view_args(r: &ViewRequest) -> Vec<String> {
    argv([
        "release",
        "view",
        r.tag.as_str(),
        "-R",
        r.repo.as_str(),
        "--json",
        VIEW_JSON,
    ])
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Tag to release (created at target if it does not exist).
    pub tag: String,
    /// Release title.
    pub title: Option<String>,
    /// Release notes body (markdown).
    pub notes: Option<String>,
    /// Autogenerate notes from merged PRs since the last release.
    pub generate_notes: Option<bool>,
    /// Create as draft.
    pub draft: Option<bool>,
    /// Mark as prerelease.
    pub prerelease: Option<bool>,
    /// Target branch or commit SHA for the tag (gh default: the default branch).
    pub target: Option<String>,
}

pub fn create_args(r: &CreateRequest) -> Vec<String> {
    let mut a = argv(["release", "create", r.tag.as_str(), "-R", r.repo.as_str()]);
    push_opt(&mut a, "--title", r.title.as_deref());
    push_opt(&mut a, "--notes", r.notes.as_deref());
    push_bool(&mut a, "--generate-notes", r.generate_notes);
    push_bool(&mut a, "--draft", r.draft);
    push_bool(&mut a, "--prerelease", r.prerelease);
    push_opt(&mut a, "--target", r.target.as_deref());
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn create_maps_generate_notes_and_target() {
        let r: CreateRequest = serde_json::from_value(json!({
            "repo": "o/r", "tag": "v0.1.0", "generate_notes": true, "target": "main"
        }))
        .unwrap();
        assert_eq!(
            create_args(&r),
            vec![
                "release",
                "create",
                "v0.1.0",
                "-R",
                "o/r",
                "--generate-notes",
                "--target",
                "main",
            ]
        );
    }
}
