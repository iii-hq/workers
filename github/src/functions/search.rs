//! `github::search::*` — typed wrappers over `gh search …`. No `-R` flag:
//! repo scoping lives in the query itself (e.g. "repo:owner/name fix").

use schemars::JsonSchema;
use serde::Deserialize;

use super::{argv, push_opt};

pub const REPOS_ID: &str = "github::search::repos";
pub const REPOS_DESC: &str = "Search repositories: { query, limit? } -> { value: [{fullName, description, url, visibility, stargazersCount, language, updatedAt, ...}] }. GitHub search syntax (e.g. \"language:rust stars:>100 topic:cli\").";
pub const REPOS_JSON: &str =
    "fullName,description,url,visibility,isArchived,isFork,stargazersCount,forksCount,language,updatedAt";

pub const ISSUES_ID: &str = "github::search::issues";
pub const ISSUES_DESC: &str = "Search issues across repositories: { query, limit? } -> { value: [{number, title, state, url, repository, author, labels, commentsCount, ...}] }. Use qualifiers like \"repo:o/r is:open label:bug\".";
pub const ISSUES_JSON: &str =
    "number,title,state,url,repository,author,labels,commentsCount,createdAt,updatedAt";

pub const PRS_ID: &str = "github::search::prs";
pub const PRS_DESC: &str = "Search pull requests across repositories: { query, limit? } -> { value }. Use qualifiers like \"repo:o/r is:open review:required\".";
pub const PRS_JSON: &str =
    "number,title,state,url,repository,author,labels,commentsCount,createdAt,updatedAt,isDraft";

pub const CODE_ID: &str = "github::search::code";
pub const CODE_DESC: &str = "Search code: { query, limit? } -> { value: [{path, repository, sha, url, textMatches}] }. Use qualifiers like \"repo:o/r language:rust symbol\".";
pub const CODE_JSON: &str = "path,repository,sha,url,textMatches";

macro_rules! search_request {
    ($name:ident) => {
        #[derive(Debug, Deserialize, JsonSchema)]
        pub struct $name {
            /// Search query (GitHub search syntax, qualifiers included).
            pub query: String,
            /// Maximum number of results (gh default: 30).
            pub limit: Option<u32>,
        }
    };
}

search_request!(ReposRequest);
search_request!(IssuesRequest);
search_request!(PrsRequest);
search_request!(CodeRequest);

fn search_args(kind: &str, json: &str, query: &str, limit: Option<u32>) -> Vec<String> {
    let mut a = argv(["search", kind, "--json", json]);
    push_opt(&mut a, "--limit", limit);
    // `--` ends option parsing: a query starting with a negative qualifier
    // (e.g. "-label:bug") would otherwise be swallowed as a flag by gh.
    a.push("--".to_string());
    a.push(query.to_string());
    a
}

pub fn repos_args(r: &ReposRequest) -> Vec<String> {
    search_args("repos", REPOS_JSON, &r.query, r.limit)
}

pub fn issues_args(r: &IssuesRequest) -> Vec<String> {
    search_args("issues", ISSUES_JSON, &r.query, r.limit)
}

pub fn prs_args(r: &PrsRequest) -> Vec<String> {
    search_args("prs", PRS_JSON, &r.query, r.limit)
}

pub fn code_args(r: &CodeRequest) -> Vec<String> {
    search_args("code", CODE_JSON, &r.query, r.limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn code_search_maps_query_and_limit() {
        let r: CodeRequest = serde_json::from_value(json!({
            "query": "repo:cli/cli read_bounded", "limit": 5
        }))
        .unwrap();
        assert_eq!(
            code_args(&r),
            vec![
                "search",
                "code",
                "--json",
                CODE_JSON,
                "--limit",
                "5",
                "--",
                "repo:cli/cli read_bounded",
            ]
        );
    }

    #[test]
    fn leading_negative_qualifier_stays_positional() {
        let r: ReposRequest = serde_json::from_value(json!({
            "query": "-topic:python cli"
        }))
        .unwrap();
        assert_eq!(
            repos_args(&r),
            vec![
                "search",
                "repos",
                "--json",
                REPOS_JSON,
                "--",
                "-topic:python cli"
            ]
        );
    }
}
