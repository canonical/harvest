use anyhow::{bail, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{json, Map, Value};
use tokio_postgres::types::Type;
use tokio_postgres::Row;

pub fn row_to_json(row: &Row) -> Result<Value> {
    let mut map = Map::with_capacity(row.len());
    for (i, column) in row.columns().iter().enumerate() {
        map.insert(column.name().to_string(), column_to_json(row, i, column.type_(), column.name())?);
    }
    Ok(Value::Object(map))
}

fn column_to_json(row: &Row, i: usize, ty: &Type, name: &str) -> Result<Value> {
    Ok(match *ty {
        Type::BOOL => json!(row.try_get::<_, Option<bool>>(i)?),
        Type::INT2 => json!(row.try_get::<_, Option<i16>>(i)?),
        Type::INT4 => json!(row.try_get::<_, Option<i32>>(i)?),
        Type::INT8 => json!(row.try_get::<_, Option<i64>>(i)?),
        Type::OID => json!(row.try_get::<_, Option<u32>>(i)?),
        Type::FLOAT4 => json!(row.try_get::<_, Option<f32>>(i)?),
        Type::FLOAT8 => json!(row.try_get::<_, Option<f64>>(i)?),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN => {
            json!(row.try_get::<_, Option<String>>(i)?)
        }
        Type::TIMESTAMPTZ => json!(row.try_get::<_, Option<DateTime<Utc>>>(i)?.map(|t| t.to_rfc3339())),
        Type::TIMESTAMP => json!(row.try_get::<_, Option<NaiveDateTime>>(i)?.map(|t| t.to_string())),
        Type::JSON | Type::JSONB => row.try_get::<_, Option<Value>>(i)?.unwrap_or(Value::Null),
        Type::TEXT_ARRAY | Type::VARCHAR_ARRAY | Type::NAME_ARRAY => {
            json!(row.try_get::<_, Option<Vec<Option<String>>>>(i)?)
        }
        Type::INT4_ARRAY => json!(row.try_get::<_, Option<Vec<Option<i32>>>>(i)?),
        Type::INT8_ARRAY => json!(row.try_get::<_, Option<Vec<Option<i64>>>>(i)?),
        Type::BOOL_ARRAY => json!(row.try_get::<_, Option<Vec<Option<bool>>>>(i)?),
        Type::VOID => Value::Null,
        ref other => bail!("column {name} has unsupported type {other}; cast it in the query"),
    })
}
