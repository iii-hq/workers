//! `database::formatSql` — pretty-print SQL (the `sqlformat` crate).
//!
//! Statements and comments are preserved; only whitespace and keyword case
//! change. Spans the formatter does not understand are protected first and
//! put back verbatim afterwards: postgres dollar-quoted bodies (a function
//! body is code in another language) and sqlite `[bracket identifiers]`
//! (the formatter would pad them with spaces, renaming the column).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlformat::{Dialect, FormatOptions, Indent, QueryParams};

use super::query::err_to_str;
use super::AppState;
use crate::config::DriverKind;
use crate::error::DbError;
use crate::pool::Pool;

/// Longest input accepted (formatting is linear, but a megabyte of SQL in a
/// console editor is a mistake, not a query).
const MAX_SQL_LEN: usize = 1_000_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FormatSqlReq {
    pub sql: String,
    /// A configured database whose driver picks the dialect.
    #[serde(default)]
    pub db: Option<String>,
    /// The dialect directly: `postgres`, `mysql` or `sqlite` (wins over `db`).
    #[serde(default)]
    pub driver: Option<DriverKind>,
    /// Uppercase keywords. Default true.
    #[serde(default)]
    pub uppercase: Option<bool>,
    /// Spaces per indent level, 1-8. Default 2.
    #[serde(default)]
    pub indent: Option<u8>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FormatSqlResp {
    pub sql: String,
}

pub async fn handle(state: &AppState, req: FormatSqlReq) -> Result<FormatSqlResp, String> {
    if req.sql.len() > MAX_SQL_LEN {
        return Err(err_to_str(DbError::InvalidParam {
            index: 0,
            reason: format!("sql is longer than {MAX_SQL_LEN} bytes"),
        }));
    }
    let driver = match (req.driver, &req.db) {
        (Some(driver), _) => Some(driver),
        (None, Some(db)) => Some(match state.pool(db).await.map_err(err_to_str)? {
            Pool::Sqlite(_) => DriverKind::Sqlite,
            Pool::Postgres(_) => DriverKind::Postgres,
            Pool::Mysql(_) => DriverKind::Mysql,
        }),
        (None, None) => None,
    };
    Ok(FormatSqlResp {
        sql: format(
            &req.sql,
            driver,
            req.uppercase.unwrap_or(true),
            req.indent.unwrap_or(2).clamp(1, 8),
        ),
    })
}

/// Format `sql` for `driver` (generic SQL when `None`).
pub fn format(sql: &str, driver: Option<DriverKind>, uppercase: bool, indent: u8) -> String {
    if sql.trim().is_empty() {
        return sql.to_string();
    }
    let (protected, kept) = protect(sql, driver);
    let options = FormatOptions {
        indent: Indent::Spaces(indent),
        uppercase: Some(uppercase),
        // A blank line between statements.
        lines_between_queries: 2,
        dialect: match driver {
            Some(DriverKind::Postgres) => Dialect::PostgreSql,
            _ => Dialect::Generic,
        },
        ..Default::default()
    };
    let formatted = sqlformat::format(&protected, &QueryParams::None, &options);
    restore(&formatted, &kept)
}

const MARK: &str = "__iii_fmt_";

fn placeholder(n: usize) -> String {
    format!("'{MARK}{n}__'")
}

/// Replace the spans the formatter would damage with string-literal
/// placeholders (which it keeps verbatim).
fn protect(sql: &str, driver: Option<DriverKind>) -> (String, Vec<String>) {
    if sql.contains(MARK) {
        return (sql.to_string(), Vec::new());
    }
    let b = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut kept = Vec::new();
    let mut i = 0usize;
    let mut copied = 0usize;
    // Skip a span closed by `close`, doubling escaping it.
    let skip_quoted = |i: usize, close: u8| -> usize {
        let mut j = i + 1;
        while j < b.len() {
            if b[j] == close {
                if j + 1 < b.len() && b[j + 1] == close {
                    j += 2;
                    continue;
                }
                return j + 1;
            }
            if b[j] == b'\\' && close == b'\'' && driver == Some(DriverKind::Mysql) {
                j += 2;
                continue;
            }
            j += 1;
        }
        b.len()
    };
    while i < b.len() {
        match b[i] {
            b'\'' => i = skip_quoted(i, b'\''),
            b'"' => i = skip_quoted(i, b'"'),
            b'`' => i = skip_quoted(i, b'`'),
            b'-' if b.get(i + 1) == Some(&b'-') => {
                i = sql[i..].find('\n').map_or(b.len(), |p| i + p);
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = sql[i + 2..].find("*/").map_or(b.len(), |p| i + 2 + p + 2);
            }
            b'[' if driver == Some(DriverKind::Sqlite) => {
                let end = sql[i..].find(']').map_or(b.len(), |p| i + p + 1);
                out.push_str(&sql[copied..i]);
                out.push_str(&placeholder(kept.len()));
                kept.push(sql[i..end].to_string());
                i = end;
                copied = end;
            }
            b'$' if driver == Some(DriverKind::Postgres)
                && !(i > 0
                    && (b[i - 1].is_ascii_alphanumeric()
                        || b[i - 1] == b'_'
                        || b[i - 1] == b'$')) =>
            {
                let tag_end = b[i + 1..]
                    .iter()
                    .position(|&d| !(d.is_ascii_alphanumeric() || d == b'_'))
                    .map(|p| i + 1 + p);
                match tag_end {
                    Some(end)
                        if b[end] == b'$' && !b.get(i + 1).is_some_and(|d| d.is_ascii_digit()) =>
                    {
                        let tag = &sql[i..=end];
                        match sql[end + 1..].find(tag) {
                            Some(close) => {
                                let span_end = end + 1 + close + tag.len();
                                out.push_str(&sql[copied..i]);
                                out.push_str(&placeholder(kept.len()));
                                kept.push(sql[i..span_end].to_string());
                                i = span_end;
                                copied = span_end;
                            }
                            None => i = b.len(),
                        }
                    }
                    _ => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    out.push_str(&sql[copied..]);
    (out, kept)
}

fn restore(formatted: &str, kept: &[String]) -> String {
    let mut out = formatted.to_string();
    // Highest index first so `…_1__` never matches inside `…_11__`.
    for (n, original) in kept.iter().enumerate().rev() {
        out = out.replacen(&placeholder(n), original, 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_keywords_and_keeps_comments_and_statements() {
        let out = format(
            "select a, b from t where x = 1 -- why\n/* keep */ ; select 2",
            None,
            true,
            2,
        );
        assert!(out.contains("SELECT"), "{out}");
        assert!(out.contains("-- why"), "{out}");
        assert!(out.contains("/* keep */"), "{out}");
        assert_eq!(out.matches("SELECT").count(), 2, "{out}");
        assert!(out.contains("FROM\n  t"), "{out}");
    }

    #[test]
    fn lowercase_and_indent_are_honoured() {
        let out = format("SELECT a FROM t", None, false, 4);
        assert!(out.contains("select") && out.contains("\n    a"), "{out}");
    }

    #[test]
    fn postgres_casts_placeholders_and_dollar_bodies_survive() {
        let sql = "select a::int, $1 from t; create function f() returns int as $body$ select  1;  select 2 $body$ language sql";
        let out = format(sql, Some(DriverKind::Postgres), true, 2);
        assert!(out.contains("a::int"), "{out}");
        assert!(out.contains("$1"), "{out}");
        assert!(out.contains("$body$ select  1;  select 2 $body$"), "{out}");
    }

    #[test]
    fn sqlite_bracket_identifiers_are_not_padded() {
        let out = format("select [col x] from t", Some(DriverKind::Sqlite), true, 2);
        assert!(out.contains("[col x]"), "{out}");
    }

    #[test]
    fn mysql_hash_comments_and_backticks_survive() {
        let out = format(
            "select `a b` from t # note\nwhere c = 'x\\'y'",
            Some(DriverKind::Mysql),
            true,
            2,
        );
        assert!(out.contains("`a b`"), "{out}");
        assert!(out.contains("# note"), "{out}");
        assert!(out.contains("'x\\'y'"), "{out}");
    }

    #[test]
    fn empty_input_is_returned_as_is() {
        assert_eq!(format("  \n", None, true, 2), "  \n");
    }
}
