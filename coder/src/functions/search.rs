//! `coder::search` — combined path + content search.
//!
//! Walks `base_path` with `walkdir`, filtering by include/exclude globs
//! and skipping non-accessible files entirely so the search can't reveal
//! their content. Path matches and content matches are reported in
//! separate arrays of one response.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::CoderConfig;
use crate::error::{err_to_string, CoderError};
use crate::path::PathResolver;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchInput {
    /// Pattern to search for. Treated as a regex when `regex: true`,
    /// otherwise as a literal substring.
    pub query: String,
    /// Folder, relative to `base_path`, scoping the walk. Defaults to `.`
    /// (the base itself). Globs and result `path`s remain anchored at
    /// `base_path` regardless of this value.
    #[serde(default = "default_path")]
    pub path: String,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub ignore_case: bool,
    /// Glob patterns (relative to `base_path`) that paths must match
    /// to be considered. Empty = include everything.
    #[serde(default)]
    pub include_globs: Vec<String>,
    /// Glob patterns (relative to `base_path`) that exclude matching paths.
    #[serde(default)]
    pub exclude_globs: Vec<String>,
    /// Optional explicit cap. Falls back to config when unset.
    #[serde(default)]
    pub max_matches: Option<u32>,
    /// Bytes per line to consider when scanning content; longer lines are
    /// truncated for the match snippet.
    #[serde(default)]
    pub max_line_bytes: Option<u32>,
    /// Search file contents (default true).
    #[serde(default = "default_true")]
    pub search_content: bool,
    /// Search file paths (default true).
    #[serde(default = "default_true")]
    pub search_paths: bool,
}

fn default_true() -> bool {
    true
}

fn default_path() -> String {
    ".".to_string()
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ContentMatch {
    pub path: String,
    pub line: u32,
    pub column: u32,
    /// Matched line; truncated to `max_line_bytes` and never spans newlines.
    pub text: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PathMatch {
    pub path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchOutput {
    pub content_matches: Vec<ContentMatch>,
    pub path_matches: Vec<PathMatch>,
    /// True if either match list was cut off at the configured cap.
    pub truncated: bool,
}

pub async fn handle(
    resolver: Arc<PathResolver>,
    cfg: Arc<CoderConfig>,
    req: SearchInput,
) -> Result<SearchOutput, String> {
    inner(&resolver, &cfg, req).map_err(err_to_string)
}

fn inner(
    resolver: &PathResolver,
    cfg: &CoderConfig,
    req: SearchInput,
) -> Result<SearchOutput, CoderError> {
    if req.query.is_empty() {
        return Err(CoderError::BadInput("query must not be empty".into()));
    }
    if !req.search_content && !req.search_paths {
        return Err(CoderError::BadInput(
            "at least one of search_content / search_paths must be true".into(),
        ));
    }
    let max_matches = req.max_matches.unwrap_or(cfg.search_default_max_matches) as usize;
    let max_line_bytes = req
        .max_line_bytes
        .unwrap_or(cfg.search_default_max_line_bytes) as usize;

    // Use `resolve` rather than `require_writable` so a search rooted at
    // a folder that *contains* non-accessible children still works; the
    // per-file `is_non_accessible` filter below still guards their bytes.
    let walk_root = resolver.resolve(&req.path)?;
    let md = std::fs::metadata(&walk_root)?;
    if !md.is_dir() {
        return Err(CoderError::BadInput(format!(
            "not a directory: {}",
            req.path
        )));
    }

    let include = build_globset(&req.include_globs)?;
    let exclude = build_globset(&req.exclude_globs)?;

    let content_matcher = if req.search_content {
        Some(build_matcher(&req.query, req.regex, req.ignore_case)?)
    } else {
        None
    };
    let path_matcher = if req.search_paths {
        Some(build_matcher(&req.query, req.regex, req.ignore_case)?)
    } else {
        None
    };

    let mut content_matches: Vec<ContentMatch> = Vec::new();
    let mut path_matches: Vec<PathMatch> = Vec::new();
    let mut truncated = false;

    let walker = walkdir::WalkDir::new(&walk_root).follow_links(false);
    for entry in walker.into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path();
        let Some(rel) = resolver.relative(abs) else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }
        if resolver.is_non_accessible(abs) {
            continue;
        }
        if let Some(set) = &include {
            if !set.is_match(&rel) {
                continue;
            }
        }
        if let Some(set) = &exclude {
            if set.is_match(&rel) {
                continue;
            }
        }

        if let Some(matcher) = &path_matcher {
            if matcher.is_match(&rel) {
                if path_matches.len() >= max_matches {
                    truncated = true;
                } else {
                    path_matches.push(PathMatch { path: rel.clone() });
                }
            }
        }

        if let Some(matcher) = &content_matcher {
            // Skip files larger than max_read_bytes during a search — we
            // don't want to load multi-GB blobs into memory by accident.
            if let Ok(md) = std::fs::metadata(abs) {
                if md.len() > cfg.max_read_bytes {
                    continue;
                }
            }
            let bytes = match std::fs::read(abs) {
                Ok(b) => b,
                Err(_) => continue,
            };
            // Cheap binary heuristic: presence of any NUL byte. Skip
            // binary files so the response stays human-readable.
            if bytes.contains(&0) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            for (line_idx, line) in text.lines().enumerate() {
                let truncated_line = if line.len() > max_line_bytes {
                    &line[..max_line_bytes]
                } else {
                    line
                };
                if let Some(m) = matcher.find(truncated_line) {
                    if content_matches.len() >= max_matches {
                        truncated = true;
                        break;
                    }
                    content_matches.push(ContentMatch {
                        path: rel.clone(),
                        line: (line_idx as u32) + 1,
                        column: (m.start as u32) + 1,
                        text: truncated_line.to_string(),
                    });
                }
            }
            if truncated {
                break;
            }
        }
    }

    Ok(SearchOutput {
        content_matches,
        path_matches,
        truncated,
    })
}

fn build_globset(patterns: &[String]) -> Result<Option<globset::GlobSet>, CoderError> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut b = globset::GlobSetBuilder::new();
    for p in patterns {
        let g = globset::Glob::new(p)
            .map_err(|e| CoderError::BadInput(format!("bad glob {p:?}: {e}")))?;
        b.add(g);
    }
    let set = b
        .build()
        .map_err(|e| CoderError::BadInput(format!("globset build failed: {e}")))?;
    Ok(Some(set))
}

/// Lightweight matcher abstraction — wraps either a `Regex` or a literal
/// substring search. Both report a 0-based column for the first match
/// position so the wire `column` can be 1-based.
enum Matcher {
    Regex(regex::Regex),
    Literal { needle: String, ignore_case: bool },
}

struct Match {
    start: usize,
}

impl Matcher {
    fn is_match(&self, hay: &str) -> bool {
        self.find(hay).is_some()
    }
    fn find(&self, hay: &str) -> Option<Match> {
        match self {
            Matcher::Regex(re) => re.find(hay).map(|m| Match { start: m.start() }),
            Matcher::Literal {
                needle,
                ignore_case,
            } => {
                if *ignore_case {
                    let hay_l = hay.to_lowercase();
                    let needle_l = needle.to_lowercase();
                    hay_l.find(&needle_l).map(|s| Match { start: s })
                } else {
                    hay.find(needle.as_str()).map(|s| Match { start: s })
                }
            }
        }
    }
}

fn build_matcher(query: &str, regex: bool, ignore_case: bool) -> Result<Matcher, CoderError> {
    if regex {
        let mut builder = regex::RegexBuilder::new(query);
        builder.case_insensitive(ignore_case);
        let re = builder
            .build()
            .map_err(|e| CoderError::BadInput(format!("bad regex {query:?}: {e}")))?;
        Ok(Matcher::Regex(re))
    } else {
        Ok(Matcher::Literal {
            needle: query.to_string(),
            ignore_case,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn setup() -> (tempfile::TempDir, Arc<PathResolver>, Arc<CoderConfig>) {
        let tmp = tempdir().unwrap();
        let cfg = CoderConfig {
            base_path: tmp.path().to_path_buf(),
            non_accessible_globs: vec!["**/.env".to_string()],
            max_read_bytes: 1024 * 1024,
            search_default_max_matches: 1000,
            search_default_max_line_bytes: 4096,
            ..CoderConfig::default()
        };
        let cfg = Arc::new(cfg);
        let resolver = Arc::new(PathResolver::new(&cfg).unwrap());
        (tmp, resolver, cfg)
    }

    fn write(tmp: &tempfile::TempDir, rel: &str, body: &str) {
        let p = tmp.path().join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, body).unwrap();
    }

    #[tokio::test]
    async fn literal_content_match_returns_line_column() {
        let (tmp, r, c) = setup();
        write(&tmp, "a.txt", "alpha\nbeta needle here\ngamma\n");
        let out = handle(
            r,
            c,
            SearchInput {
                query: "needle".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                search_content: true,
                search_paths: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 1);
        let m = &out.content_matches[0];
        assert_eq!(m.path, "a.txt");
        assert_eq!(m.line, 2);
        assert_eq!(m.column, 6);
    }

    #[tokio::test]
    async fn regex_content_match() {
        let (tmp, r, c) = setup();
        write(&tmp, "a.txt", "fn foo() {}\nfn Bar() {}\n");
        let out = handle(
            r,
            c,
            SearchInput {
                query: "^fn [A-Z]".into(),
                path: ".".into(),
                regex: true,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                search_content: true,
                search_paths: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 1);
        assert_eq!(out.content_matches[0].line, 2);
    }

    #[tokio::test]
    async fn path_match_returns_paths() {
        let (tmp, r, c) = setup();
        write(&tmp, "src/foo.rs", "x");
        write(&tmp, "src/bar.ts", "x");
        let out = handle(
            r,
            c,
            SearchInput {
                query: "foo".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                search_content: false,
                search_paths: true,
            },
        )
        .await
        .unwrap();
        let paths: Vec<_> = out.path_matches.iter().map(|p| p.path.as_str()).collect();
        assert!(paths.contains(&"src/foo.rs"));
        assert!(!paths.contains(&"src/bar.ts"));
    }

    #[tokio::test]
    async fn non_accessible_files_skipped() {
        let (tmp, r, c) = setup();
        write(&tmp, ".env", "API_KEY=needle");
        write(&tmp, "a.txt", "needle here");
        let out = handle(
            r,
            c,
            SearchInput {
                query: "needle".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                search_content: true,
                search_paths: true,
            },
        )
        .await
        .unwrap();
        // The .env file must not appear in either match list.
        for m in &out.content_matches {
            assert_ne!(m.path, ".env", "non-accessible file leaked content");
        }
        for m in &out.path_matches {
            assert_ne!(m.path, ".env", "non-accessible file leaked path");
        }
    }

    #[tokio::test]
    async fn include_and_exclude_globs() {
        let (tmp, r, c) = setup();
        write(&tmp, "src/a.rs", "needle");
        write(&tmp, "src/b.ts", "needle");
        write(&tmp, "build/c.rs", "needle");
        let out = handle(
            r,
            c,
            SearchInput {
                query: "needle".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec!["**/*.rs".into()],
                exclude_globs: vec!["build/**".into()],
                max_matches: None,
                max_line_bytes: None,
                search_content: true,
                search_paths: false,
            },
        )
        .await
        .unwrap();
        let paths: Vec<_> = out
            .content_matches
            .iter()
            .map(|m| m.path.as_str())
            .collect();
        assert_eq!(paths, vec!["src/a.rs"]);
    }

    #[tokio::test]
    async fn truncation_flag_when_cap_hit() {
        let (tmp, r, _c) = setup();
        for i in 0..5 {
            write(&tmp, &format!("f{i}.txt"), "needle\n");
        }
        let cfg = Arc::new(CoderConfig {
            base_path: tmp.path().to_path_buf(),
            non_accessible_globs: vec![],
            search_default_max_matches: 1000,
            max_read_bytes: 1024 * 1024,
            ..CoderConfig::default()
        });
        let out = handle(
            r,
            cfg,
            SearchInput {
                query: "needle".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: Some(2),
                max_line_bytes: None,
                search_content: true,
                search_paths: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 2);
        assert!(out.truncated);
    }

    #[tokio::test]
    async fn empty_query_rejected() {
        let (_tmp, r, c) = setup();
        let err = handle(
            r,
            c,
            SearchInput {
                query: "".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                search_content: true,
                search_paths: true,
            },
        )
        .await
        .unwrap_err();
        assert!(err.contains("C210"), "got: {err}");
    }

    #[tokio::test]
    async fn ignore_case_literal() {
        let (tmp, r, c) = setup();
        write(&tmp, "a.txt", "NEEDLE here");
        let out = handle(
            r,
            c,
            SearchInput {
                query: "needle".into(),
                path: ".".into(),
                regex: false,
                ignore_case: true,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                search_content: true,
                search_paths: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content_matches.len(), 1);
    }

    #[tokio::test]
    async fn skips_binary_files() {
        let (tmp, r, c) = setup();
        std::fs::write(tmp.path().join("blob.bin"), [0u8, 1, 2, 3, b'n', b'e']).unwrap();
        let out = handle(
            r,
            c,
            SearchInput {
                query: "ne".into(),
                path: ".".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                search_content: true,
                search_paths: false,
            },
        )
        .await
        .unwrap();
        assert!(out.content_matches.is_empty());
    }

    #[tokio::test]
    async fn path_scopes_walk_to_subdir() {
        let (tmp, r, c) = setup();
        write(&tmp, "a/x.txt", "needle here");
        write(&tmp, "b/y.txt", "needle here");
        let out = handle(
            r,
            c,
            SearchInput {
                query: "needle".into(),
                path: "a".into(),
                regex: false,
                ignore_case: false,
                include_globs: vec![],
                exclude_globs: vec![],
                max_matches: None,
                max_line_bytes: None,
                search_content: true,
                search_paths: false,
            },
        )
        .await
        .unwrap();
        let paths: Vec<_> = out
            .content_matches
            .iter()
            .map(|m| m.path.as_str())
            .collect();
        assert_eq!(paths, vec!["a/x.txt"]);
    }
}
