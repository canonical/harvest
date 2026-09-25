use std::error::Error;

use anyhow::{anyhow, Result};
use bytes::BytesMut;
use chrono::{DateTime, FixedOffset, Utc};
use serde_json::Value;
use tokio_postgres::types::{to_sql_checked, IsNull, ToSql, Type};

type BoxError = Box<dyn Error + Sync + Send>;

/// Rewrites `$name` placeholders to positional `$1…$n` and returns the matching values,
/// skipping string literals, quoted identifiers, comments and dollar-quoted strings.
pub fn bind_named(sql: &str, params: &Value) -> Result<(String, Vec<Value>)> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let mut names: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' | '"' => {
                let end = skip_quoted(&chars, i, c);
                out.extend(&chars[i..end]);
                i = end;
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                let end = chars[i..].iter().position(|&ch| ch == '\n').map_or(chars.len(), |p| i + p);
                out.extend(&chars[i..end]);
                i = end;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                let end = find_seq(&chars, i + 2, &['*', '/']).map_or(chars.len(), |p| p + 2);
                out.extend(&chars[i..end]);
                i = end;
            }
            '$' => {
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
                    end += 1;
                }
                let ident: String = chars[start..end].iter().collect();
                if chars.get(end) == Some(&'$') {
                    let tag: Vec<char> = chars[i..=end].to_vec();
                    let close = find_seq(&chars, end + 1, &tag).map_or(chars.len(), |p| p + tag.len());
                    out.extend(&chars[i..close]);
                    i = close;
                } else if ident.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_') {
                    let position = match names.iter().position(|n| *n == ident) {
                        Some(p) => p,
                        None => { names.push(ident.clone()); names.len() - 1 }
                    };
                    out.push('$');
                    out.push_str(&(position + 1).to_string());
                    i = end;
                } else {
                    out.push(c);
                    i += 1;
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }

    let values = names.iter().map(|name| {
        params.get(name).cloned().ok_or_else(|| anyhow!("missing query parameter ${name}"))
    }).collect::<Result<Vec<_>>>()?;
    Ok((out, values))
}

fn skip_quoted(chars: &[char], start: usize, quote: char) -> usize {
    let mut i = start + 1;
    while i < chars.len() {
        if chars[i] == quote {
            if chars.get(i + 1) == Some(&quote) { i += 2; continue; }
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

fn find_seq(chars: &[char], from: usize, seq: &[char]) -> Option<usize> {
    (from..chars.len()).find(|&p| chars[p..].starts_with(seq))
}

/// A JSON value bound as whatever type Postgres inferred for the placeholder.
#[derive(Debug)]
pub struct JsonParam<'a>(pub &'a Value);

impl ToSql for JsonParam<'_> {
    fn to_sql(&self, ty: &Type, out: &mut BytesMut) -> Result<IsNull, BoxError> {
        let v = self.0;
        if v.is_null() {
            return Ok(IsNull::Yes);
        }
        match *ty {
            Type::BOOL => as_bool(v)?.to_sql(ty, out),
            Type::INT2 => i16::try_from(as_i64(v)?)?.to_sql(ty, out),
            Type::INT4 => i32::try_from(as_i64(v)?)?.to_sql(ty, out),
            Type::INT8 => as_i64(v)?.to_sql(ty, out),
            Type::FLOAT4 => (as_f64(v)? as f32).to_sql(ty, out),
            Type::FLOAT8 => as_f64(v)?.to_sql(ty, out),
            Type::TIMESTAMPTZ => as_timestamp(v)?.to_sql(ty, out),
            Type::JSON | Type::JSONB => v.to_sql(ty, out),
            Type::TEXT_ARRAY | Type::VARCHAR_ARRAY => as_text_vec(v)?.to_sql(ty, out),
            Type::INT8_ARRAY => as_array(v)?.iter().map(as_i64).collect::<Result<Vec<_>, _>>()?.to_sql(ty, out),
            Type::INT4_ARRAY => as_array(v)?.iter()
                .map(|x| as_i64(x).and_then(|n| Ok(i32::try_from(n)?)))
                .collect::<Result<Vec<_>, _>>()?.to_sql(ty, out),
            Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => as_text(v).to_sql(ty, out),
            ref other => Err(format!("unsupported parameter type {other}").into()),
        }
    }

    fn accepts(_: &Type) -> bool {
        true
    }

    to_sql_checked!();
}

fn as_bool(v: &Value) -> Result<bool, BoxError> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::String(s) => Ok(s.parse()?),
        other => Err(format!("expected a boolean, got {other}").into()),
    }
}

fn as_i64(v: &Value) -> Result<i64, BoxError> {
    match v {
        Value::Number(n) => n.as_i64()
            .or_else(|| n.as_u64().and_then(|u| i64::try_from(u).ok()))
            .or_else(|| n.as_f64().map(|f| f as i64))
            .ok_or_else(|| format!("number {n} does not fit in an integer").into()),
        Value::String(s) => Ok(s.parse()?),
        Value::Bool(b) => Ok(*b as i64),
        other => Err(format!("expected an integer, got {other}").into()),
    }
}

fn as_f64(v: &Value) -> Result<f64, BoxError> {
    match v {
        Value::Number(n) => n.as_f64().ok_or_else(|| format!("invalid number {n}").into()),
        Value::String(s) => Ok(s.parse()?),
        other => Err(format!("expected a number, got {other}").into()),
    }
}

fn as_timestamp(v: &Value) -> Result<DateTime<Utc>, BoxError> {
    match v {
        Value::String(s) => Ok(DateTime::<FixedOffset>::parse_from_rfc3339(s)
            .map_err(|e| format!("invalid RFC 3339 timestamp {s:?}: {e}"))?
            .with_timezone(&Utc)),
        other => Err(format!("expected an RFC 3339 timestamp, got {other}").into()),
    }
}

fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn as_array(v: &Value) -> Result<&Vec<Value>, BoxError> {
    v.as_array().ok_or_else(|| format!("expected an array, got {v}").into())
}

fn as_text_vec(v: &Value) -> Result<Vec<String>, BoxError> {
    Ok(as_array(v)?.iter().map(as_text).collect())
}

#[cfg(test)]
mod tests {
    use super::bind_named;
    use serde_json::json;

    #[test]
    fn rewrites_named_placeholders_in_order_of_first_use() {
        let (sql, values) = bind_named(
            "SELECT * FROM t WHERE a = $pid AND b = $did OR c = $pid",
            &json!({ "pid": "p", "did": "d", "unused": 1 }),
        ).unwrap();
        assert_eq!(sql, "SELECT * FROM t WHERE a = $1 AND b = $2 OR c = $1");
        assert_eq!(values, vec![json!("p"), json!("d")]);
    }

    #[test]
    fn leaves_literals_comments_casts_and_dollar_quotes_alone() {
        let (sql, values) = bind_named(
            "SELECT '$x', \"$y\", $z::text -- $c\n, $$ $q $$, $t$ $w $t$ /* $b */",
            &json!({ "z": 1 }),
        ).unwrap();
        assert_eq!(sql, "SELECT '$x', \"$y\", $1::text -- $c\n, $$ $q $$, $t$ $w $t$ /* $b */");
        assert_eq!(values, vec![json!(1)]);
    }

    #[test]
    fn keeps_positional_placeholders() {
        let (sql, values) = bind_named("SELECT $1", &json!({})).unwrap();
        assert_eq!(sql, "SELECT $1");
        assert!(values.is_empty());
    }

    #[test]
    fn missing_parameter_is_an_error() {
        assert!(bind_named("SELECT $nope", &json!({})).is_err());
    }
}
