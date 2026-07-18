//! `github::repo::*` — typed wrappers over `gh repo …`. Unlike the other
//! groups, `gh repo view`/`list` take the repo/owner as a POSITIONAL argument
//! (`-R` is not a flag these subcommands accept).

use schemars::JsonSchema;
use serde::Deserialize;

use super::{argv, push_opt};

pub const VIEW_ID: &str = "github::repo::view";
pub const VIEW_DESC: &str = "View a repository's metadata (description, default branch, visibility, stars, language, topics, latest release): { repo: \"owner/name\" } -> { value }.";
pub const VIEW_JSON: &str = "name,nameWithOwner,description,url,defaultBranchRef,visibility,isPrivate,isFork,isArchived,stargazerCount,forkCount,primaryLanguage,repositoryTopics,latestRelease,pushedAt,updatedAt,homepageUrl";

pub const LIST_ID: &str = "github::repo::list";
pub const LIST_DESC: &str = "List repositories of an owner: { owner?, limit? } -> { value: [{name, nameWithOwner, description, url, visibility, ...}] }. owner defaults to the authenticated user.";
pub const LIST_JSON: &str = "name,nameWithOwner,description,url,visibility,isPrivate,isFork,isArchived,stargazerCount,primaryLanguage,pushedAt";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ViewRequest {
    /// Target repository, "owner/name".
    pub repo: String,
}

pub fn view_args(r: &ViewRequest) -> Vec<String> {
    argv(["repo", "view", r.repo.as_str(), "--json", VIEW_JSON])
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRequest {
    /// User or organization login (default: the authenticated user).
    pub owner: Option<String>,
    /// Maximum number of results (gh default: 30).
    pub limit: Option<u32>,
}

pub fn list_args(r: &ListRequest) -> Vec<String> {
    let mut a = argv(["repo", "list"]);
    if let Some(owner) = &r.owner {
        a.push(owner.clone());
    }
    a.push("--json".to_string());
    a.push(LIST_JSON.to_string());
    push_opt(&mut a, "--limit", r.limit);
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn view_is_positional_no_dash_r() {
        let r: ViewRequest = serde_json::from_value(json!({ "repo": "cli/cli" })).unwrap();
        assert_eq!(
            view_args(&r),
            vec!["repo", "view", "cli/cli", "--json", VIEW_JSON]
        );
    }

    #[test]
    fn list_owner_precedes_flags() {
        let r: ListRequest =
            serde_json::from_value(json!({ "owner": "iii-hq", "limit": 10 })).unwrap();
        assert_eq!(
            list_args(&r),
            vec!["repo", "list", "iii-hq", "--json", LIST_JSON, "--limit", "10"]
        );
    }
}
