//! `github::issue::*` — typed wrappers over `gh issue …`.

use schemars::JsonSchema;
use serde::Deserialize;

use super::{argv, push_each, push_opt};

pub const LIST_ID: &str = "github::issue::list";
pub const LIST_DESC: &str = "List issues: { repo: \"owner/name\", state?, limit?, author?, labels?, assignee?, search? } -> { value: [{number, title, state, url, author, labels, assignees, milestone, createdAt, updatedAt}] }.";
pub const LIST_JSON: &str =
    "number,title,state,url,author,labels,assignees,milestone,createdAt,updatedAt";

pub const VIEW_ID: &str = "github::issue::view";
pub const VIEW_DESC: &str =
    "View one issue with body and comments: { repo: \"owner/name\", number } -> { value }.";
pub const VIEW_JSON: &str = "number,title,state,url,author,labels,assignees,milestone,createdAt,updatedAt,body,stateReason,comments,closedAt";

pub const CREATE_ID: &str = "github::issue::create";
pub const CREATE_DESC: &str = "Open an issue: { repo: \"owner/name\", title, body, labels?, assignees? } -> { output: <issue url> }.";

pub const EDIT_ID: &str = "github::issue::edit";
pub const EDIT_DESC: &str = "Edit an issue's title/body/labels/assignees: { repo, number, title?, body?, add_labels?, remove_labels?, add_assignees? } -> { output }.";

pub const COMMENT_ID: &str = "github::issue::comment";
pub const COMMENT_DESC: &str =
    "Comment on an issue: { repo, number, body } -> { output: <comment url> }.";

pub const CLOSE_ID: &str = "github::issue::close";
pub const CLOSE_DESC: &str =
    "Close an issue: { repo, number, comment?, reason?: completed|not-planned } -> { output }.";

/// Issue state filter.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum IssueState {
    Open,
    Closed,
    All,
}

impl IssueState {
    fn as_str(self) -> &'static str {
        match self {
            IssueState::Open => "open",
            IssueState::Closed => "closed",
            IssueState::All => "all",
        }
    }
}

/// Close reason. The wire value is kebab-case; gh's flag value for
/// `not-planned` is the space-separated "not planned".
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CloseReason {
    Completed,
    NotPlanned,
}

impl CloseReason {
    fn as_str(self) -> &'static str {
        match self {
            CloseReason::Completed => "completed",
            CloseReason::NotPlanned => "not planned",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Filter by state (gh default: open).
    pub state: Option<IssueState>,
    /// Maximum number of results (gh default: 30).
    pub limit: Option<u32>,
    /// Filter by author login.
    pub author: Option<String>,
    /// Filter by labels (all must match).
    pub labels: Option<Vec<String>>,
    /// Filter by assignee login.
    pub assignee: Option<String>,
    /// Search query narrowing the list further (GitHub search syntax).
    pub search: Option<String>,
}

pub fn list_args(r: &ListRequest) -> Vec<String> {
    let mut a = argv(["issue", "list", "-R", r.repo.as_str(), "--json", LIST_JSON]);
    push_opt(&mut a, "--state", r.state.map(|s| s.as_str()));
    push_opt(&mut a, "--limit", r.limit);
    push_opt(&mut a, "--author", r.author.as_deref());
    push_each(&mut a, "--label", &r.labels);
    push_opt(&mut a, "--assignee", r.assignee.as_deref());
    push_opt(&mut a, "--search", r.search.as_deref());
    a
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ViewRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Issue number.
    pub number: u64,
}

pub fn view_args(r: &ViewRequest) -> Vec<String> {
    argv([
        "issue",
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
    /// Issue title.
    pub title: String,
    /// Issue body (markdown).
    pub body: String,
    /// Labels to apply.
    pub labels: Option<Vec<String>>,
    /// Assignee logins.
    pub assignees: Option<Vec<String>>,
}

pub fn create_args(r: &CreateRequest) -> Vec<String> {
    let mut a = argv([
        "issue",
        "create",
        "-R",
        r.repo.as_str(),
        "--title",
        r.title.as_str(),
        "--body",
        r.body.as_str(),
    ]);
    push_each(&mut a, "--label", &r.labels);
    push_each(&mut a, "--assignee", &r.assignees);
    a
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EditRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Issue number.
    pub number: u64,
    /// New title.
    pub title: Option<String>,
    /// New body (markdown).
    pub body: Option<String>,
    /// Labels to add.
    pub add_labels: Option<Vec<String>>,
    /// Labels to remove.
    pub remove_labels: Option<Vec<String>>,
    /// Assignee logins to add.
    pub add_assignees: Option<Vec<String>>,
}

pub fn edit_args(r: &EditRequest) -> Vec<String> {
    let mut a = argv([
        "issue",
        "edit",
        r.number.to_string().as_str(),
        "-R",
        r.repo.as_str(),
    ]);
    push_opt(&mut a, "--title", r.title.as_deref());
    push_opt(&mut a, "--body", r.body.as_deref());
    push_each(&mut a, "--add-label", &r.add_labels);
    push_each(&mut a, "--remove-label", &r.remove_labels);
    push_each(&mut a, "--add-assignee", &r.add_assignees);
    a
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommentRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Issue number.
    pub number: u64,
    /// Comment body (markdown).
    pub body: String,
}

pub fn comment_args(r: &CommentRequest) -> Vec<String> {
    argv([
        "issue",
        "comment",
        r.number.to_string().as_str(),
        "-R",
        r.repo.as_str(),
        "--body",
        r.body.as_str(),
    ])
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CloseRequest {
    /// Target repository, "owner/name".
    pub repo: String,
    /// Issue number.
    pub number: u64,
    /// Closing comment.
    pub comment: Option<String>,
    /// Close reason.
    pub reason: Option<CloseReason>,
}

pub fn close_args(r: &CloseRequest) -> Vec<String> {
    let mut a = argv([
        "issue",
        "close",
        r.number.to_string().as_str(),
        "-R",
        r.repo.as_str(),
    ]);
    push_opt(&mut a, "--comment", r.comment.as_deref());
    push_opt(&mut a, "--reason", r.reason.map(|x| x.as_str()));
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn close_maps_not_planned_to_the_spaced_flag_value() {
        let r: CloseRequest = serde_json::from_value(json!({
            "repo": "o/r", "number": 3, "reason": "not-planned"
        }))
        .unwrap();
        assert_eq!(
            close_args(&r),
            vec![
                "issue",
                "close",
                "3",
                "-R",
                "o/r",
                "--reason",
                "not planned"
            ]
        );
    }

    #[test]
    fn create_repeats_labels_and_assignees() {
        let r: CreateRequest = serde_json::from_value(json!({
            "repo": "o/r", "title": "t", "body": "b",
            "labels": ["bug"], "assignees": ["octocat", "hubot"]
        }))
        .unwrap();
        assert_eq!(
            create_args(&r),
            vec![
                "issue",
                "create",
                "-R",
                "o/r",
                "--title",
                "t",
                "--body",
                "b",
                "--label",
                "bug",
                "--assignee",
                "octocat",
                "--assignee",
                "hubot",
            ]
        );
    }
}
