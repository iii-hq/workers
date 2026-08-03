//! First-line defense against side-channel transaction finalization.
//!
//! `transactionExecute` and `transactionQuery` accept caller-supplied SQL and
//! run it on a connection that the worker holds inside `BEGIN ... COMMIT`.
//! If a caller submits raw `COMMIT` / `ROLLBACK` / `BEGIN` / `SAVEPOINT` /
//! `SET TRANSACTION` / `START TRANSACTION`, the driver executes it server-
//! side — finalizing or restarting the transaction without the `TxRegistry`
//! ever finding out. The next handler call then operates on a pinned conn
//! whose txn state no longer matches the registry's view, silently breaking
//! the atomicity contract.
//!
//! [`is_transaction_control_sql`] returns `true` for any SQL whose first
//! non-comment token is a transaction-control keyword, letting handlers
//! reject with `INVALID_PARAM` before the registry lookup.
//!
//! Coverage:
//!   - Plain keyword forms: `BEGIN`, `COMMIT`, `ROLLBACK`, `END`,
//!     `SAVEPOINT`, `RELEASE`, two-token forms `SET TRANSACTION ...` and
//!     `START TRANSACTION ...` (ANSI / MySQL spelling of `BEGIN`).
//!   - Comment-prefixed forms — `/* */COMMIT`, `-- foo\nCOMMIT`,
//!     `/* a */ /* b */COMMIT`, and any chain of leading whitespace, `;`,
//!     line comments, and block comments (including nested block comments
//!     for postgres parity).
//!
//! Why nested block comments: postgres parses `/* /* nest */ */` as a single
//! comment. mysql and sqlite don't, but if we over-strip on those drivers
//! the worst case is a false-positive rejection — far better than letting a
//! finalization SQL through on postgres.

/// Returns `true` if the first non-comment, non-whitespace, non-`;` token
/// of `sql` is a transaction-control keyword. The two-token forms
/// `SET TRANSACTION ...` and `START TRANSACTION ...` are matched only when
/// the second token is exactly `TRANSACTION` (case-insensitive) — bare
/// `SET LOCAL ...` (legitimate per-tx session settings) and MySQL
/// replication's `START SLAVE` / `START REPLICA` are left alone.
pub(super) fn is_transaction_control_sql(sql: &str) -> bool {
    let trimmed = strip_leading_noise(sql);
    let mut tokens = trimmed.split_whitespace();
    let Some(first) = tokens.next() else {
        return false;
    };
    // `COMMIT;` / `end;` reach here including the trailing `;`. Strip it so
    // terminated and unterminated forms share one branch.
    let first = first.trim_end_matches(';');
    let upper = first.to_ascii_uppercase();
    match upper.as_str() {
        "BEGIN" | "COMMIT" | "ROLLBACK" | "END" | "SAVEPOINT" | "RELEASE" => true,
        "SET" | "START" => tokens
            .next()
            .map(|t| t.trim_end_matches(';').eq_ignore_ascii_case("TRANSACTION"))
            .unwrap_or(false),
        _ => false,
    }
}

/// Statements that only read. Anything not on this list is treated as a
/// write, so an unrecognised or vendor-specific verb fails closed.
const READ_ONLY_VERBS: &[&str] = &[
    "select", "with", "explain", "pragma", "show", "describe", "desc", "values", "table",
];

/// Keywords that mutate. Checked anywhere in the statement, not just at the
/// front, so a data-modifying CTE (`WITH x AS (DELETE ...) SELECT ...`) is
/// caught — it leads with `WITH` but is very much a write.
const WRITE_KEYWORDS: &[&str] = &[
    "insert", "update", "delete", "replace", "merge", "upsert", "drop", "create", "alter",
    "truncate", "grant", "revoke", "attach", "detach", "reindex", "vacuum", "call", "do",
];

/// Whether `sql` is a single statement that cannot modify data.
///
/// Used to gate `EXPLAIN ANALYZE`, which really executes the statement — an
/// `EXPLAIN ANALYZE DELETE FROM users` deletes users. The client-side check
/// in the console is a convenience; this one is the authority.
///
/// Fails closed: anything it cannot confidently classify as a read is a
/// write. Reuses `strip_leading_noise`, so a leading `--` or `/* */` comment
/// cannot smuggle a verb past the check.
pub(crate) fn is_read_only_sql(sql: &str) -> bool {
    let head = strip_leading_noise(sql);
    if head.is_empty() {
        return false;
    }

    // More than one statement means the tail is unchecked; refuse rather than
    // classify only the first. A trailing `;` alone is fine.
    if head.trim_end().trim_end_matches(';').contains(';') {
        return false;
    }

    let lowered = head.to_ascii_lowercase();
    let leading = lowered
        .split(|c: char| !c.is_ascii_alphabetic())
        .find(|w| !w.is_empty())
        .unwrap_or_default();
    if !READ_ONLY_VERBS.contains(&leading) {
        return false;
    }

    // `PRAGMA foo = 1` writes; `PRAGMA foo` reads.
    if leading == "pragma" && lowered.contains('=') {
        return false;
    }

    // Word-boundary scan so `created_at` does not trip the `create` keyword,
    // and so `EXPLAIN ANALYZE DELETE ...` is caught by its `delete`.
    !lowered
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .any(|w| WRITE_KEYWORDS.contains(&w))
}

/// Strip any prefix of leading whitespace, `;`, line comments (`-- ...\n`),
/// and block comments (`/* ... */`, with nesting supported) so the returned
/// slice begins at the first character of the first real token. Returns an
/// empty slice when the input is comment-only or contains an unterminated
/// comment — the caller treats both as "no command present".
fn strip_leading_noise(sql: &str) -> &str {
    let mut s = sql;
    loop {
        let trimmed = s.trim_start_matches(|c: char| c.is_whitespace() || c == ';');
        if let Some(after) = trimmed.strip_prefix("--") {
            // Line comment ends at the next newline; if no newline exists,
            // the comment swallows the rest of the input.
            match after.find('\n') {
                Some(pos) => s = &after[pos + 1..],
                None => return "",
            }
        } else if let Some(after) = trimmed.strip_prefix("/*") {
            // Block comment with postgres-style nesting. If the comment is
            // unterminated, consume the rest — the driver would error on it
            // regardless, and we'd rather over-strip than risk a bypass.
            match strip_block_comment_body(after) {
                Some(rest) => s = rest,
                None => return "",
            }
        } else {
            return trimmed;
        }
    }
}

/// Consume the body of a block comment whose opening `/*` was already
/// stripped, returning the remainder of the input after the matching `*/`.
/// Handles nested `/* /* */ */`. Returns `None` when unterminated.
///
/// Byte-level scanning is sound here because `/` (0x2F) and `*` (0x2A) are
/// both ASCII and never appear as continuation bytes in valid UTF-8
/// (continuation bytes are 0x80..0xBF). A returned slice always begins
/// immediately after the matched `*/` — i.e. on a char boundary — because
/// `/` is a 1-byte ASCII char.
fn strip_block_comment_body(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let mut depth: usize = 1;
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return Some(&s[i..]);
            }
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Existing keyword coverage (no regression) -------------------------

    #[test]
    fn flags_plain_finalization_keywords() {
        assert!(is_transaction_control_sql("COMMIT"));
        assert!(is_transaction_control_sql(" rollback "));
        assert!(is_transaction_control_sql("ROLLBACK TO SAVEPOINT foo"));
        assert!(is_transaction_control_sql(
            "BEGIN ISOLATION LEVEL SERIALIZABLE"
        ));
        assert!(is_transaction_control_sql("end;"));
        assert!(is_transaction_control_sql("SAVEPOINT s1"));
        assert!(is_transaction_control_sql("RELEASE SAVEPOINT s1"));
        assert!(is_transaction_control_sql("SET TRANSACTION READ ONLY"));
        assert!(is_transaction_control_sql(
            "set transaction isolation level serializable"
        ));
    }

    #[test]
    fn passes_normal_dml() {
        assert!(!is_transaction_control_sql("INSERT INTO t (n) VALUES (1)"));
        assert!(!is_transaction_control_sql("UPDATE t SET n = 2"));
        assert!(!is_transaction_control_sql("DELETE FROM t"));
        assert!(!is_transaction_control_sql("SELECT 1"));
        // Bare `SET` for per-tx session settings — second token isn't TRANSACTION.
        assert!(!is_transaction_control_sql("SET LOCAL search_path = foo"));
        assert!(!is_transaction_control_sql("SET search_path TO foo"));
    }

    // --- Finding #3: START TRANSACTION (MySQL/ANSI spelling) ---------------

    #[test]
    fn flags_start_transaction() {
        assert!(is_transaction_control_sql("START TRANSACTION"));
        assert!(is_transaction_control_sql("start transaction"));
        assert!(is_transaction_control_sql(
            "START TRANSACTION ISOLATION LEVEL SERIALIZABLE"
        ));
        assert!(is_transaction_control_sql("start transaction;"));
        assert!(is_transaction_control_sql("START\tTRANSACTION"));
    }

    #[test]
    fn passes_start_other_than_transaction() {
        // MySQL replication ops use START with a different second token —
        // not finalization, must pass through.
        assert!(!is_transaction_control_sql("START SLAVE"));
        assert!(!is_transaction_control_sql("START REPLICA SQL_THREAD"));
        // Bare `START` with nothing after isn't a real statement, but it
        // also isn't transaction-control.
        assert!(!is_transaction_control_sql("START"));
    }

    // --- Finding #1: comment-prefixed bypass --------------------------------

    #[test]
    fn flags_block_comment_prefixed_finalization() {
        assert!(is_transaction_control_sql("/* sneak */COMMIT"));
        assert!(is_transaction_control_sql("/* sneak */ COMMIT"));
        assert!(is_transaction_control_sql("/**/ COMMIT"));
        assert!(is_transaction_control_sql("/*\n multi\n line\n*/ ROLLBACK"));
        // Chained block comments.
        assert!(is_transaction_control_sql("/* a *//* b */ COMMIT"));
        assert!(is_transaction_control_sql("/* a */ /* b */ /* c */ COMMIT"));
        // Mixed with whitespace and `;`.
        assert!(is_transaction_control_sql(
            ";  /* foo */ ;; /* bar */COMMIT"
        ));
        // Postgres-style nested block comments must strip completely.
        assert!(is_transaction_control_sql(
            "/* outer /* inner */ outer */ COMMIT"
        ));
    }

    #[test]
    fn flags_line_comment_prefixed_finalization() {
        assert!(is_transaction_control_sql("-- sneak\nCOMMIT"));
        assert!(is_transaction_control_sql("-- foo\n  -- bar\nROLLBACK"));
        // Mixed line + block comments.
        assert!(is_transaction_control_sql("-- foo\n/* bar */ COMMIT"));
        assert!(is_transaction_control_sql("/* bar */-- foo\nCOMMIT"));
    }

    #[test]
    fn flags_comment_prefixed_start_transaction() {
        // Combine finding #1 and finding #3: a comment-prefixed
        // START TRANSACTION must also be rejected (otherwise MySQL would
        // implicitly commit the outer txn).
        assert!(is_transaction_control_sql("/* */ START TRANSACTION"));
        assert!(is_transaction_control_sql("-- skip\nSTART TRANSACTION"));
    }

    #[test]
    fn passes_comment_only_or_unterminated() {
        // A SQL string that's *only* comments has no command — driver will
        // no-op or error; we don't need to reject.
        assert!(!is_transaction_control_sql("/* just a comment */"));
        assert!(!is_transaction_control_sql("-- end-of-line comment\n"));
        assert!(!is_transaction_control_sql("-- no newline"));
        // Unterminated block comment: strip consumes the rest, no token left.
        assert!(!is_transaction_control_sql("/* unterminated COMMIT"));
        assert!(!is_transaction_control_sql(""));
        assert!(!is_transaction_control_sql("   "));
        assert!(!is_transaction_control_sql(";;;;"));
    }

    #[test]
    fn passes_comment_prefixed_dml() {
        // Stripping must not turn benign comment-prefixed DML into a flag.
        assert!(!is_transaction_control_sql("/* note */ SELECT 1"));
        assert!(!is_transaction_control_sql(
            "-- note\nINSERT INTO t VALUES (1)"
        ));
        assert!(!is_transaction_control_sql(
            "/* a */ /* b */ UPDATE t SET n=1"
        ));
    }

    // --- UTF-8 safety -------------------------------------------------------

    #[test]
    fn handles_utf8_inside_comments() {
        // Multi-byte characters inside the comment body must not break
        // byte-level scanning. The string after `/* … */` is just `COMMIT`.
        assert!(is_transaction_control_sql("/* 漢字 🔥 مرحبا */ COMMIT"));
        assert!(is_transaction_control_sql("-- 漢字 emoji 🚀\nROLLBACK"));
    }

    #[test]
    fn handles_utf8_in_statement_body() {
        // UTF-8 chars *after* the keyword must also survive (regression
        // safety net for the byte-scan implementation).
        assert!(!is_transaction_control_sql("SELECT '漢字 🔥' AS n"));
        assert!(!is_transaction_control_sql(
            "INSERT INTO t VALUES ('مرحبا')"
        ));
    }

    #[test]
    fn read_only_accepts_plain_reads() {
        for sql in [
            "SELECT 1",
            "  select * from users where created_at > now()",
            "WITH x AS (SELECT 1) SELECT * FROM x",
            "EXPLAIN SELECT * FROM t",
            "PRAGMA table_info(users)",
            "SHOW TABLES",
            "VALUES (1), (2)",
            "-- a comment\nSELECT 1",
            "/* block */ SELECT 1",
            "SELECT 1;",
        ] {
            assert!(is_read_only_sql(sql), "should be read-only: {sql}");
        }
    }

    #[test]
    fn read_only_rejects_writes_and_the_ways_they_hide() {
        for sql in [
            // Plain writes.
            "DELETE FROM users",
            "UPDATE users SET a = 1",
            "DROP TABLE users",
            // The one that matters most: EXPLAIN ANALYZE really executes.
            "EXPLAIN ANALYZE DELETE FROM users",
            // A data-modifying CTE leads with WITH but is a write.
            "WITH x AS (DELETE FROM users RETURNING *) SELECT * FROM x",
            // A second statement would go unchecked.
            "SELECT 1; DROP TABLE users",
            // A comment must not smuggle the verb past the check.
            "-- SELECT\nDELETE FROM users",
            "/* SELECT */ DROP TABLE t",
            // PRAGMA with an assignment writes.
            "PRAGMA journal_mode = WAL",
            // Unknown verbs fail closed.
            "LOCK TABLE users",
            "",
            "   ",
        ] {
            assert!(!is_read_only_sql(sql), "should be refused: {sql}");
        }
    }

    #[test]
    fn read_only_does_not_trip_on_identifiers_containing_keywords() {
        // `created_at` contains "create"; a substring scan would reject this.
        assert!(is_read_only_sql(
            "SELECT created_at, updated_at FROM t ORDER BY created_at"
        ));
    }
}
