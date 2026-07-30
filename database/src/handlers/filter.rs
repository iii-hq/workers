//! Typed filters and sorts, compiled to dialect SQL here rather than in the
//! caller.
//!
//! A caller says "email contains test and plan equals free" as data; this
//! module turns that into a parameterised `WHERE` clause for the driver in
//! hand. That matters for correctness as much as convenience — the traps
//! below are ones every hand-written client filter gets wrong at least once:
//!
//! * **LIKE metacharacters.** `contains "50%"` must match a literal percent,
//!   not every row. Values are escaped and the clause carries `ESCAPE '\'`.
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
    /// Set membership, over `values`. Expressing "status is one of open,
    /// pending, held" as three OR'd equality filters is not possible here —
    /// filters stack with AND — so without this the question cannot be asked
    /// at all.
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
    pub column: String,
    pub op: FilterOp,
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
    /// Kept in the list but not applied. A caller refining a query wants to
    /// switch one condition off and back on without losing how it was built,
    /// and a console that only offers delete makes that a retype.
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

/// Type-aware sort modes. These exist server-side because the grid is paged:
/// sorting the fetched page would order 50 rows out of N, which is a
/// different answer from sorting the table.
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
    let mut ph = Placeholders::new(driver, start_param);
    let mut parts: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();

    for f in filters {
        // A disabled filter is skipped, not rejected: it stays in the caller's
        // list so it can be switched back on without being rebuilt.
        if f.disabled {
            continue;
        }
        if f.column.trim().is_empty() {
            return Err(invalid("filter is missing a column"));
        }
        // Check operands by position, not by count. Counting accepts
        // `{op: equals, value2: 5}` — one operand supplied for a one-operand
        // operator — and the compiler then unwraps the `value` that is not
        // there. The request body is caller-controlled, so that is a panic
        // reachable from the wire.
        let arity = f.op.arity();
        if arity >= 1 && f.value.is_none() {
            return Err(invalid(format!("filter on `{}` needs `value`", f.column)));
        }
        if arity >= 2 && f.value2.is_none() {
            return Err(invalid(format!(
                "filter on `{}` needs both `value` and `value2`",
                f.column
            )));
        }
        if f.op.is_set() && f.values.is_empty() {
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
        if f.case_sensitive.is_some() && !(driver == DriverKind::Postgres && f.op.is_like()) {
            return Err(invalid(
                "case_sensitive applies only to postgres pattern operators; \
                 elsewhere it is determined by the column collation",
            ));
        }

        let col = quote_ident(driver, &f.column);
        let sensitive = f.case_sensitive.unwrap_or(false);

        let part = match f.op {
            FilterOp::IsNull => format!("{col} IS NULL"),
            FilterOp::IsNotNull => format!("{col} IS NOT NULL"),
            FilterOp::IsTrue => format!("{col} = TRUE"),
            FilterOp::IsFalse => format!("{col} = FALSE"),
            // NULL *or* empty string — the distinction from is_null is the point.
            FilterOp::IsEmpty => format!("({col} IS NULL OR {col} = '')"),

            FilterOp::In | FilterOp::NotIn => {
                let marks: Vec<String> = f
                    .values
                    .iter()
                    .map(|v| {
                        params.push(v.clone());
                        ph.take()
                    })
                    .collect();
                let list = marks.join(", ");
                if f.op == FilterOp::In {
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
                let pattern = match f.op {
                    FilterOp::StartsWith => format!("{escaped}%"),
                    FilterOp::EndsWith => format!("%{escaped}"),
                    _ => format!("%{escaped}%"),
                };
                let negate = f.op == FilterOp::NotContains;
                let op = match (driver, sensitive) {
                    (DriverKind::Postgres, false) => "ILIKE",
                    _ => "LIKE",
                };
                params.push(Value::String(pattern));
                let p = ph.take();
                let expr = format!("{col} {op} {p} ESCAPE '{LIKE_ESCAPE}'");
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
                params.push(f.value.clone().expect("arity checked"));
                let lo = ph.take();
                params.push(f.value2.clone().expect("arity checked"));
                let hi = ph.take();
                format!("{col} BETWEEN {lo} AND {hi}")
            }

            _ => {
                let sym = match f.op {
                    FilterOp::Equals => "=",
                    FilterOp::NotEquals => "<>",
                    FilterOp::Gt => ">",
                    FilterOp::Gte => ">=",
                    FilterOp::Lt => "<",
                    FilterOp::Lte => "<=",
                    other => unreachable!("{other:?} handled above"),
                };
                params.push(f.value.clone().expect("arity checked"));
                let p = ph.take();
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
            op,
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
}
