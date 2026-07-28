//! Which table a statement mutates, read off the SQL text.
//!
//! This is a classifier, not a parser, and the distinction matters: it decides
//! whether an event fires and what `table` it carries, so being wrong in the
//! quiet direction — dropping an event — is the failure to avoid. Anything
//! recognisably DML produces an event; when the table cannot be read out with
//! confidence the event still fires with `table: null` and the subscriber that
//! filtered on a table simply does not match it.
//!
//! What it deliberately does not do: resolve CTEs, follow `INSERT … SELECT`
//! sources, or expand views. A statement it cannot pin down is reported as
//! unknown rather than guessed at.

use serde::{Deserialize, Serialize};

/// The kind of row change a statement makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Insert,
    Update,
    Delete,
    /// Recognisably a write, but not one of the three above (a CTE-wrapped
    /// statement, `MERGE`, a driver-specific form). Subscribers still hear
    /// about it.
    Other,
}

/// A statement's effect: what it does, and to what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mutation {
    pub op: Op,
    /// `None` when the statement mutates something this classifier will not
    /// guess at. A binding filtered on a table never matches these.
    pub table: Option<String>,
}

/// Classify one statement. `None` means "not a row change" — a read, DDL,
/// transaction control, or anything else that cannot change rows.
pub fn classify(sql: &str) -> Option<Mutation> {
    let stripped = strip_leading_noise(sql);
    let mut words = stripped.split_whitespace();
    let first = words.next()?.to_ascii_uppercase();

    match first.as_str() {
        "INSERT" | "REPLACE" => {
            // MySQL permits `INSERT t` / `REPLACE t`; INTO is optional.
            let rest = strip_insert_modifiers(stripped[first.len()..].trim_start());
            Some(Mutation {
                op: Op::Insert,
                table: first_identifier(rest),
            })
        }
        "UPDATE" => {
            // `UPDATE t SET …`, `UPDATE OR IGNORE t SET …`.
            let after = stripped[first.len()..].trim_start();
            let after = strip_conflict_clause(after);
            Some(Mutation {
                op: Op::Update,
                table: first_identifier(after),
            })
        }
        "DELETE" => {
            let rest = skip_until_keyword(&stripped, "FROM")?;
            Some(Mutation {
                op: Op::Delete,
                table: first_identifier(rest),
            })
        }
        // A CTE can wrap any of the above. Rather than parse the CTE list, say
        // "a write happened, table unknown" — a subscriber watching the whole
        // db still hears it, and one filtered on a table is not told a lie.
        "WITH" if mentions_dml(&stripped) => Some(Mutation {
            op: Op::Other,
            table: None,
        }),
        "MERGE" | "UPSERT" => Some(Mutation {
            op: Op::Other,
            table: skip_until_keyword(&stripped, "INTO").and_then(first_identifier),
        }),
        _ => None,
    }
}

/// Drop leading whitespace and SQL comments so the first keyword is really the
/// first keyword.
fn strip_leading_noise(sql: &str) -> String {
    let mut s = sql.trim_start();
    loop {
        if let Some(rest) = s.strip_prefix("--") {
            s = match rest.find('\n') {
                Some(i) => rest[i + 1..].trim_start(),
                None => "",
            };
        } else if let Some(rest) = s.strip_prefix("/*") {
            s = match rest.find("*/") {
                Some(i) => rest[i + 2..].trim_start(),
                None => "",
            };
        } else {
            break;
        }
    }
    s.to_string()
}

/// The text following the first standalone occurrence of `keyword`.
fn skip_until_keyword<'a>(sql: &'a str, keyword: &str) -> Option<&'a str> {
    let upper = sql.to_ascii_uppercase();
    let mut from = 0usize;
    while let Some(idx) = upper[from..].find(keyword) {
        let start = from + idx;
        let end = start + keyword.len();
        let before_ok = start == 0
            || !upper[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let after_ok = upper[end..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
        if before_ok && after_ok {
            return Some(&sql[end..]);
        }
        from = end;
    }
    None
}

/// The SELECT source when it is a single table. Ambiguous joins and
/// comma-separated sources deliberately produce no schema hint.
pub(crate) fn table_after_from(sql: &str) -> Option<String> {
    let stripped = strip_leading_noise(sql);
    let rest = skip_until_keyword(&stripped, "FROM")?;
    let end = ["WHERE", "GROUP", "ORDER", "HAVING", "LIMIT", "UNION"]
        .iter()
        .filter_map(|keyword| {
            skip_until_keyword(rest, keyword).map(|after| rest.len() - after.len() - keyword.len())
        })
        .min()
        .unwrap_or(rest.len());
    let sources = &rest[..end];
    if sources.contains(',') || skip_until_keyword(sources, "JOIN").is_some() {
        return None;
    }
    first_identifier(sources)
}

/// `OR REPLACE` / `OR IGNORE` / … between `UPDATE` and the table name.
fn strip_conflict_clause(s: &str) -> &str {
    let Some(prefix) = s.get(..2) else {
        return s;
    };
    let after_or = &s[2..];
    if !prefix.eq_ignore_ascii_case("OR")
        || !after_or.chars().next().is_some_and(char::is_whitespace)
    {
        return s;
    }
    let after_or = after_or.trim_start();
    let action_end = after_or.find(char::is_whitespace).unwrap_or(after_or.len());
    after_or[action_end..].trim_start()
}

/// MySQL modifiers and optional `INTO` between INSERT/REPLACE and the table.
/// SQLite's `OR <action>` form is stripped first by the existing helper.
fn strip_insert_modifiers(s: &str) -> &str {
    let mut rest = strip_conflict_clause(s);
    loop {
        let trimmed = rest.trim_start();
        let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
        let word = &trimmed[..end];
        if ["LOW_PRIORITY", "DELAYED", "HIGH_PRIORITY", "IGNORE"]
            .iter()
            .any(|modifier| word.eq_ignore_ascii_case(modifier))
        {
            rest = &trimmed[end..];
            continue;
        }
        if word.eq_ignore_ascii_case("INTO") {
            return &trimmed[end..];
        }
        return trimmed;
    }
}

/// The first identifier in `s`, unquoted. Stops at whitespace, `(`, or a
/// comma — enough for `t`, `"t"`, `` `t` ``, `[t]`, `schema.t`, `t(col,…)`.
fn first_identifier(s: &str) -> Option<String> {
    let s = s.trim_start();
    let mut out = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == closing(q) => quote = None,
            (Some(_), c) => out.push(c),
            (None, '"') | (None, '`') | (None, '[') => quote = Some(c),
            (None, c) if c.is_whitespace() || c == '(' || c == ',' || c == ';' => break,
            (None, c) => out.push(c),
        }
    }
    let out = out.trim().to_string();
    (!out.is_empty()).then_some(out)
}

fn closing(open: char) -> char {
    match open {
        '[' => ']',
        c => c,
    }
}

fn mentions_dml(sql: &str) -> bool {
    let upper = sql.to_ascii_uppercase();
    ["INSERT", "UPDATE", "DELETE", "MERGE"]
        .iter()
        .any(|k| skip_until_keyword(&upper, k).is_some())
}

/// Whether two table references name the same table, ignoring case and any
/// schema qualifier. `public.orders`, `"Orders"` and `orders` all match — a
/// subscriber should not have to guess how the writer spelled it.
pub fn same_table(a: &str, b: &str) -> bool {
    fn bare(t: &str) -> String {
        let t = t.rsplit('.').next().unwrap_or(t).trim();
        let t = [('"', '"'), ('`', '`'), ('[', ']')]
            .into_iter()
            .find_map(|(open, close)| t.strip_prefix(open)?.strip_suffix(close))
            .unwrap_or(t);
        t.to_ascii_lowercase()
    }
    bare(a) == bare(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(sql: &str) -> Option<Mutation> {
        classify(sql)
    }

    #[test]
    fn table_after_from_reads_the_select_source() {
        assert_eq!(
            table_after_from("SELECT a, b FROM receiving_shipments WHERE x = 1"),
            Some("receiving_shipments".into())
        );
        assert_eq!(
            table_after_from("select * from \"Orders\" join x on 1"),
            None
        );
        assert_eq!(table_after_from("PRAGMA journal_mode"), None);
        assert_eq!(
            table_after_from("SELECT missing FROM a JOIN b ON a.id = b.id"),
            None
        );
        assert_eq!(
            table_after_from("SELECT missing FROM a, b WHERE a.id = b.id"),
            None
        );
    }

    #[test]
    fn the_three_ordinary_forms() {
        assert_eq!(
            m("INSERT INTO orders (id) VALUES (1)"),
            Some(Mutation {
                op: Op::Insert,
                table: Some("orders".into())
            })
        );
        assert_eq!(
            m("UPDATE orders SET n = 1 WHERE id = 2"),
            Some(Mutation {
                op: Op::Update,
                table: Some("orders".into())
            })
        );
        assert_eq!(
            m("DELETE FROM orders WHERE id = 2"),
            Some(Mutation {
                op: Op::Delete,
                table: Some("orders".into())
            })
        );
    }

    #[test]
    fn reads_and_ddl_are_not_row_changes() {
        for sql in [
            "SELECT * FROM orders",
            "CREATE TABLE orders (id INT)",
            "DROP TABLE orders",
            "ALTER TABLE orders ADD COLUMN n INT",
            "BEGIN",
            "COMMIT",
            "PRAGMA foreign_keys = ON",
        ] {
            assert_eq!(m(sql), None, "{sql} must not fire a row change");
        }
    }

    #[test]
    fn conflict_clauses_do_not_become_the_table_name() {
        // The bug this pins: `INSERT OR REPLACE INTO t` naively reading the
        // word after INSERT would report a table called `OR`.
        assert_eq!(
            m("INSERT OR REPLACE INTO beats (n) VALUES (1)")
                .unwrap()
                .table,
            Some("beats".into())
        );
        assert_eq!(
            m("INSERT OR IGNORE INTO beats (n) VALUES (1)")
                .unwrap()
                .table,
            Some("beats".into())
        );
        assert_eq!(
            m("UPDATE OR ROLLBACK beats SET n = 1").unwrap().table,
            Some("beats".into())
        );
        assert_eq!(
            m("UPDATE OR  ROLLBACK beats SET n = 1").unwrap().table,
            Some("beats".into())
        );
        assert_eq!(
            m("INSERT OR\nREPLACE INTO beats (n) VALUES (1)")
                .unwrap()
                .table,
            Some("beats".into())
        );
        assert_eq!(
            m("REPLACE INTO beats (n) VALUES (1)").unwrap(),
            Mutation {
                op: Op::Insert,
                table: Some("beats".into())
            }
        );
    }

    #[test]
    fn mysql_insert_and_replace_allow_optional_into() {
        for sql in [
            "INSERT orders (id) VALUES (1)",
            "INSERT LOW_PRIORITY IGNORE orders (id) VALUES (1)",
            "REPLACE orders (id) VALUES (1)",
            "REPLACE DELAYED INTO orders (id) VALUES (1)",
        ] {
            assert_eq!(m(sql).unwrap().table.as_deref(), Some("orders"), "{sql}");
        }
    }

    #[test]
    fn quoted_and_qualified_identifiers_unwrap() {
        for (sql, want) in [
            (r#"INSERT INTO "Orders" (id) VALUES (1)"#, "Orders"),
            ("INSERT INTO `orders` (id) VALUES (1)", "orders"),
            ("INSERT INTO [orders] (id) VALUES (1)", "orders"),
            ("INSERT INTO public.orders (id) VALUES (1)", "public.orders"),
            ("INSERT INTO orders(id) VALUES (1)", "orders"),
            ("DELETE FROM orders;", "orders"),
        ] {
            assert_eq!(m(sql).unwrap().table.as_deref(), Some(want), "{sql}");
        }
    }

    #[test]
    fn leading_comments_do_not_hide_the_verb() {
        assert_eq!(
            m("-- seed the table\nINSERT INTO orders (id) VALUES (1)")
                .unwrap()
                .table,
            Some("orders".into())
        );
        assert_eq!(
            m("/* batch 2 */ DELETE FROM orders").unwrap().op,
            Op::Delete
        );
    }

    #[test]
    fn lowercase_and_ragged_whitespace_still_classify() {
        assert_eq!(
            m("  insert\n  into\n  orders (id) values (1)").unwrap(),
            Mutation {
                op: Op::Insert,
                table: Some("orders".into())
            }
        );
    }

    #[test]
    fn a_cte_wrapped_write_fires_with_an_unknown_table() {
        // Not parsed — but a write did happen, and dropping the event is worse
        // than reporting it without a table.
        let got = m("WITH moved AS (DELETE FROM a RETURNING *) INSERT INTO b SELECT * FROM moved")
            .unwrap();
        assert_eq!(got.op, Op::Other);
        assert_eq!(got.table, None);
        // A read-only CTE is still not a row change.
        assert_eq!(m("WITH x AS (SELECT 1) SELECT * FROM x"), None);
    }

    #[test]
    fn table_matching_ignores_case_and_schema() {
        assert!(same_table("orders", "ORDERS"));
        assert!(same_table("public.orders", "orders"));
        assert!(same_table("orders", "public.orders"));
        assert!(same_table("\"Orders\"", "orders"));
        assert!(same_table("`orders`", "orders"));
        assert!(same_table("[orders]", "orders"));
        assert!(!same_table("orders", "order_items"));
    }

    #[test]
    fn a_statement_naming_no_table_is_reported_not_dropped() {
        // `INSERT INTO` with nothing after it is malformed SQL the driver will
        // reject, but the classifier must not panic or invent a name.
        assert_eq!(m("INSERT INTO").unwrap().table, None);
        assert_eq!(m("DELETE FROM  ").unwrap().table, None);
    }
}
