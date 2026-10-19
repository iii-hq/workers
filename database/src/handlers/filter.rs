//! Typed filters and sorts, compiled to dialect SQL here rather than in the
//! caller.
//!
//! A caller says "email contains test and plan equals free" as data; this
//! module turns that into a parameterised `WHERE` clause for the driver in
//! hand. That matters for correctness as much as convenience — the traps
//! below are ones every hand-written client filter gets wrong at least once:
//!
//! * **LIKE metacharacters.** `contains "50%"` must match a literal percent,
//!   not every row. Values are escaped and the clause carries `ESCAPE '\'`
//!   (`'\\'` on mysql, where a backslash escapes inside literals too).
//! * **`IS NULL` is not `= ''`.** They are separate operators, because
//!   conflating them silently changes the answer.
//! * **Case-insensitivity is not portable.** Postgres expresses it as
//!   `ILIKE`; sqlite and mysql apply it through collation and cannot honour a
//!   per-query flag. Asking for it there is rejected rather than ignored.
//! * **Placeholders differ.** Postgres numbers them, the others do not.

use crate::config::DriverKind;
use crate::error::DbError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Escape character for `LIKE` patterns. Backslash is the conventional choice
/// and is stated explicitly in every clause so no driver default applies.
const LIKE_ESCAPE: char = '\\';

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Contains,
    NotContains,
    Equals,
    NotEquals,
    StartsWith,
    EndsWith,
    Gt,
    Gte,
    Lt,
    Lte,
    /// Inclusive range; needs both `value` and `value2`.
    Between,
    IsTrue,
    IsFalse,
    IsNull,
    IsNotNull,
    /// NULL or the empty string. Distinct from `is_null` on purpose.
    IsEmpty,
    /// Set membership, over `values`.
    // Filters stack with AND, so "status is one of open, pending, held" cannot
    // be expressed as OR'd equality filters; without this it cannot be asked.
    In,
    NotIn,
}

impl FilterOp {
    /// How many operands the operator consumes. Used to reject an incomplete
    /// filter up front instead of compiling something that means nothing.
    fn arity(self) -> usize {
        match self {
            FilterOp::IsTrue
            | FilterOp::IsFalse
            | FilterOp::IsNull
            | FilterOp::IsNotNull
            | FilterOp::IsEmpty => 0,
            FilterOp::Between => 2,
            // Variadic: operands come from `values`, not `value`.
            FilterOp::In | FilterOp::NotIn => 0,
            _ => 1,
        }
    }

    fn is_like(self) -> bool {
        matches!(
            self,
            FilterOp::Contains | FilterOp::NotContains | FilterOp::StartsWith | FilterOp::EndsWith
        )
    }

    fn is_set(self) -> bool {
        matches!(self, FilterOp::In | FilterOp::NotIn)
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct FilterSpec {
    /// The column (ignored with `raw`).
    #[serde(default)]
    pub column: String,
    /// The operator (ignored with `raw`; required otherwise).
    #[serde(default)]
    pub op: Option<FilterOp>,
    /// A SQL boolean expression in the driver's dialect (`price > 10 AND
    /// name ILIKE 'a%'`), ANDed with the other filters inside its own
    /// parentheses. One expression only: no `;`, no comments, no unbalanced
    /// parentheses, no placeholders — and it runs in a read-only transaction
    /// (sqlite: `query_only`), so it can never write.
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub value: Option<Value>,
    /// Upper bound for `between`.
    #[serde(default)]
    pub value2: Option<Value>,
    /// Operands for `in` / `not_in`.
    #[serde(default)]
    pub values: Vec<Value>,
    /// Postgres only. Rejected elsewhere rather than silently ignored.
    #[serde(default)]
    pub case_sensitive: Option<bool>,
    /// Kept in the list but not applied.
    // Lets a caller switch one condition off and back on without losing how
    // it was built; a console that only offers delete makes that a retype.
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NullsPosition {
    First,
    Last,
}

/// Type-aware sort modes, applied server-side over the whole table.
// They exist here because the grid is paged: sorting the fetched page would
// order 50 rows out of N, a different answer from sorting the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SortMode {
    #[default]
    Default,
    /// `item2` before `item10`. The mode users actually notice.
    Natural,
    Length,
    AbsoluteValue,
    Random,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SortSpec {
    pub column: String,
    #[serde(default = "default_direction")]
    pub direction: Direction,
    #[serde(default)]
    pub nulls: Option<NullsPosition>,
    #[serde(default)]
    pub mode: SortMode,
}

fn default_direction() -> Direction {
    Direction::Asc
}

/// Quote an identifier for the driver, doubling the quote character so a
/// crafted column name cannot escape it.
pub fn quote_ident(driver: DriverKind, ident: &str) -> String {
    match driver {
        DriverKind::Mysql => format!("`{}`", ident.replace('`', "``")),
        _ => format!("\"{}\"", ident.replace('"', "\"\"")),
    }
}

/// Qualify a table reference. Only postgres has a schema above the table.
pub fn quote_table(driver: DriverKind, schema: Option<&str>, table: &str) -> String {
    match (driver, schema) {
        (DriverKind::Postgres, Some(s)) => {
            format!("{}.{}", quote_ident(driver, s), quote_ident(driver, table))
        }
        _ => quote_ident(driver, table),
    }
}

/// Emits `$1`, `$2`, … on postgres and `?` elsewhere.
struct Placeholders {
    driver: DriverKind,
    next: usize,
}

impl Placeholders {
    fn new(driver: DriverKind, start: usize) -> Self {
        Self {
            driver,
            next: start,
        }
    }

    fn take(&mut self) -> String {
        let n = self.next;
        self.next += 1;
        match self.driver {
            DriverKind::Postgres => format!("${n}"),
            _ => "?".to_string(),
        }
    }
}

fn invalid(reason: impl Into<String>) -> DbError {
    DbError::InvalidParam {
        index: 0,
        reason: reason.into(),
    }
}

/// Escape `%`, `_` and the escape character itself so they match literally.
fn escape_like(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch == '%' || ch == '_' || ch == LIKE_ESCAPE {
            out.push(LIKE_ESCAPE);
        }
        out.push(ch);
    }
    out
}

fn as_text(value: &Value, column: &str) -> Result<String, DbError> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => Err(invalid(format!(
            "filter on `{column}` needs a text-comparable value"
        ))),
    }
}

#[derive(Debug, Clone)]
pub struct WhereClause {
    /// Empty when there are no filters; otherwise a bare boolean expression
    /// with no leading `WHERE`, so callers can compose it.
    pub sql: String,
    pub params: Vec<Value>,
}

/// Compile filters into a parameterised predicate. `start_param` is the first
/// free placeholder index, so a caller that already bound values keeps
/// numbering correct on postgres.
pub fn compile_where(
    driver: DriverKind,
    filters: &[FilterSpec],
    start_param: usize,
) -> Result<WhereClause, DbError> {
    compile_where_typed(driver, filters, start_param, None)
}

/// A postgres type spelling from the catalog that can follow `::` (letters,
/// digits, spaces, `_ . " [ ] ( ) ,`).
pub(crate) fn castable_pg_type(ty: &str) -> bool {
    !ty.is_empty()
        && ty.len() <= 200
        && ty
            .chars()
            .all(|c| c.is_alphanumeric() || " _.\"[](),".contains(c))
        && !ty.contains("--")
}

fn is_pg_text_type(ty: &str) -> bool {
    let t = ty.to_ascii_lowercase();
    t == "text"
        || t == "name"
        || t == "citext"
        || t.starts_with("character")
        || t.starts_with("varchar")
        || t.starts_with("char")
}

/// The text form a value is bound as when cast server-side.
fn as_cast_text(value: &Value) -> Value {
    match value {
        Value::Null | Value::String(_) => value.clone(),
        Value::Number(n) => Value::String(n.to_string()),
        Value::Bool(b) => Value::String(b.to_string()),
        other => Value::String(other.to_string()),
    }
}

/// [`compile_where`] with the columns' catalog types (postgres). Each value
/// is then bound as text and cast to its column's type (`$n::text::<type>`)
/// — postgres parses it with the type's own input function, exactly — and a
/// pattern operator on a non-text column compares the column's text form.
/// Without types, a JSON number bound to a NUMERIC (or a string to a date,
/// uuid or inet) parameter went over the wire in the wrong binary format:
/// errors at best, a garbage bound that matched every row at worst.
pub fn compile_where_typed(
    driver: DriverKind,
    filters: &[FilterSpec],
    start_param: usize,
    column_types: Option<&std::collections::HashMap<String, String>>,
) -> Result<WhereClause, DbError> {
    let mut ph = Placeholders::new(driver, start_param);
    let mut parts: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();

    for f in filters {
        // A disabled filter is skipped, not rejected: it stays in the caller's
        // list so it can be switched back on without being rebuilt.
        if f.disabled {
            continue;
        }
        if let Some(raw) = &f.raw {
            check_raw_filter(driver, raw)?;
            parts.push(format!("({})", raw.trim()));
            continue;
        }
        let Some(op) = f.op else {
            return Err(invalid(format!(
                "filter on `{}` needs `op` (or `raw` for a SQL expression)",
                f.column
            )));
        };
        if f.column.trim().is_empty() {
            return Err(invalid("filter is missing a column"));
        }
        // With the catalog at hand, a column the table does not have is the
        // caller's mistake, said plainly — not a database error later.
        if let Some(types) = column_types {
            if !types.contains_key(&f.column) {
                return Err(invalid(format!("the table has no column `{}`", f.column)));
            }
        }
        // Check operands by position, not by count. Counting accepts
        // `{op: equals, value2: 5}` — one operand supplied for a one-operand
        // operator — and the compiler then unwraps the `value` that is not
        // there. The request body is caller-controlled, so that is a panic
        // reachable from the wire.
        let arity = op.arity();
        if arity >= 1 && f.value.is_none() {
            return Err(invalid(format!("filter on `{}` needs `value`", f.column)));
        }
        if arity >= 2 && f.value2.is_none() {
            return Err(invalid(format!(
                "filter on `{}` needs both `value` and `value2`",
                f.column
            )));
        }
        if op.is_set() && f.values.is_empty() {
            // `IN ()` is a syntax error on every driver, and silently
            // dropping the filter would quietly widen the result set.
            return Err(invalid(format!(
                "filter on `{}` needs at least one value in `values`",
                f.column
            )));
        }

        // Case sensitivity is only expressible on postgres. Everywhere else
        // it is a property of the column's collation, so honouring the flag
        // would be a lie and ignoring it would be worse.
        // Rejected whenever it cannot be honoured, which includes operators
        // that never consult it. Accepting `{op: equals, case_sensitive: true}`
        // and quietly doing nothing is the failure the module header warns
        // about, just in a different place.
        if f.case_sensitive.is_some() && !(driver == DriverKind::Postgres && op.is_like()) {
            return Err(invalid(
                "case_sensitive applies only to postgres pattern operators; \
                 elsewhere it is determined by the column collation",
            ));
        }

        let col = quote_ident(driver, &f.column);
        let sensitive = f.case_sensitive.unwrap_or(false);
        // The column's catalog type, when there is one to cast to.
        let cast_to = match (driver, column_types) {
            (DriverKind::Postgres, Some(types)) => types
                .get(&f.column)
                .map(String::as_str)
                .filter(|ty| castable_pg_type(ty)),
            _ => None,
        };
        let text_col = match cast_to {
            Some(ty) if !is_pg_text_type(ty) => format!("CAST({col} AS TEXT)"),
            _ => col.clone(),
        };
        let bind = |params: &mut Vec<Value>, ph: &mut Placeholders, v: &Value| match cast_to {
            Some(ty) => {
                params.push(as_cast_text(v));
                format!("{}::text::{ty}", ph.take())
            }
            None => {
                params.push(v.clone());
                ph.take()
            }
        };

        let part = match op {
            FilterOp::IsNull => format!("{col} IS NULL"),
            FilterOp::IsNotNull => format!("{col} IS NOT NULL"),
            FilterOp::IsTrue => format!("{col} = TRUE"),
            FilterOp::IsFalse => format!("{col} = FALSE"),
            // NULL *or* empty string — the distinction from is_null is the point.
            FilterOp::IsEmpty => format!("({col} IS NULL OR {text_col} = '')"),

            FilterOp::In | FilterOp::NotIn => {
                let marks: Vec<String> = f
                    .values
                    .iter()
                    .map(|v| bind(&mut params, &mut ph, v))
                    .collect();
                let list = marks.join(", ");
                if op == FilterOp::In {
                    format!("{col} IN ({list})")
                } else {
                    // NOT IN drops NULLs on every driver, because `NULL <> x`
                    // is unknown. A reader asking for "not one of these" means
                    // to keep the NULLs, so say so explicitly.
                    format!("({col} IS NULL OR {col} NOT IN ({list}))")
                }
            }

            FilterOp::Contains
            | FilterOp::NotContains
            | FilterOp::StartsWith
            | FilterOp::EndsWith => {
                let raw = as_text(f.value.as_ref().expect("arity checked"), &f.column)?;
                let escaped = escape_like(&raw);
                let pattern = match op {
                    FilterOp::StartsWith => format!("{escaped}%"),
                    FilterOp::EndsWith => format!("%{escaped}"),
                    _ => format!("%{escaped}%"),
                };
                let negate = op == FilterOp::NotContains;
                let op = match (driver, sensitive) {
                    (DriverKind::Postgres, false) => "ILIKE",
                    _ => "LIKE",
                };
                params.push(Value::String(pattern));
                let p = ph.take();
                // mysql reads a backslash in a string literal as an escape
                // itself: `'\'` there is an unterminated literal.
                let escape = match driver {
                    DriverKind::Mysql => "'\\\\'",
                    _ => "'\\'",
                };
                let expr = format!("{text_col} {op} {p} ESCAPE {escape}");
                if negate {
                    // A NULL never matches LIKE, so a naive NOT LIKE drops
                    // NULL rows the user would expect to see in "does not
                    // contain".
                    format!("({col} IS NULL OR NOT ({expr}))")
                } else {
                    expr
                }
            }

            FilterOp::Between => {
                let lo = bind(
                    &mut params,
                    &mut ph,
                    f.value.as_ref().expect("arity checked"),
                );
                let hi = bind(
                    &mut params,
                    &mut ph,
                    f.value2.as_ref().expect("arity checked"),
                );
                format!("{col} BETWEEN {lo} AND {hi}")
            }

            _ => {
                let sym = match op {
                    FilterOp::Equals => "=",
                    FilterOp::NotEquals => "<>",
                    FilterOp::Gt => ">",
                    FilterOp::Gte => ">=",
                    FilterOp::Lt => "<",
                    FilterOp::Lte => "<=",
                    other => unreachable!("{other:?} handled above"),
                };
                let p = bind(
                    &mut params,
                    &mut ph,
                    f.value.as_ref().expect("arity checked"),
                );
                format!("{col} {sym} {p}")
            }
        };
        parts.push(part);
    }

    Ok(WhereClause {
        sql: parts.join(" AND "),
        params,
    })
}

/// Longest raw filter accepted.
pub const MAX_RAW_FILTER_LEN: usize = 4_096;

/// Check that a raw filter is ONE boolean expression that stays inside the
/// parentheses it is wrapped in: no statement separator, no comment (which
/// could swallow the closing parenthesis, or on mysql `/*! … */` run code),
/// no parenthesis closing below its own level, no placeholder. Quoting is
/// lexed the way the driver lexes it — including postgres dollar quotes,
/// mysql backticks and sqlite brackets — so nothing hides inside a span the
/// server reads differently; a backslash inside a quoted span is refused
/// where its meaning depends on server settings.
///
/// This is the outer wall. The inner one is the server: the filter runs in a
/// read-only transaction (postgres, mysql) or under `query_only` (sqlite),
/// through the extended / prepared protocol that takes a single statement.
pub fn check_raw_filter(driver: DriverKind, raw: &str) -> Result<(), DbError> {
    let refuse = |why: &str| invalid(format!("raw filter refused: {why}"));
    let text = raw.trim();
    if text.is_empty() {
        return Err(refuse("it is empty"));
    }
    if text.len() > MAX_RAW_FILTER_LEN {
        return Err(refuse(&format!(
            "longer than {MAX_RAW_FILTER_LEN} characters"
        )));
    }
    if text.contains('\0') {
        return Err(refuse("it contains a NUL byte"));
    }
    let b = text.as_bytes();
    let mut i = 0usize;
    let mut depth = 0usize;
    // Inside a quoted span closed by `close`, doubling it escapes it.
    let quoted = |i: &mut usize, close: u8, backslash: Backslash| -> Result<(), DbError> {
        let open_at = *i;
        *i += 1;
        while *i < b.len() {
            let c = b[*i];
            if c == b'\\' {
                match backslash {
                    Backslash::Escape => {
                        *i += 2;
                        continue;
                    }
                    Backslash::Refuse => {
                        return Err(refuse(
                            "a backslash inside a quoted value is ambiguous here (its meaning \
                             depends on server settings); use another character",
                        ))
                    }
                    Backslash::Literal => {}
                }
            }
            if c == close {
                if *i + 1 < b.len() && b[*i + 1] == close {
                    *i += 2;
                    continue;
                }
                *i += 1;
                return Ok(());
            }
            *i += 1;
        }
        let _ = open_at;
        Err(refuse("a quoted value or identifier is not closed"))
    };
    while i < b.len() {
        let c = b[i];
        match c {
            b'\'' => {
                // Postgres E'…' strings take backslash escapes; plain ones
                // depend on standard_conforming_strings. MySQL strings take
                // them unless NO_BACKSLASH_ESCAPES. SQLite never does.
                let e_string = driver == DriverKind::Postgres
                    && i > 0
                    && matches!(b[i - 1], b'e' | b'E')
                    && (i < 2 || !(b[i - 2].is_ascii_alphanumeric() || b[i - 2] == b'_'));
                let backslash = match driver {
                    DriverKind::Sqlite => Backslash::Literal,
                    DriverKind::Postgres if e_string => Backslash::Escape,
                    _ => Backslash::Refuse,
                };
                quoted(&mut i, b'\'', backslash)?;
            }
            b'"' => {
                // An identifier on postgres/sqlite (no escapes); a string on
                // mysql unless ANSI_QUOTES — refuse backslashes there.
                let backslash = if driver == DriverKind::Mysql {
                    Backslash::Refuse
                } else {
                    Backslash::Literal
                };
                quoted(&mut i, b'"', backslash)?;
            }
            b'`' if driver != DriverKind::Postgres => quoted(&mut i, b'`', Backslash::Literal)?,
            b'[' if driver == DriverKind::Sqlite => quoted(&mut i, b']', Backslash::Literal)?,
            // Inside an identifier (`a$b`) a `$` is just a character, as the
            // postgres lexer reads it — never the start of a dollar quote.
            b'$' if driver == DriverKind::Postgres
                && i > 0
                && (b[i - 1].is_ascii_alphanumeric()
                    || b[i - 1] == b'_'
                    || b[i - 1] == b'$'
                    || b[i - 1] >= 0x80) =>
            {
                i += 1;
            }
            b'$' if driver == DriverKind::Postgres => {
                // `$1` is a placeholder; `$tag$ … $tag$` a dollar quote.
                if b.get(i + 1).is_some_and(|d| d.is_ascii_digit()) {
                    return Err(refuse("placeholders ($1) are not allowed; write the value"));
                }
                let tag_end = b[i + 1..]
                    .iter()
                    .position(|&d| !(d.is_ascii_alphanumeric() || d == b'_'))
                    .map(|p| i + 1 + p);
                match tag_end {
                    Some(end) if b[end] == b'$' => {
                        let tag = &text[i..=end];
                        let body_start = end + 1;
                        match text[body_start..].find(tag) {
                            Some(close) => i = body_start + close + tag.len(),
                            None => return Err(refuse("a dollar-quoted value is not closed")),
                        }
                    }
                    _ => i += 1,
                }
            }
            b';' => return Err(refuse("`;` would start another statement")),
            b'-' if b.get(i + 1) == Some(&b'-') => {
                return Err(refuse("comments (`--`) are not allowed"))
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                return Err(refuse("comments (`/* */`) are not allowed"))
            }
            b'#' if driver == DriverKind::Mysql => {
                return Err(refuse("comments (`#`) are not allowed"))
            }
            b'?' if driver != DriverKind::Postgres => {
                return Err(refuse("placeholders (?) are not allowed; write the value"))
            }
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                if depth == 0 {
                    return Err(refuse("a `)` closes a parenthesis the filter did not open"));
                }
                depth -= 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    if depth != 0 {
        return Err(refuse("a `(` is never closed"));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Backslash {
    /// Escapes the next character.
    Escape,
    /// An ordinary character.
    Literal,
    /// Meaning depends on server settings: refuse.
    Refuse,
}

/// Compile sorts into an `ORDER BY` body (no leading keyword). Column names
/// are quoted; modes that a driver cannot express fall back to plain ordering
/// rather than erroring, because a sort is a presentation choice and refusing
/// one would be worse than approximating it.
pub fn compile_order_by(driver: DriverKind, sorts: &[SortSpec]) -> Result<String, DbError> {
    let mut parts = Vec::new();
    for s in sorts {
        if s.column.trim().is_empty() {
            return Err(invalid("sort is missing a column"));
        }
        let col = quote_ident(driver, &s.column);
        let expr = match s.mode {
            SortMode::Default => col.clone(),
            SortMode::Length => match driver {
                DriverKind::Mysql => format!("CHAR_LENGTH({col})"),
                _ => format!("LENGTH({col})"),
            },
            SortMode::AbsoluteValue => format!("ABS({col})"),
            SortMode::Random => match driver {
                DriverKind::Postgres => "RANDOM()".to_string(),
                DriverKind::Mysql => "RAND()".to_string(),
                DriverKind::Sqlite => "RANDOM()".to_string(),
            },
            // Natural order: pad the leading digit run so `item2` sorts before
            // `item10`. Postgres can express this inline; the others have no
            // portable equivalent, so they order plainly rather than pretend.
            SortMode::Natural => match driver {
                DriverKind::Postgres => format!(
                    "regexp_replace({col}, '\\d+', lpad(substring({col} from '\\d+'), 12, '0'))"
                ),
                _ => col.clone(),
            },
        };

        let dir = match s.direction {
            Direction::Asc => "ASC",
            Direction::Desc => "DESC",
        };
        let mut term = format!("{expr} {dir}");
        if let Some(nulls) = s.nulls {
            let n = match nulls {
                NullsPosition::First => "FIRST",
                NullsPosition::Last => "LAST",
            };
            match driver {
                // MySQL has no NULLS FIRST/LAST; emulate with a leading key.
                DriverKind::Mysql => {
                    let flip = matches!(nulls, NullsPosition::First);
                    term = format!("{col} IS NOT NULL = {}, {term}", u8::from(flip));
                }
                _ => term.push_str(&format!(" NULLS {n}")),
            }
        }
        parts.push(term);
    }
    Ok(parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(column: &str, op: FilterOp, value: Option<Value>) -> FilterSpec {
        FilterSpec {
            column: column.into(),
            op: Some(op),
            raw: None,
            value,
            value2: None,
            values: Vec::new(),
            case_sensitive: None,
            disabled: false,
        }
    }

    #[test]
    fn no_filters_compiles_to_an_empty_clause() {
        let w = compile_where(DriverKind::Sqlite, &[], 1).unwrap();
        assert!(w.sql.is_empty());
        assert!(w.params.is_empty());
    }

    #[test]
    fn like_metacharacters_are_escaped_so_a_literal_percent_matches() {
        let w = compile_where(
            DriverKind::Sqlite,
            &[spec("code", FilterOp::Contains, Some(json!("50%")))],
            1,
        )
        .unwrap();
        // Without escaping this pattern would match every row.
        assert_eq!(w.params[0], json!("%50\\%%"));
        assert!(w.sql.contains("ESCAPE '\\'"), "got: {}", w.sql);
        // mysql: a lone backslash would escape the closing quote.
        let w = compile_where(
            DriverKind::Mysql,
            &[spec("code", FilterOp::Contains, Some(json!("50%")))],
            1,
        )
        .unwrap();
        assert!(w.sql.ends_with("ESCAPE '\\\\'"), "got: {}", w.sql);
    }

    #[test]
    fn underscore_and_the_escape_char_are_escaped_too() {
        let w = compile_where(
            DriverKind::Sqlite,
            &[spec("p", FilterOp::StartsWith, Some(json!("a_b\\c")))],
            1,
        )
        .unwrap();
        assert_eq!(w.params[0], json!("a\\_b\\\\c%"));
    }

    #[test]
    fn postgres_numbers_placeholders_and_others_do_not() {
        let filters = [
            spec("a", FilterOp::Equals, Some(json!(1))),
            spec("b", FilterOp::Equals, Some(json!(2))),
        ];
        let pg = compile_where(DriverKind::Postgres, &filters, 1).unwrap();
        assert_eq!(pg.sql, r#""a" = $1 AND "b" = $2"#);

        let my = compile_where(DriverKind::Mysql, &filters, 1).unwrap();
        assert_eq!(my.sql, "`a` = ? AND `b` = ?");
    }

    #[test]
    fn placeholder_numbering_continues_from_the_callers_offset() {
        let w = compile_where(
            DriverKind::Postgres,
            &[spec("a", FilterOp::Equals, Some(json!(1)))],
            4,
        )
        .unwrap();
        assert_eq!(w.sql, r#""a" = $4"#);
    }

    #[test]
    fn is_null_and_is_empty_are_different_questions() {
        let null =
            compile_where(DriverKind::Sqlite, &[spec("a", FilterOp::IsNull, None)], 1).unwrap();
        assert_eq!(null.sql, r#""a" IS NULL"#);
        assert!(null.params.is_empty());

        let empty =
            compile_where(DriverKind::Sqlite, &[spec("a", FilterOp::IsEmpty, None)], 1).unwrap();
        assert_eq!(empty.sql, r#"("a" IS NULL OR "a" = '')"#);
    }

    #[test]
    fn not_contains_keeps_null_rows() {
        let w = compile_where(
            DriverKind::Sqlite,
            &[spec("a", FilterOp::NotContains, Some(json!("x")))],
            1,
        )
        .unwrap();
        // A bare NOT LIKE would drop NULLs, which reads as data loss.
        assert!(
            w.sql.starts_with(r#"("a" IS NULL OR NOT ("#),
            "got: {}",
            w.sql
        );
    }

    #[test]
    fn case_insensitive_uses_ilike_on_postgres_only() {
        let pg = compile_where(
            DriverKind::Postgres,
            &[spec("a", FilterOp::Contains, Some(json!("x")))],
            1,
        )
        .unwrap();
        assert!(pg.sql.contains("ILIKE"), "got: {}", pg.sql);

        let lite = compile_where(
            DriverKind::Sqlite,
            &[spec("a", FilterOp::Contains, Some(json!("x")))],
            1,
        )
        .unwrap();
        assert!(lite.sql.contains("LIKE") && !lite.sql.contains("ILIKE"));
    }

    #[test]
    fn case_sensitive_flag_is_rejected_where_it_cannot_be_honoured() {
        let mut f = spec("a", FilterOp::Contains, Some(json!("x")));
        f.case_sensitive = Some(true);
        let err = compile_where(DriverKind::Sqlite, &[f], 1).unwrap_err();
        let body = serde_json::to_string(&err).unwrap();
        assert!(body.contains("postgres pattern operators"), "got: {body}");
    }

    #[test]
    fn an_incomplete_filter_is_refused_rather_than_guessed() {
        let err =
            compile_where(DriverKind::Sqlite, &[spec("a", FilterOp::Equals, None)], 1).unwrap_err();
        let body = serde_json::to_string(&err).unwrap();
        assert!(body.contains("needs `value`"), "got: {body}");

        let mut between = spec("a", FilterOp::Between, Some(json!(1)));
        between.value2 = None;
        let err = compile_where(DriverKind::Sqlite, &[between], 1).unwrap_err();
        let body = serde_json::to_string(&err).unwrap();
        assert!(body.contains("`value2`"), "got: {body}");
    }

    #[test]
    fn between_binds_both_bounds_in_order() {
        let mut f = spec("n", FilterOp::Between, Some(json!(1)));
        f.value2 = Some(json!(9));
        let w = compile_where(DriverKind::Postgres, &[f], 1).unwrap();
        assert_eq!(w.sql, r#""n" BETWEEN $1 AND $2"#);
        assert_eq!(w.params, vec![json!(1), json!(9)]);
    }

    #[test]
    fn quoting_defeats_an_identifier_break_out() {
        assert_eq!(
            quote_ident(DriverKind::Postgres, r#"a" OR 1=1--"#),
            r#""a"" OR 1=1--""#
        );
        assert_eq!(quote_ident(DriverKind::Mysql, "a`b"), "`a``b`");
    }

    #[test]
    fn table_is_schema_qualified_only_on_postgres() {
        assert_eq!(
            quote_table(DriverKind::Postgres, Some("analytics"), "events"),
            r#""analytics"."events""#
        );
        assert_eq!(
            quote_table(DriverKind::Mysql, Some("ignored"), "events"),
            "`events`"
        );
    }

    #[test]
    fn order_by_emits_direction_and_nulls_placement() {
        let sorts = [SortSpec {
            column: "created_at".into(),
            direction: Direction::Desc,
            nulls: Some(NullsPosition::Last),
            mode: SortMode::Default,
        }];
        let pg = compile_order_by(DriverKind::Postgres, &sorts).unwrap();
        assert_eq!(pg, r#""created_at" DESC NULLS LAST"#);

        // MySQL has no NULLS clause, so it emulates with a leading key.
        let my = compile_order_by(DriverKind::Mysql, &sorts).unwrap();
        assert!(my.contains("IS NOT NULL ="), "got: {my}");
        assert!(!my.contains("NULLS"), "got: {my}");
    }

    #[test]
    fn length_mode_uses_the_drivers_own_function() {
        let s = [SortSpec {
            column: "name".into(),
            direction: Direction::Asc,
            nulls: None,
            mode: SortMode::Length,
        }];
        assert_eq!(
            compile_order_by(DriverKind::Mysql, &s).unwrap(),
            "CHAR_LENGTH(`name`) ASC"
        );
        assert_eq!(
            compile_order_by(DriverKind::Sqlite, &s).unwrap(),
            r#"LENGTH("name") ASC"#
        );
    }

    #[test]
    fn the_wrong_operand_alone_is_refused_rather_than_unwrapped() {
        // `value2` without `value` used to satisfy a count-based arity check
        // and then panic on the missing `value`. Reachable from the wire.
        let mut f = spec("a", FilterOp::Equals, None);
        f.value2 = Some(json!(5));
        let err = compile_where(DriverKind::Sqlite, &[f], 1).unwrap_err();
        assert!(format!("{err:?}").contains("needs `value`"));
    }

    #[test]
    fn between_needs_both_bounds_not_just_a_count_of_two() {
        let mut f = spec("a", FilterOp::Between, None);
        f.value2 = Some(json!(5));
        assert!(compile_where(DriverKind::Sqlite, &[f], 1).is_err());
    }

    #[test]
    fn case_sensitive_is_refused_where_it_would_do_nothing() {
        // Not a pattern operator, so the flag can never apply — even on the
        // one driver that supports it for LIKE.
        let mut f = spec("a", FilterOp::Equals, Some(json!("x")));
        f.case_sensitive = Some(true);
        assert!(compile_where(DriverKind::Postgres, &[f], 1).is_err());

        // Still accepted where it is honoured.
        let mut ok = spec("a", FilterOp::Contains, Some(json!("x")));
        ok.case_sensitive = Some(true);
        assert!(compile_where(DriverKind::Postgres, &[ok], 1).is_ok());
    }

    #[test]
    fn in_binds_every_member_and_numbers_them_on_postgres() {
        let mut f = spec("status", FilterOp::In, None);
        f.values = vec![json!("open"), json!("held")];
        let c = compile_where(DriverKind::Postgres, &[f], 1).unwrap();
        assert_eq!(c.sql, r#""status" IN ($1, $2)"#);
        assert_eq!(c.params, vec![json!("open"), json!("held")]);
    }

    #[test]
    fn not_in_keeps_nulls() {
        // `NULL NOT IN (...)` is unknown, so a plain NOT IN silently drops
        // every NULL row. Asking for "not one of these" should keep them.
        let mut f = spec("status", FilterOp::NotIn, None);
        f.values = vec![json!("open")];
        let c = compile_where(DriverKind::Sqlite, &[f], 1).unwrap();
        assert_eq!(c.sql, r#"("status" IS NULL OR "status" NOT IN (?))"#);
    }

    #[test]
    fn an_empty_set_is_refused_rather_than_dropped() {
        let f = spec("status", FilterOp::In, None);
        let err = compile_where(DriverKind::Sqlite, &[f], 1).unwrap_err();
        assert!(format!("{err:?}").contains("at least one value"));
    }

    #[test]
    fn a_disabled_filter_is_skipped_but_its_neighbours_still_compile() {
        let mut off = spec("plan", FilterOp::Equals, Some(json!("free")));
        off.disabled = true;
        let on = spec("status", FilterOp::Equals, Some(json!("open")));
        let c = compile_where(DriverKind::Postgres, &[off, on], 1).unwrap();
        // Numbering must close up: the skipped filter must not burn $1.
        assert_eq!(c.sql, r#""status" = $1"#);
        assert_eq!(c.params, vec![json!("open")]);
    }

    #[test]
    fn a_disabled_filter_is_not_validated() {
        // Half-built filters are the normal state of a chip being edited;
        // disabling one must not turn the whole request into an error.
        let mut half = spec("", FilterOp::Equals, None);
        half.op = None;
        half.disabled = true;
        let c = compile_where(DriverKind::Sqlite, &[half], 1).unwrap();
        assert_eq!(c.sql, "");
    }

    #[test]
    fn natural_mode_falls_back_rather_than_faking_it() {
        let s = [SortSpec {
            column: "label".into(),
            direction: Direction::Asc,
            nulls: None,
            mode: SortMode::Natural,
        }];
        assert!(compile_order_by(DriverKind::Postgres, &s)
            .unwrap()
            .contains("regexp_replace"));
        // No portable equivalent — order plainly instead of approximating.
        assert_eq!(
            compile_order_by(DriverKind::Sqlite, &s).unwrap(),
            r#""label" ASC"#
        );
    }

    fn raw(text: &str) -> FilterSpec {
        FilterSpec {
            column: String::new(),
            op: None,
            raw: Some(text.into()),
            value: None,
            value2: None,
            values: Vec::new(),
            case_sensitive: None,
            disabled: false,
        }
    }

    const ALL: [DriverKind; 3] = [DriverKind::Postgres, DriverKind::Mysql, DriverKind::Sqlite];

    #[test]
    fn a_raw_filter_is_anded_in_its_own_parentheses() {
        let c = compile_where(
            DriverKind::Postgres,
            &[
                spec("a", FilterOp::Equals, Some(json!(1))),
                raw("price > 10 OR name = 'x'"),
            ],
            1,
        )
        .unwrap();
        assert_eq!(c.sql, r#""a" = $1 AND (price > 10 OR name = 'x')"#);
        // `column`/`op` are ignored with `raw`.
        let mut with_op = raw("n > 1");
        with_op.op = Some(FilterOp::IsNull);
        with_op.column = "ignored".into();
        assert_eq!(
            compile_where(DriverKind::Sqlite, &[with_op], 1)
                .unwrap()
                .sql,
            "(n > 1)"
        );
    }

    #[test]
    fn a_raw_filter_deserializes_alone_or_next_to_a_column_and_op() {
        for v in [
            json!({ "raw": "n > 1" }),
            json!({ "column": "", "op": "equals", "raw": "n > 1" }),
        ] {
            let f: FilterSpec = serde_json::from_value(v).unwrap();
            assert_eq!(
                compile_where(DriverKind::Postgres, &[f], 1).unwrap().sql,
                "(n > 1)"
            );
        }
        // Without raw, op is still required.
        let f: FilterSpec = serde_json::from_value(json!({ "column": "a" })).unwrap();
        assert!(compile_where(DriverKind::Postgres, &[f], 1).is_err());
    }

    #[test]
    fn raw_filters_that_could_leave_their_parentheses_are_refused() {
        for driver in ALL {
            for bad in [
                "1=1; DELETE FROM t",
                "1=1) OR (1=1",
                "1=1) UNION SELECT * FROM secrets WHERE (1=1",
                "x = 1 -- ) trailing",
                "x = 1 /* ) */",
                "((x = 1)",
                "x = 'unterminated",
                ")",
                "",
                "   ",
            ] {
                assert!(
                    compile_where(driver, &[raw(bad)], 1).is_err(),
                    "{driver:?} accepted {bad:?}"
                );
            }
        }
        // Dialect-specific comment / placeholder spellings.
        assert!(compile_where(DriverKind::Mysql, &[raw("x = 1 # )")], 1).is_err());
        assert!(compile_where(DriverKind::Mysql, &[raw("x = ?")], 1).is_err());
        assert!(compile_where(DriverKind::Sqlite, &[raw("x = ?")], 1).is_err());
        assert!(compile_where(DriverKind::Postgres, &[raw("x = $1")], 1).is_err());
        // pg's `#` is XOR, not a comment.
        assert!(compile_where(DriverKind::Postgres, &[raw("(a # b) = 0")], 1).is_ok());
    }

    #[test]
    fn quoting_is_lexed_like_the_server_lexes_it() {
        // Parens, semicolons and dashes inside quotes are data.
        for driver in ALL {
            assert!(
                compile_where(driver, &[raw("name = 'a); DELETE --'")], 1).is_ok(),
                "{driver:?}"
            );
            assert!(
                compile_where(driver, &[raw("name = 'it''s'")], 1).is_ok(),
                "{driver:?}"
            );
        }
        assert!(compile_where(DriverKind::Postgres, &[raw(r#""we;ird)" = 1"#)], 1).is_ok());
        assert!(compile_where(DriverKind::Mysql, &[raw("`we;ird)` = 1")], 1).is_ok());
        assert!(compile_where(DriverKind::Sqlite, &[raw("[we;ird)] = 1")], 1).is_ok());
        // A sqlite bracket identifier cannot be used to hide a quote: the
        // quote inside it is part of the identifier, and the `)` after it is
        // code.
        assert!(compile_where(
            DriverKind::Sqlite,
            &[raw("[x'] = 1 ) ; DELETE FROM t --'")],
            1
        )
        .is_err());
        // Postgres dollar quotes: what is inside is data; a quote inside one
        // does not open a string that could hide a `)`.
        assert!(compile_where(DriverKind::Postgres, &[raw("note = $$a);--$$")], 1).is_ok());
        assert!(compile_where(
            DriverKind::Postgres,
            &[raw("note = $q$ ' $q$ ) OR (1=1")],
            1
        )
        .is_err());
        // `$` inside an identifier is not a dollar quote: `a$b$ ) $b$` is an
        // identifier followed by a stray `)`.
        assert!(compile_where(DriverKind::Postgres, &[raw("a$b$ ) $b$ = 1")], 1).is_err());
        assert!(compile_where(DriverKind::Postgres, &[raw("price$usd > 1")], 1).is_ok());
        // E-strings take backslash escapes on postgres.
        assert!(compile_where(DriverKind::Postgres, &[raw(r"note = E'a\') x'")], 1).is_ok());
        // Where a backslash's meaning depends on server settings, refuse it.
        assert!(compile_where(
            DriverKind::Postgres,
            &[raw(r"note = 'a\' ) OR (1=1 --'")],
            1
        )
        .is_err());
        assert!(compile_where(DriverKind::Mysql, &[raw(r"note = 'a\' ) OR (1=1'")], 1).is_err());
        assert!(compile_where(DriverKind::Mysql, &[raw(r#"note = "a\" ) x""#)], 1).is_err());
        // sqlite has no backslash escapes at all.
        assert!(compile_where(DriverKind::Sqlite, &[raw(r"note = 'a\'")], 1).is_ok());
    }

    #[test]
    fn postgres_values_are_cast_to_the_column_type_from_the_catalog() {
        let types: std::collections::HashMap<String, String> = [
            ("total", "numeric(10,2)"),
            ("placed_at", "timestamp with time zone"),
            ("id", "uuid"),
            ("name", "character varying(255)"),
            ("ip", "inet"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let mut between = spec("placed_at", FilterOp::Between, Some(json!("2026-01-01")));
        between.value2 = Some(json!("2026-02-01"));
        let mut any_of = spec("id", FilterOp::In, None);
        any_of.values = vec![json!("6f1c2a8e-3b5d-4c7e-9f10-2a3b4c5d6e7f")];
        let filters = [
            spec("total", FilterOp::Gt, Some(json!(100))),
            between,
            any_of,
            spec("ip", FilterOp::Contains, Some(json!("192.168"))),
            spec("name", FilterOp::Contains, Some(json!("an"))),
        ];
        let c = compile_where_typed(DriverKind::Postgres, &filters, 1, Some(&types)).unwrap();
        assert!(
            c.sql.contains(r#""total" > $1::text::numeric(10,2)"#),
            "{}",
            c.sql
        );
        assert!(
            c.sql.contains(
                "BETWEEN $2::text::timestamp with time zone AND $3::text::timestamp with time zone"
            ),
            "{}",
            c.sql
        );
        assert!(c.sql.contains(r#""id" IN ($4::text::uuid)"#), "{}", c.sql);
        assert!(
            c.sql.contains(r#"CAST("ip" AS TEXT) ILIKE $5"#),
            "{}",
            c.sql
        );
        assert!(c.sql.contains(r#""name" ILIKE $6"#), "{}", c.sql);
        // Numbers travel as their exact text.
        assert_eq!(c.params[0], json!("100"));
        // A column the catalog does not list is refused up front.
        let err = compile_where_typed(
            DriverKind::Postgres,
            &[spec("nope", FilterOp::Equals, Some(json!(1)))],
            1,
            Some(&types),
        )
        .unwrap_err();
        assert!(err.to_string().contains("no column `nope`"), "{err}");
        // Untyped (or other drivers): unchanged.
        let plain = compile_where(DriverKind::Mysql, &filters[..1], 1).unwrap();
        assert_eq!(plain.sql, "`total` > ?");
    }
}
