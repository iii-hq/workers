//! `database::formatSql` — pretty-print SQL (the `sqlformat` crate).
//!
//! Statements and comments are preserved; only whitespace and keyword case
//! change. Spans the formatter does not understand are protected first and
//! put back verbatim afterwards: postgres dollar-quoted bodies (a function
//! body is code in another language) and sqlite `[bracket identifiers]`
//! (the formatter would pad them with spaces, renaming the column).
//!
//! Keyword case is applied here, not by the formatter: its keyword list
//! includes words that are routinely column names (`status`, `name`,
//! `type`, `date`, `key`), and uppercasing those renames a quoted-less
//! identifier in a reader's eyes. Only reserved words in [`RESERVED`] change
//! case; every identifier, function name and non-reserved keyword keeps the
//! case it was written in.

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
    let sql = format(
        &req.sql,
        driver,
        req.uppercase.unwrap_or(true),
        req.indent.unwrap_or(2).clamp(1, 8),
    )
    .map_err(|reason| err_to_str(DbError::InvalidParam { index: 0, reason }))?;
    Ok(FormatSqlResp { sql })
}

/// Deepest parenthesis nesting formatted. Each level indents every line
/// under it, so output grows with depth × lines; past this the input is
/// almost certainly generated, not written.
const MAX_DEPTH: usize = 100;
/// Ceiling on the formatted size, checked against an upper bound BEFORE
/// formatting so a hostile input never gets to allocate it.
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

/// Format `sql` for `driver` (generic SQL when `None`). Literal bytes —
/// strings, quoted identifiers, dollar-quoted bodies, sqlite brackets — are
/// never changed. Refused: NUL bytes, nesting deeper than [`MAX_DEPTH`], and
/// input whose formatted size could exceed [`MAX_OUTPUT_BYTES`].
pub fn format(
    sql: &str,
    driver: Option<DriverKind>,
    uppercase: bool,
    indent: u8,
) -> Result<String, String> {
    if sql.contains('\0') {
        return Err("sql contains a NUL byte".into());
    }
    if sql.trim().is_empty() {
        return Ok(sql.to_string());
    }
    let protected = protect(sql, driver);
    if protected.max_depth > MAX_DEPTH {
        return Err(format!(
            "parentheses nest {} deep; formatSql formats up to {MAX_DEPTH} levels",
            protected.max_depth
        ));
    }
    // Every token can land on its own line, indented once per level, and
    // operators gain a space on each side: an upper bound on the output.
    let bound = protected
        .text
        .len()
        .saturating_mul(3 + protected.max_depth * usize::from(indent));
    if bound > MAX_OUTPUT_BYTES {
        return Err(format!(
            "this statement could format to more than {} MB; format a smaller piece",
            MAX_OUTPUT_BYTES / (1024 * 1024)
        ));
    }
    let options = FormatOptions {
        indent: Indent::Spaces(indent),
        // Case is ours to apply (see the module doc): keep everything as
        // written here.
        uppercase: None,
        // A blank line between statements.
        lines_between_queries: 2,
        dialect: match driver {
            Some(DriverKind::Postgres) => Dialect::PostgreSql,
            _ => Dialect::Generic,
        },
        ..Default::default()
    };
    let formatted = sqlformat::format(&protected.text, &QueryParams::None, &options);
    Ok(restore(
        &recase(&formatted, driver, uppercase),
        &protected.kept,
    ))
}

const MARK: &str = "__iii_fmt_";

fn placeholder(n: usize) -> String {
    format!("'{MARK}{n}__'")
}

struct Protected {
    /// The input with every literal replaced by a placeholder.
    text: String,
    /// The literals, by placeholder number.
    kept: Vec<String>,
    /// Deepest parenthesis nesting outside literals and comments.
    max_depth: usize,
}

/// Replace every literal — string (with its `E`/`N`/`X`/`B`/`U&` prefix),
/// quoted identifier, dollar-quoted body, sqlite bracket — with a
/// string-literal placeholder the formatter keeps verbatim, lexing quotes the
/// way the driver does (`E'…'` and mysql strings take backslash escapes). An
/// unterminated literal runs to the end and is kept as it is.
fn protect(sql: &str, driver: Option<DriverKind>) -> Protected {
    let b = sql.as_bytes();
    if sql.contains(MARK) {
        return Protected {
            text: sql.to_string(),
            kept: Vec::new(),
            max_depth: paren_depth(sql),
        };
    }
    let mut out = String::with_capacity(sql.len());
    let mut kept = Vec::new();
    let mut copied = 0usize;
    let mut i = 0usize;
    let (mut depth, mut max_depth) = (0usize, 0usize);
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80;
    // End (exclusive) of a quoted span starting at `start`, closed by
    // `close`; doubling escapes it, and so does a backslash when `backslash`.
    let span_end = |start: usize, close: u8, backslash: bool| -> usize {
        let mut j = start + 1;
        while j < b.len() {
            if backslash && b[j] == b'\\' {
                j += 2;
                continue;
            }
            if b[j] == close {
                if j + 1 < b.len() && b[j + 1] == close {
                    j += 2;
                    continue;
                }
                return j + 1;
            }
            j += 1;
        }
        b.len()
    };
    let mut keep = |from: usize, to: usize, out: &mut String, copied: &mut usize| {
        out.push_str(&sql[*copied..from]);
        out.push_str(&placeholder(kept.len()));
        kept.push(sql[from..to].to_string());
        *copied = to;
    };
    while i < b.len() {
        match b[i] {
            b'\'' => {
                // A one-letter prefix (E, N, X, B) or `U&` belongs to the
                // literal, unless it ends a longer identifier.
                let mut from = i;
                if i >= 1
                    && matches!(
                        b[i - 1],
                        b'e' | b'E' | b'n' | b'N' | b'x' | b'X' | b'b' | b'B'
                    )
                    && (i < 2 || !ident(b[i - 2]))
                {
                    from = i - 1;
                } else if i >= 2
                    && b[i - 1] == b'&'
                    && matches!(b[i - 2], b'u' | b'U')
                    && (i < 3 || !ident(b[i - 3]))
                {
                    from = i - 2;
                }
                let e_string = driver == Some(DriverKind::Postgres)
                    && from + 1 == i
                    && matches!(b[from], b'e' | b'E');
                let backslash = e_string || driver == Some(DriverKind::Mysql);
                let end = span_end(i, b'\'', backslash);
                // The prefix was already copied as code: take it back.
                if from < i && copied <= from {
                    keep(from, end, &mut out, &mut copied);
                } else {
                    keep(i, end, &mut out, &mut copied);
                }
                i = end;
            }
            b'"' => {
                let end = span_end(i, b'"', driver == Some(DriverKind::Mysql));
                keep(i, end, &mut out, &mut copied);
                i = end;
            }
            b'`' => {
                let end = span_end(i, b'`', false);
                keep(i, end, &mut out, &mut copied);
                i = end;
            }
            b'[' if driver == Some(DriverKind::Sqlite) => {
                let end = sql[i..].find(']').map_or(b.len(), |p| i + p + 1);
                keep(i, end, &mut out, &mut copied);
                i = end;
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                i = sql[i..].find('\n').map_or(b.len(), |p| i + p);
            }
            b'#' if driver == Some(DriverKind::Mysql) => {
                i = sql[i..].find('\n').map_or(b.len(), |p| i + p);
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = sql[i + 2..].find("*/").map_or(b.len(), |p| i + 2 + p + 2);
            }
            b'(' => {
                depth += 1;
                max_depth = max_depth.max(depth);
                i += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            b'$' if driver == Some(DriverKind::Postgres) && !(i > 0 && ident(b[i - 1])) => {
                let tag_end = b[i + 1..]
                    .iter()
                    .position(|&d| !(d.is_ascii_alphanumeric() || d == b'_'))
                    .map(|p| i + 1 + p);
                match tag_end {
                    Some(end)
                        if b[end] == b'$' && !b.get(i + 1).is_some_and(|d| d.is_ascii_digit()) =>
                    {
                        let tag = &sql[i..=end];
                        let close = sql[end + 1..]
                            .find(tag)
                            .map_or(b.len(), |c| end + 1 + c + tag.len());
                        keep(i, close, &mut out, &mut copied);
                        i = close;
                    }
                    _ => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    out.push_str(&sql[copied..]);
    Protected {
        text: out,
        kept,
        max_depth,
    }
}

fn paren_depth(sql: &str) -> usize {
    let (mut depth, mut max) = (0usize, 0usize);
    for c in sql.bytes() {
        match c {
            b'(' => {
                depth += 1;
                max = max.max(depth);
            }
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max
}

/// Words that are reserved (or structural) in SQL on every supported driver
/// and are not plausible column names. Changing their case never changes
/// what an identifier means.
const RESERVED: &[&str] = &[
    "ADD",
    "ALL",
    "ALTER",
    "AND",
    "ANY",
    "AS",
    "ASC",
    "BETWEEN",
    "BIGINT",
    "BOOLEAN",
    "BY",
    "CASCADE",
    "CASE",
    "CAST",
    "CHECK",
    "COLLATE",
    "COLUMN",
    "COMMIT",
    "CONSTRAINT",
    "CREATE",
    "CROSS",
    "CURRENT_DATE",
    "CURRENT_TIME",
    "CURRENT_TIMESTAMP",
    "CURRENT_USER",
    "DECIMAL",
    "DEFAULT",
    "DELETE",
    "DESC",
    "DISTINCT",
    "DROP",
    "ELSE",
    "END",
    "ESCAPE",
    "EXCEPT",
    "EXISTS",
    "EXPLAIN",
    "FALSE",
    "FETCH",
    "FOR",
    "FOREIGN",
    "FROM",
    "FULL",
    "FUNCTION",
    "GRANT",
    "GROUP",
    "HAVING",
    "IF",
    "ILIKE",
    "IN",
    "INDEX",
    "INNER",
    "INSERT",
    "INTEGER",
    "INTERSECT",
    "INTERVAL",
    "INTO",
    "IS",
    "JOIN",
    "LATERAL",
    "LEFT",
    "LIKE",
    "LIMIT",
    "MATERIALIZED",
    "NATURAL",
    "NOT",
    "NULL",
    "NUMERIC",
    "OFFSET",
    "ON",
    "OR",
    "ORDER",
    "OUTER",
    "OVER",
    "PARTITION",
    "PRIMARY",
    "PROCEDURE",
    "RECURSIVE",
    "REFERENCES",
    "REPLACE",
    "RESTRICT",
    "RETURNING",
    "RETURNS",
    "REVOKE",
    "RIGHT",
    "ROLLBACK",
    "SELECT",
    "SET",
    "SMALLINT",
    "SOME",
    "TABLE",
    "TEMPORARY",
    "THEN",
    "TRIGGER",
    "TRUE",
    "TRUNCATE",
    "UNION",
    "UNIQUE",
    "UPDATE",
    "USING",
    "VALUES",
    "VARCHAR",
    "VIEW",
    "WHEN",
    "WHERE",
    "WINDOW",
    "WITH",
];

/// Uppercase (or lowercase) the [`RESERVED`] words outside quotes and
/// comments. `KEY` changes only after `PRIMARY` / `FOREIGN` (it is a common
/// column name). A word after `.` is a qualified name, left alone.
fn recase(sql: &str, driver: Option<DriverKind>, upper: bool) -> String {
    let b = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0usize;
    let mut prev_word = String::new();
    let skip_quoted = |start: usize, close: u8| -> usize {
        let mut j = start + 1;
        while j < b.len() {
            if b[j] == b'\\' && close == b'\'' && driver == Some(DriverKind::Mysql) {
                j += 2;
                continue;
            }
            if b[j] == close {
                if j + 1 < b.len() && b[j + 1] == close {
                    j += 2;
                    continue;
                }
                return j + 1;
            }
            j += 1;
        }
        b.len()
    };
    while i < b.len() {
        let c = b[i];
        let end = match c {
            b'\'' | b'"' | b'`' => skip_quoted(i, c),
            b'[' if driver == Some(DriverKind::Sqlite) => {
                sql[i..].find(']').map_or(b.len(), |p| i + p + 1)
            }
            b'-' if b.get(i + 1) == Some(&b'-') => sql[i..].find('\n').map_or(b.len(), |p| i + p),
            b'#' if driver == Some(DriverKind::Mysql) => {
                sql[i..].find('\n').map_or(b.len(), |p| i + p)
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                sql[i + 2..].find("*/").map_or(b.len(), |p| i + 2 + p + 2)
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let mut j = i;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b'$')
                {
                    j += 1;
                }
                let word = &sql[i..j];
                let upper_word = word.to_ascii_uppercase();
                let qualified = i > 0 && b[i - 1] == b'.';
                let reserved = RESERVED.binary_search(&upper_word.as_str()).is_ok()
                    || (upper_word == "KEY" && matches!(prev_word.as_str(), "PRIMARY" | "FOREIGN"));
                if reserved && !qualified {
                    if upper {
                        out.push_str(&upper_word);
                    } else {
                        out.push_str(&word.to_ascii_lowercase());
                    }
                } else {
                    out.push_str(word);
                }
                prev_word = upper_word;
                i = j;
                continue;
            }
            _ => {
                // A multibyte character is copied whole.
                let ch = sql[i..].chars().next().map_or(1, char::len_utf8);
                i + ch
            }
        };
        out.push_str(&sql[i..end]);
        i = end;
    }
    out
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
        )
        .unwrap();
        assert!(out.contains("SELECT"), "{out}");
        assert!(out.contains("-- why"), "{out}");
        assert!(out.contains("/* keep */"), "{out}");
        assert_eq!(out.matches("SELECT").count(), 2, "{out}");
        assert!(out.contains("FROM\n  t"), "{out}");
    }

    #[test]
    fn lowercase_and_indent_are_honoured() {
        let out = format("SELECT a FROM t", None, false, 4).unwrap();
        assert!(out.contains("select") && out.contains("\n    a"), "{out}");
    }

    #[test]
    fn postgres_casts_placeholders_and_dollar_bodies_survive() {
        let sql = "select a::int, $1 from t; create function f() returns int as $body$ select  1;  select 2 $body$ language sql";
        let out = format(sql, Some(DriverKind::Postgres), true, 2).unwrap();
        assert!(out.contains("a::int"), "{out}");
        assert!(out.contains("$1"), "{out}");
        assert!(out.contains("$body$ select  1;  select 2 $body$"), "{out}");
    }

    #[test]
    fn sqlite_bracket_identifiers_are_not_padded() {
        let out = format("select [col x] from t", Some(DriverKind::Sqlite), true, 2).unwrap();
        assert!(out.contains("[col x]"), "{out}");
    }

    #[test]
    fn mysql_hash_comments_and_backticks_survive() {
        let out = format(
            "select `a b` from t # note\nwhere c = 'x\\'y'",
            Some(DriverKind::Mysql),
            true,
            2,
        )
        .unwrap();
        assert!(out.contains("`a b`"), "{out}");
        assert!(out.contains("# note"), "{out}");
        assert!(out.contains("'x\\'y'"), "{out}");
    }

    #[test]
    fn identifiers_that_are_non_reserved_keywords_keep_their_case() {
        let out = format(
            "select status, name, type, key, date from orders where status = 'paid' order by date",
            Some(DriverKind::Postgres),
            true,
            2,
        )
        .unwrap();
        assert!(out.contains("  status,"), "{out}");
        assert!(out.contains("  name,"), "{out}");
        assert!(out.contains("  key,"), "{out}");
        assert!(out.contains("status = 'paid'"), "{out}");
        assert!(out.contains("ORDER BY"), "{out}");
        assert!(out.contains("SELECT") && out.contains("WHERE"), "{out}");
        // Function names keep their case too; strings and comments are data.
        let out = format(
            "select count(*), now() from t -- select from",
            None,
            true,
            2,
        )
        .unwrap();
        assert!(out.contains("count(*)") && out.contains("now()"), "{out}");
        assert!(out.contains("-- select from"), "{out}");
        let out = format(
            "create table t (id int primary key, key text)",
            None,
            true,
            2,
        )
        .unwrap();
        assert!(out.contains("PRIMARY KEY"), "{out}");
        assert!(out.contains("key text"), "{out}");
        // Lowercase on request.
        let out = format("SELECT Status FROM T", None, false, 2).unwrap();
        assert!(
            out.contains("select") && out.contains("Status") && out.contains("T"),
            "{out}"
        );
    }

    #[test]
    fn the_reserved_list_is_sorted_for_binary_search() {
        let mut sorted = RESERVED.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, RESERVED);
    }

    #[test]
    fn literal_bytes_never_change() {
        // E-strings take backslash escapes: the quote after `\` does not
        // close it, and nothing inside (or in later literals) is recased.
        let sql =
            r"select E'it\'s select from', 'select lower' from t where x = E'a\\' and y = 'from'";
        let out = format(sql, Some(DriverKind::Postgres), true, 2).unwrap();
        for lit in [
            r"E'it\'s select from'",
            "'select lower'",
            r"E'a\\'",
            "'from'",
        ] {
            assert!(out.contains(lit), "{lit} changed: {out}");
        }
        assert!(out.contains("SELECT") && out.contains("WHERE"), "{out}");
        // mysql backslash escapes and double-quoted strings.
        let out = format(
            r#"select 'a\'b select', "c\"d from" from t"#,
            Some(DriverKind::Mysql),
            true,
            2,
        )
        .unwrap();
        assert!(
            out.contains(r"'a\'b select'") && out.contains(r#""c\"d from""#),
            "{out}"
        );
        // Prefixed literals stay glued to their prefix.
        let out = format(
            "select x'00ff', N'naïve select', U&'d\\0061ta'",
            None,
            true,
            2,
        )
        .unwrap();
        assert!(
            out.contains("x'00ff'")
                && out.contains("N'naïve select'")
                && out.contains("U&'d\\0061ta'"),
            "{out}"
        );
        // An unterminated literal is kept as typed.
        let out = format("select 'unterminated  select  ", None, true, 2).unwrap();
        assert!(out.ends_with("'unterminated  select  "), "{out:?}");
    }

    #[test]
    fn hostile_inputs_are_refused_before_formatting() {
        let err = format("select 1\0 from t", None, true, 2).unwrap_err();
        assert!(err.contains("NUL"), "{err}");
        let nested = format!("select {}1{}", "(".repeat(5000), ")".repeat(5000));
        let err = format(&nested, None, true, 2).unwrap_err();
        assert!(err.contains("nest"), "{err}");
        // Moderate depth on a large input: refused on the size bound.
        let term = format!("{} a + b {} + ", "(".repeat(10), ")".repeat(10));
        let wide = format!("select {} 1", term.repeat(20_000));
        let err = format(&wide, None, true, 8).unwrap_err();
        assert!(err.contains("MB"), "{err}");
        // Parentheses inside literals do not count.
        let quoted = format!("select '{}'", "(".repeat(5000));
        assert!(format(&quoted, None, true, 2).is_ok());
    }

    #[test]
    fn empty_input_is_returned_as_is() {
        assert_eq!(format("  \n", None, true, 2).unwrap(), "  \n");
    }
}
