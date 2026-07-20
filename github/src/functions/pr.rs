//! `github::pr::*` — typed wrappers over `gh pr …`. Builders are pure
//! (request → argv) so tests assert them without spawning anything.

use schemars::JsonSchema;
use serde::Deserialize;

use super::{argv, push_bool, push_each, push_opt};

pub const LIST_ID: &str = "github::pr::list";
pub const LIST_DESC: &str = "List pull requests: { repo: \"owner/name\", state?, limit?, author?, labels?, base?, search? } -> { value: [{number, title, state, url, author, headRefName, baseRefName, isDraft, labels, createdAt, updatedAt}] }.";
pub const LIST_JSON: &str =
    "number,title,state,url,author,headRefName,baseRefName,isDraft,labels,createdAt,updatedAt";

pub const VIEW_ID: &str = "github::pr::view";
pub const VIEW_DESC: &str = "View one pull request with body, mergeability, review decision, and diff stats: { repo: \"owner/name\", number } -> { value }.";
pub const VIEW_JSON: &str = "number,title,state,url,author,headRefName,baseRefName,isDraft,labels,createdAt,updatedAt,body,mergeable,mergeStateStatus,reviewDecision,additions,deletions,changedFiles,assignees,milestone,mergedAt,closedAt";

pub const CREATE_ID: &str = "github::pr::create";
pub const CREATE_DESC: &str = "Open a pull request: { repo: \"owner/name\", title, body, head, base?, draft? } -> { output: <pr url> }. head is required: the worker runs outside any checkout, so gh cannot infer a current branch.";

pub const EDIT_ID: &str = "github::pr::edit";
pub const EDIT_DESC: &str = "Edit a pull request's title/body/base/labels: { repo, number, title?, body?, base?, add_labels?, remove_labels? } -> { output }.";

pub const MERGE_ID: &str = "github::pr::merge";
pub const MERGE_DESC: &str = "Merge a pull request: { repo, number, method: merge|squash|rebase, delete_branch?, auto? } -> { output }. auto enables auto-merge once requirements pass.";

pub const COMMENT_ID: &str = "github::pr::comment";
pub const COMMENT_DESC: &str =
    "Comment on a pull request: { repo, number, body } -> { output: <comment url> }.";

pub const REVIEW_ID: &str = "github::pr::review";
pub const REVIEW_DESC: &str = "Review a pull request: { repo, number, event: approve|request-changes|comment, body? } -> { output }. gh requires body for request-changes and comment reviews.";

pub const DIFF_ID: &str = "github::pr::diff";
pub const DIFF_DESC: &str = "Unified diff of a pull request: { repo, number } -> { diff, truncated }. truncated is true when the diff exceeded the capture cap.";

pub const CHECKS_ID: &str = "github::pr::checks";
pub const CHECKS_DESC: &str = "CI check rollup for a pull request: { repo, number } -> { value: [{bucket, name, state, link, workflow, description, startedAt, completedAt}] }. Failing or pending checks are data, not errors.";
pub const CHECKS_JSON: &str = "bucket,name,state,link,workflow,description,startedAt,completedAt";

/// PR state filter.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PrState {
    Open,
    Closed,
    Merged,
    All,
}

impl PrState {
    fn as_str(self) -> &'static str {
        match self {
            PrState::Open => "open",
            PrState::Closed => "closed",
            PrState::Merged => "merged",
            PrState::All => "all",
        }
    }
}

/// Merge method.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    fn as_flag(self) -> &'static str {
        match self {
            MergeMethod::Merge => "--merge",
            MergeMethod::Squash => "--squash",
            MergeMethod::Rebase => "--rebase",
        }
    }
}

/// Review event.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewEvent {
    Approve,
    RequestChanges,
    Comment,
}

impl ReviewEvent {
    fn as_flag(self) -> &'static str {
        match self {
            ReviewEvent::Approve => "--approve",
            ReviewEvent::RequestChanges => "--request-changes",
            ReviewEvent::Comment => "--comment",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Filter by state (gh default: open).
    pub state: Option<PrState>,
    /// Maximum number of results (gh default: 30).
    pub limit: Option<u32>,
    /// Filter by author login.
    pub author: Option<String>,
    /// Filter by labels (all must match).
    pub labels: Option<Vec<String>>,
    /// Filter by base branch.
    pub base: Option<String>,
    /// Search query narrowing the list further (GitHub search syntax).
    pub search: Option<String>,
}

pub fn list_args(r: &ListRequest) -> Vec<String> {
    let mut a = argv(["pr", "list", "-R", r.repo.as_str(), "--json", LIST_JSON]);
    push_opt(&mut a, "--state", r.state.map(|s| s.as_str()));
    push_opt(&mut a, "--limit", r.limit);
    push_opt(&mut a, "--author", r.author.as_deref());
    push_each(&mut a, "--label", &r.labels);
    push_opt(&mut a, "--base", r.base.as_deref());
    push_opt(&mut a, "--search", r.search.as_deref());
    a
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ViewRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Pull request number.
    pub number: u64,
}

pub fn view_args(r: &ViewRequest) -> Vec<String> {
    argv([
        "pr",
        "view",
        r.number.to_string().as_str(),
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
    /// Pull request title.
    pub title: String,
    /// Pull request body (markdown).
    pub body: String,
    /// Head branch the PR ships. Required: the worker runs outside any
    /// checkout, so gh cannot infer a current branch.
    pub head: String,
    /// Base branch to merge into (gh default: the repo default branch).
    pub base: Option<String>,
    /// Open as draft.
    pub draft: Option<bool>,
}

pub fn create_args(r: &CreateRequest) -> Vec<String> {
    let mut a = argv([
        "pr",
        "create",
        "-R",
        r.repo.as_str(),
        "--title",
        r.title.as_str(),
        "--body",
        r.body.as_str(),
        "--head",
        r.head.as_str(),
    ]);
    push_opt(&mut a, "--base", r.base.as_deref());
    push_bool(&mut a, "--draft", r.draft);
    a
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Pull request number.
    pub number: u64,
    /// New title.
    pub title: Option<String>,
    /// New body (markdown).
    pub body: Option<String>,
    /// New base branch.
    pub base: Option<String>,
    /// Labels to add.
    pub add_labels: Option<Vec<String>>,
    /// Labels to remove.
    pub remove_labels: Option<Vec<String>>,
}

pub fn edit_args(r: &EditRequest) -> Vec<String> {
    let mut a = argv([
        "pr",
        "edit",
        r.number.to_string().as_str(),
        "-R",
        r.repo.as_str(),
    ]);
    push_opt(&mut a, "--title", r.title.as_deref());
    push_opt(&mut a, "--body", r.body.as_deref());
    push_opt(&mut a, "--base", r.base.as_deref());
    push_each(&mut a, "--add-label", &r.add_labels);
    push_each(&mut a, "--remove-label", &r.remove_labels);
    a
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MergeRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Pull request number.
    pub number: u64,
    /// Merge method.
    pub method: MergeMethod,
    /// Delete the head branch after merging.
    pub delete_branch: Option<bool>,
    /// Enable auto-merge: merge automatically once requirements pass.
    pub auto: Option<bool>,
}

pub fn merge_args(r: &MergeRequest) -> Vec<String> {
    let mut a = argv([
        "pr",
        "merge",
        r.number.to_string().as_str(),
        "-R",
        r.repo.as_str(),
        r.method.as_flag(),
    ]);
    push_bool(&mut a, "--delete-branch", r.delete_branch);
    push_bool(&mut a, "--auto", r.auto);
    a
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommentRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Pull request number.
    pub number: u64,
    /// Comment body (markdown).
    pub body: String,
}

pub fn comment_args(r: &CommentRequest) -> Vec<String> {
    argv([
        "pr",
        "comment",
        r.number.to_string().as_str(),
        "-R",
        r.repo.as_str(),
        "--body",
        r.body.as_str(),
    ])
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReviewRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Pull request number.
    pub number: u64,
    /// Review event.
    pub event: ReviewEvent,
    /// Review body (gh requires it for request-changes and comment).
    pub body: Option<String>,
}

pub fn review_args(r: &ReviewRequest) -> Vec<String> {
    let mut a = argv([
        "pr",
        "review",
        r.number.to_string().as_str(),
        "-R",
        r.repo.as_str(),
        r.event.as_flag(),
    ]);
    push_opt(&mut a, "--body", r.body.as_deref());
    a
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiffRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Pull request number.
    pub number: u64,
}

pub fn diff_args(r: &DiffRequest) -> Vec<String> {
    argv([
        "pr",
        "diff",
        r.number.to_string().as_str(),
        "-R",
        r.repo.as_str(),
    ])
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChecksRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Pull request number.
    pub number: u64,
}

pub fn checks_args(r: &ChecksRequest) -> Vec<String> {
    argv([
        "pr",
        "checks",
        r.number.to_string().as_str(),
        "-R",
        r.repo.as_str(),
        "--json",
        CHECKS_JSON,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn list_maps_every_filter() {
        let r: ListRequest = serde_json::from_value(json!({
            "repo": "o/r",
            "state": "merged",
            "limit": 5,
            "author": "octocat",
            "labels": ["bug", "p1"],
            "base": "main",
            "search": "review:required"
        }))
        .unwrap();
        assert_eq!(
            list_args(&r),
            vec![
                "pr",
                "list",
                "-R",
                "o/r",
                "--json",
                LIST_JSON,
                "--state",
                "merged",
                "--limit",
                "5",
                "--author",
                "octocat",
                "--label",
                "bug",
                "--label",
                "p1",
                "--base",
                "main",
                "--search",
                "review:required",
            ]
        );
    }

    #[test]
    fn minimal_list_is_just_the_json_fields() {
        let r: ListRequest = serde_json::from_value(json!({ "repo": "o/r" })).unwrap();
        assert_eq!(
            list_args(&r),
            vec!["pr", "list", "-R", "o/r", "--json", LIST_JSON]
        );
    }

    #[test]
    fn create_always_passes_head() {
        let r: CreateRequest = serde_json::from_value(json!({
            "repo": "o/r", "title": "t", "body": "b", "head": "feat/x", "draft": true
        }))
        .unwrap();
        assert_eq!(
            create_args(&r),
            vec![
                "pr", "create", "-R", "o/r", "--title", "t", "--body", "b", "--head", "feat/x",
                "--draft",
            ]
        );
    }

    #[test]
    fn merge_maps_method_and_flags() {
        let r: MergeRequest = serde_json::from_value(json!({
            "repo": "o/r", "number": 7, "method": "squash", "delete_branch": true
        }))
        .unwrap();
        assert_eq!(
            merge_args(&r),
            vec![
                "pr",
                "merge",
                "7",
                "-R",
                "o/r",
                "--squash",
                "--delete-branch"
            ]
        );
    }

    #[test]
    fn review_kebab_case_event() {
        let r: ReviewRequest = serde_json::from_value(json!({
            "repo": "o/r", "number": 7, "event": "request-changes", "body": "nit"
        }))
        .unwrap();
        assert_eq!(
            review_args(&r),
            vec![
                "pr",
                "review",
                "7",
                "-R",
                "o/r",
                "--request-changes",
                "--body",
                "nit"
            ]
        );
    }
}
