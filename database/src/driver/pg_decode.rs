//! Postgres result cells → [`RowValue`], for every type.
//!
//! tokio-postgres reads results in the binary format and its typed
//! `FromSql` impls cover a fixed list of OIDs; anything else (`date`,
//! `interval`, enums, arrays, `inet`, `money`, extension types…) used to fail
//! with "error deserializing column N" and take the whole query down. Here
//! every cell is read as raw bytes ([`Raw`]) and decoded by type:
//!
//! | type | JSON |
//! |---|---|
//! | `int2`, `int4`, `oid` | number |
//! | `int8` | string (exact beyond 2^53) |
//! | `numeric`, `money` | string (`money` without currency symbol) |
//! | `float4`, `float8` | number; `NaN`/`Infinity` as strings |
//! | `bool` | boolean |
//! | text family, `xml`, enums, `"char"` | string |
//! | `json`, `jsonb` | the JSON value |
//! | `uuid` | string |
//! | `bytea` | base64 string |
//! | `date` | `"YYYY-MM-DD"` |
//! | `time` / `timetz` | `"HH:MM:SS[.ffffff]"` / `…+HH[:MM]` |
//! | `timestamp`, `timestamptz` | RFC 3339 with `Z`, fraction when non-zero (a `timestamp` is read as UTC — the wire shape this worker has always had) |
//! | `interval` | postgres text form (`1 year 2 mons 3 days 04:05:06`) |
//! | `inet`, `cidr`, `macaddr[8]`, `point`, `bit`, `tsvector`, ranges | postgres text form |
//! | arrays | JSON arrays (nested for several dimensions) of the element encoding |
//! | composites | JSON objects by field name |
//! | domains | the base type's encoding |
//! | anything else | the bytes as text when they are printable UTF-8, else `\x…` hex |
//!
//! Infinite dates/timestamps come back as `"infinity"` / `"-infinity"`. A
//! cell that cannot be decoded never fails the query: it falls back to text
//! or hex (the table's last row), with a debug log.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use postgres_types::{FromSql, Kind, Type};
use serde_json::{Map, Value};

use crate::value::RowValue;

/// A cell's raw wire bytes, whatever its type.
pub(crate) struct Raw<'a>(pub &'a [u8]);

impl<'a> FromSql<'a> for Raw<'a> {
    fn from_sql(
        _ty: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Raw(raw))
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }
}

/// Days from 1970-01-01 to postgres' epoch, 2000-01-01.
fn pg_epoch_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2000, 1, 1).expect("valid date")
}

fn be_i16(b: &[u8]) -> Option<i16> {
    Some(i16::from_be_bytes(b.get(..2)?.try_into().ok()?))
}
fn be_i32(b: &[u8]) -> Option<i32> {
    Some(i32::from_be_bytes(b.get(..4)?.try_into().ok()?))
}
fn be_u32(b: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(..4)?.try_into().ok()?))
}
fn be_i64(b: &[u8]) -> Option<i64> {
    Some(i64::from_be_bytes(b.get(..8)?.try_into().ok()?))
}
fn be_f32(b: &[u8]) -> Option<f32> {
    Some(f32::from_be_bytes(b.get(..4)?.try_into().ok()?))
}
fn be_f64(b: &[u8]) -> Option<f64> {
    Some(f64::from_be_bytes(b.get(..8)?.try_into().ok()?))
}

fn float(f: f64) -> RowValue {
    if f.is_finite() {
        RowValue::Float(f)
    } else if f.is_nan() {
        RowValue::Text("NaN".into())
    } else if f > 0.0 {
        RowValue::Text("Infinity".into())
    } else {
        RowValue::Text("-Infinity".into())
    }
}

/// `HH:MM:SS[.ffffff]` from microseconds (may exceed 24h for `time` = 24:00).
fn clock(micros: i64) -> String {
    let neg = micros < 0;
    let micros = micros.unsigned_abs();
    let secs = micros / 1_000_000;
    let frac = micros % 1_000_000;
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    let mut out = format!("{}{h:02}:{m:02}:{s:02}", if neg { "-" } else { "" });
    if frac > 0 {
        let f = format!("{frac:06}");
        out.push('.');
        out.push_str(f.trim_end_matches('0'));
    }
    out
}

fn naive_from_micros(micros: i64) -> Option<NaiveDateTime> {
    let base = pg_epoch_date().and_hms_opt(0, 0, 0)?;
    base.checked_add_signed(Duration::microseconds(micros))
}

/// The postgres (`IntervalStyle = postgres`) text form.
fn interval_text(micros: i64, days: i32, months: i32) -> String {
    let mut parts: Vec<String> = Vec::new();
    let plural =
        |n: i64, one: &str, many: &str| format!("{n} {}", if n.abs() == 1 { one } else { many });
    let (years, mons) = (months / 12, months % 12);
    if years != 0 {
        parts.push(plural(years.into(), "year", "years"));
    }
    if mons != 0 {
        parts.push(plural(mons.into(), "mon", "mons"));
    }
    if days != 0 {
        parts.push(plural(days.into(), "day", "days"));
    }
    if micros != 0 || parts.is_empty() {
        parts.push(clock(micros));
    }
    parts.join(" ")
}

fn hex(raw: &[u8]) -> String {
    let mut out = String::with_capacity(2 + raw.len() * 2);
    out.push_str("\\x");
    for b in raw {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Printable UTF-8 as text, anything else as `\x…` hex (postgres' bytea
/// text form).
fn text_or_hex(raw: &[u8]) -> RowValue {
    match std::str::from_utf8(raw) {
        Ok(s)
            if !s
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r')) =>
        {
            RowValue::Text(s.to_string())
        }
        _ => RowValue::Text(hex(raw)),
    }
}

fn utf8(raw: &[u8]) -> RowValue {
    RowValue::Text(String::from_utf8_lossy(raw).into_owned())
}

fn inet(raw: &[u8], cidr: bool) -> Option<String> {
    let (family, bits, nb) = (*raw.first()?, *raw.get(1)?, *raw.get(3)? as usize);
    let addr = raw.get(4..4 + nb)?;
    let (text, full) = match family {
        2 if nb == 4 => (
            std::net::Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]).to_string(),
            32,
        ),
        3 if nb == 16 => {
            let octets: [u8; 16] = addr.try_into().ok()?;
            (std::net::Ipv6Addr::from(octets).to_string(), 128)
        }
        _ => return None,
    };
    Some(if cidr || bits != full {
        format!("{text}/{bits}")
    } else {
        text
    })
}

fn bits(raw: &[u8]) -> Option<String> {
    let n = be_i32(raw)? as usize;
    let data = raw.get(4..)?;
    let mut out = String::with_capacity(n);
    for i in 0..n {
        let byte = *data.get(i / 8)?;
        out.push(if byte & (0x80 >> (i % 8)) != 0 {
            '1'
        } else {
            '0'
        });
    }
    Some(out)
}

fn tsvector(raw: &[u8]) -> Option<String> {
    let count = be_i32(raw)? as usize;
    let mut pos = 4;
    let mut words = Vec::with_capacity(count);
    for _ in 0..count {
        let end = pos + raw.get(pos..)?.iter().position(|&b| b == 0)?;
        let lexeme = std::str::from_utf8(&raw[pos..end]).ok()?;
        pos = end + 1;
        let npos = u16::from_be_bytes(raw.get(pos..pos + 2)?.try_into().ok()?) as usize;
        pos += 2;
        let mut word = format!("'{}'", lexeme.replace('\'', "''"));
        let mut positions = Vec::with_capacity(npos);
        for _ in 0..npos {
            let p = u16::from_be_bytes(raw.get(pos..pos + 2)?.try_into().ok()?);
            pos += 2;
            let weight = match p >> 14 {
                3 => "A",
                2 => "B",
                1 => "C",
                _ => "",
            };
            positions.push(format!("{}{weight}", p & 0x3fff));
        }
        if !positions.is_empty() {
            word.push(':');
            word.push_str(&positions.join(","));
        }
        words.push(word);
    }
    Some(words.join(" "))
}

/// A JSON value's bare text form (for range bounds and the like).
fn bare_text(v: &RowValue) -> String {
    match v.to_json() {
        Value::String(s) => s,
        other => other.to_string(),
    }
}

fn range(raw: &[u8], subtype: &Type) -> Option<String> {
    let flags = *raw.first()?;
    if flags & 0x01 != 0 {
        return Some("empty".into());
    }
    let mut pos = 1;
    let mut bound = |present: bool| -> Option<String> {
        if !present {
            return Some(String::new());
        }
        let len = be_i32(raw.get(pos..)?)? as usize;
        pos += 4;
        let v = decode(subtype, raw.get(pos..pos + len)?);
        pos += len;
        Some(bare_text(&v))
    };
    let lower = bound(flags & 0x08 == 0)?;
    let upper = bound(flags & 0x10 == 0)?;
    Some(format!(
        "{}{lower},{upper}{}",
        if flags & 0x02 != 0 { '[' } else { '(' },
        if flags & 0x04 != 0 { ']' } else { ')' }
    ))
}

fn array(raw: &[u8], member: &Type) -> Option<Value> {
    let ndim = be_i32(raw)? as usize;
    if ndim == 0 {
        return Some(Value::Array(Vec::new()));
    }
    let mut dims = Vec::with_capacity(ndim);
    let mut pos = 12;
    for _ in 0..ndim {
        dims.push(be_i32(raw.get(pos..)?)? as usize);
        pos += 8;
    }
    let total: usize = dims.iter().product();
    let mut elements = Vec::with_capacity(total.min(1 << 20));
    for _ in 0..total {
        let len = be_i32(raw.get(pos..)?)?;
        pos += 4;
        if len < 0 {
            elements.push(Value::Null);
            continue;
        }
        let len = len as usize;
        elements.push(decode(member, raw.get(pos..pos + len)?).into_json());
        pos += len;
    }
    // Fold the flat list into nested arrays, innermost dimension first.
    for &dim in dims.iter().skip(1).rev() {
        if dim == 0 {
            break;
        }
        elements = elements
            .chunks(dim)
            .map(|chunk| Value::Array(chunk.to_vec()))
            .collect();
    }
    Some(Value::Array(elements))
}

fn composite(raw: &[u8], fields: &[postgres_types::Field]) -> Option<Value> {
    let count = be_i32(raw)? as usize;
    let mut pos = 4;
    let mut out = Map::new();
    for i in 0..count {
        let _oid = be_u32(raw.get(pos..)?)?;
        pos += 4;
        let len = be_i32(raw.get(pos..)?)?;
        pos += 4;
        let field = fields.get(i);
        let name = field.map_or_else(|| format!("f{}", i + 1), |f| f.name().to_string());
        let value = if len < 0 {
            Value::Null
        } else {
            let len = len as usize;
            let bytes = raw.get(pos..pos + len)?;
            pos += len;
            match field {
                Some(f) => decode(f.type_(), bytes).into_json(),
                None => text_or_hex(bytes).into_json(),
            }
        };
        out.insert(name, value);
    }
    Some(Value::Object(out))
}

/// Decode one non-NULL cell. Never fails: unknown or malformed input falls
/// back to text / hex.
pub(crate) fn decode(ty: &Type, raw: &[u8]) -> RowValue {
    decode_known(ty, raw).unwrap_or_else(|| {
        tracing::debug!(r#type = %ty.name(), "postgres value decoded as text fallback");
        text_or_hex(raw)
    })
}

fn decode_known(ty: &Type, raw: &[u8]) -> Option<RowValue> {
    match ty.kind() {
        Kind::Array(member) => return array(raw, member).map(RowValue::Json),
        Kind::Enum(_) => return Some(utf8(raw)),
        Kind::Domain(base) => return Some(decode(base, raw)),
        Kind::Composite(fields) => return composite(raw, fields).map(RowValue::Json),
        Kind::Range(subtype) => return range(raw, subtype).map(RowValue::Text),
        _ => {}
    }
    Some(match *ty {
        Type::BOOL => RowValue::Bool(*raw.first()? != 0),
        Type::INT2 => RowValue::Int(be_i16(raw)?.into()),
        Type::INT4 => RowValue::Int(be_i32(raw)?.into()),
        Type::INT8 => RowValue::BigInt(be_i64(raw)?),
        Type::OID | Type::XID | Type::CID | Type::REGCLASS | Type::REGPROC | Type::REGTYPE => {
            RowValue::Int(be_u32(raw)?.into())
        }
        Type::FLOAT4 => float(be_f32(raw)?.into()),
        Type::FLOAT8 => float(be_f64(raw)?),
        Type::NUMERIC => RowValue::Decimal(super::postgres::stringify_pg_numeric_binary(raw).ok()?),
        Type::MONEY => {
            let cents = be_i64(raw)?;
            let sign = if cents < 0 { "-" } else { "" };
            let abs = cents.unsigned_abs();
            RowValue::Decimal(format!("{sign}{}.{:02}", abs / 100, abs % 100))
        }
        Type::TEXT
        | Type::VARCHAR
        | Type::BPCHAR
        | Type::NAME
        | Type::UNKNOWN
        | Type::XML
        | Type::CSTRING => utf8(raw),
        Type::CHAR => RowValue::Text((*raw.first()? as char).to_string()),
        Type::JSON => match serde_json::from_slice(raw) {
            Ok(v) => RowValue::Json(v),
            Err(_) => utf8(raw),
        },
        Type::JSONB => {
            // Version byte (1), then the JSON text.
            let body = raw.get(1..)?;
            match serde_json::from_slice(body) {
                Ok(v) => RowValue::Json(v),
                Err(_) => utf8(body),
            }
        }
        Type::UUID => RowValue::Text(uuid::Uuid::from_slice(raw).ok()?.to_string()),
        Type::BYTEA => RowValue::Bytes(raw.to_vec()),
        Type::DATE => {
            let days = be_i32(raw)?;
            match days {
                i32::MAX => RowValue::Text("infinity".into()),
                i32::MIN => RowValue::Text("-infinity".into()),
                d => RowValue::Text(
                    pg_epoch_date()
                        .checked_add_signed(Duration::days(d.into()))?
                        .format("%Y-%m-%d")
                        .to_string(),
                ),
            }
        }
        Type::TIME => RowValue::Text(clock(be_i64(raw)?)),
        Type::TIMETZ => {
            let micros = be_i64(raw)?;
            // Seconds WEST of UTC, so the sign flips for display.
            let west = be_i32(raw.get(8..)?)?;
            let east = -west;
            let sign = if east < 0 { '-' } else { '+' };
            let e = east.unsigned_abs();
            let (h, m, s) = (e / 3600, (e / 60) % 60, e % 60);
            let mut zone = format!("{sign}{h:02}");
            if m != 0 || s != 0 {
                zone.push_str(&format!(":{m:02}"));
            }
            if s != 0 {
                zone.push_str(&format!(":{s:02}"));
            }
            RowValue::Text(format!("{}{zone}", clock(micros)))
        }
        Type::TIMESTAMP | Type::TIMESTAMPTZ => match be_i64(raw)? {
            i64::MAX => RowValue::Text("infinity".into()),
            i64::MIN => RowValue::Text("-infinity".into()),
            micros => {
                let dt: DateTime<Utc> = Utc.from_utc_datetime(&naive_from_micros(micros)?);
                RowValue::Timestamp(dt)
            }
        },
        Type::INTERVAL => RowValue::Text(interval_text(
            be_i64(raw)?,
            be_i32(raw.get(8..)?)?,
            be_i32(raw.get(12..)?)?,
        )),
        Type::INET => RowValue::Text(inet(raw, false)?),
        Type::CIDR => RowValue::Text(inet(raw, true)?),
        Type::MACADDR | Type::MACADDR8 => RowValue::Text(
            raw.iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(":"),
        ),
        Type::POINT => RowValue::Text(format!("({},{})", be_f64(raw)?, be_f64(raw.get(8..)?)?)),
        Type::BIT | Type::VARBIT => RowValue::Text(bits(raw)?),
        Type::TS_VECTOR => RowValue::Text(tsvector(raw)?),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dates_times_and_intervals_use_postgres_text_forms() {
        // 2026-10-18 is 9787 days after 2000-01-01.
        let days =
            (NaiveDate::from_ymd_opt(2026, 10, 18).unwrap() - pg_epoch_date()).num_days() as i32;
        assert_eq!(
            decode(&Type::DATE, &days.to_be_bytes()).to_json(),
            json!("2026-10-18")
        );
        assert_eq!(
            decode(&Type::DATE, &i32::MAX.to_be_bytes()).to_json(),
            json!("infinity")
        );
        let t = (23 * 3600 + 59 * 60 + 59) as i64 * 1_000_000 + 500_000;
        assert_eq!(
            decode(&Type::TIME, &t.to_be_bytes()).to_json(),
            json!("23:59:59.5")
        );
        let mut tz = t.to_be_bytes().to_vec();
        tz.extend_from_slice(&(-3 * 3600i32).to_be_bytes()); // UTC+03
        assert_eq!(decode(&Type::TIMETZ, &tz).to_json(), json!("23:59:59.5+03"));
        let mut iv = (4 * 3600 + 5 * 60 + 6i64)
            .checked_mul(1_000_000)
            .unwrap()
            .to_be_bytes()
            .to_vec();
        iv.extend_from_slice(&3i32.to_be_bytes());
        iv.extend_from_slice(&14i32.to_be_bytes());
        assert_eq!(
            decode(&Type::INTERVAL, &iv).to_json(),
            json!("1 year 2 mons 3 days 04:05:06")
        );
        let mut zero = 0i64.to_be_bytes().to_vec();
        zero.extend_from_slice(&[0; 8]);
        assert_eq!(decode(&Type::INTERVAL, &zero).to_json(), json!("00:00:00"));
    }

    #[test]
    fn timestamps_keep_fractions_and_infinity() {
        let micros = 1_000_000i64 * 86_400 + 123_456; // 2000-01-02T00:00:00.123456
        assert_eq!(
            decode(&Type::TIMESTAMP, &micros.to_be_bytes()).to_json(),
            json!("2000-01-02T00:00:00.123456Z")
        );
        assert_eq!(
            decode(&Type::TIMESTAMPTZ, &micros.to_be_bytes()).to_json(),
            json!("2000-01-02T00:00:00.123456Z")
        );
        assert_eq!(
            decode(&Type::TIMESTAMPTZ, &i64::MIN.to_be_bytes()).to_json(),
            json!("-infinity")
        );
    }

    #[test]
    fn network_and_misc_types_render_like_psql() {
        assert_eq!(
            decode(&Type::INET, &[2, 32, 0, 4, 192, 168, 0, 1]).to_json(),
            json!("192.168.0.1")
        );
        assert_eq!(
            decode(&Type::INET, &[2, 24, 0, 4, 10, 0, 0, 0]).to_json(),
            json!("10.0.0.0/24")
        );
        assert_eq!(
            decode(&Type::CIDR, &[2, 32, 1, 4, 10, 0, 0, 1]).to_json(),
            json!("10.0.0.1/32")
        );
        let mut v6 = vec![3, 128, 0, 16];
        v6.extend_from_slice(&std::net::Ipv6Addr::LOCALHOST.octets());
        assert_eq!(decode(&Type::INET, &v6).to_json(), json!("::1"));
        assert_eq!(
            decode(&Type::MACADDR, &[8, 0, 0x2b, 1, 2, 3]).to_json(),
            json!("08:00:2b:01:02:03")
        );
        assert_eq!(
            decode(&Type::MONEY, &(-123_456i64).to_be_bytes()).to_json(),
            json!("-1234.56")
        );
        let mut pt = 1.5f64.to_be_bytes().to_vec();
        pt.extend_from_slice(&(-2f64).to_be_bytes());
        assert_eq!(decode(&Type::POINT, &pt).to_json(), json!("(1.5,-2)"));
        let mut bit = 5i32.to_be_bytes().to_vec();
        bit.push(0b1010_1000);
        assert_eq!(decode(&Type::BIT, &bit).to_json(), json!("10101"));
        assert_eq!(
            decode(&Type::FLOAT8, &f64::NAN.to_be_bytes()).to_json(),
            json!("NaN")
        );
    }

    #[test]
    fn tsvector_renders_lexemes_with_positions_and_weights() {
        let mut raw = 2i32.to_be_bytes().to_vec();
        raw.extend_from_slice(b"cat\0");
        raw.extend_from_slice(&1u16.to_be_bytes());
        raw.extend_from_slice(&((3u16 << 14) | 2).to_be_bytes());
        raw.extend_from_slice(b"it's\0");
        raw.extend_from_slice(&0u16.to_be_bytes());
        assert_eq!(
            decode(&Type::TS_VECTOR, &raw).to_json(),
            json!("'cat':2A 'it''s'")
        );
    }

    fn array_bytes(elem_oid: u32, dims: &[i32], items: &[Option<&[u8]>]) -> Vec<u8> {
        let mut raw = (dims.len() as i32).to_be_bytes().to_vec();
        raw.extend_from_slice(&(items.iter().any(Option::is_none) as i32).to_be_bytes());
        raw.extend_from_slice(&elem_oid.to_be_bytes());
        for d in dims {
            raw.extend_from_slice(&d.to_be_bytes());
            raw.extend_from_slice(&1i32.to_be_bytes());
        }
        for item in items {
            match item {
                None => raw.extend_from_slice(&(-1i32).to_be_bytes()),
                Some(b) => {
                    raw.extend_from_slice(&(b.len() as i32).to_be_bytes());
                    raw.extend_from_slice(b);
                }
            }
        }
        raw
    }

    #[test]
    fn arrays_become_json_arrays_nested_by_dimension() {
        let one = 1i32.to_be_bytes();
        let two = 2i32.to_be_bytes();
        let three = 3i32.to_be_bytes();
        let four = 4i32.to_be_bytes();
        let raw = array_bytes(23, &[3], &[Some(&one), None, Some(&three)]);
        assert_eq!(
            decode(&Type::INT4_ARRAY, &raw).to_json(),
            json!([1, null, 3])
        );
        let raw = array_bytes(
            23,
            &[2, 2],
            &[Some(&one), Some(&two), Some(&three), Some(&four)],
        );
        assert_eq!(
            decode(&Type::INT4_ARRAY, &raw).to_json(),
            json!([[1, 2], [3, 4]])
        );
        let raw = array_bytes(25, &[2], &[Some(b"a"), Some(b"b c")]);
        assert_eq!(
            decode(&Type::TEXT_ARRAY, &raw).to_json(),
            json!(["a", "b c"])
        );
        let empty = [0u8; 12];
        assert_eq!(decode(&Type::INT4_ARRAY, &empty).to_json(), json!([]));
    }

    #[test]
    fn unknown_types_fall_back_to_text_or_hex_never_an_error() {
        let ty = Type::new("citext".into(), 99_999, Kind::Simple, "public".into());
        assert_eq!(decode(&ty, b"Hello").to_json(), json!("Hello"));
        assert_eq!(decode(&ty, &[0, 1, 0xff]).to_json(), json!("\\x0001ff"));
        // Truncated input for a known type falls back too.
        assert_eq!(decode(&Type::INT8, &[1, 2]).to_json(), json!("\\x0102"));
    }

    #[test]
    fn enums_domains_and_ranges() {
        let status = Type::new(
            "order_status".into(),
            90_001,
            Kind::Enum(vec!["paid".into()]),
            "public".into(),
        );
        assert_eq!(decode(&status, b"paid").to_json(), json!("paid"));
        let positive = Type::new(
            "positive".into(),
            90_002,
            Kind::Domain(Type::INT4),
            "public".into(),
        );
        assert_eq!(decode(&positive, &7i32.to_be_bytes()).to_json(), json!(7));
        let mut r = vec![0x02u8];
        r.extend_from_slice(&4i32.to_be_bytes());
        r.extend_from_slice(&1i32.to_be_bytes());
        r.extend_from_slice(&4i32.to_be_bytes());
        r.extend_from_slice(&10i32.to_be_bytes());
        assert_eq!(decode(&Type::INT4_RANGE, &r).to_json(), json!("[1,10)"));
        assert_eq!(decode(&Type::INT4_RANGE, &[0x01]).to_json(), json!("empty"));
    }
}
