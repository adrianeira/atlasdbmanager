//! Valores do SQL Server como texto, no formato que a grade espera: bit como `t`/`f`, binário em hexa (`\x...`), datas no formato do servidor.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use tiberius::{ColumnData, ColumnType, FromSql, Row};

/// O nome do tipo no SQL Server para um tipo de coluna do protocolo (a grade decide número, texto, data... por ele).
pub(crate) fn type_name(t: ColumnType) -> String {
    let raw = format!("{t:?}").to_ascii_lowercase();
    match raw.as_str() {
        "int1" => "tinyint",
        "int2" => "smallint",
        "int4" | "intn" => "int",
        "int8" => "bigint",
        "bit" | "bitn" => "bit",
        "float4" => "real",
        "float8" | "floatn" => "float",
        "decimaln" | "numericn" => "decimal",
        "money4" => "smallmoney",
        "datetime4" => "smalldatetime",
        "datetimen" => "datetime",
        "daten" => "date",
        "timen" => "time",
        "datetimeoffsetn" => "datetimeoffset",
        "guid" => "uniqueidentifier",
        "bigvarchar" => "varchar",
        "bigchar" => "char",
        "bigvarbin" => "varbinary",
        "bigbinary" => "binary",
        "ssvariant" => "sql_variant",
        other => other,
    }
    .to_string()
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push_str("\\x");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub(crate) fn value_text(d: &ColumnData<'static>) -> Option<String> {
    match d {
        ColumnData::U8(v) => v.map(|x| x.to_string()),
        ColumnData::I16(v) => v.map(|x| x.to_string()),
        ColumnData::I32(v) => v.map(|x| x.to_string()),
        ColumnData::I64(v) => v.map(|x| x.to_string()),
        ColumnData::F32(v) => v.map(|x| x.to_string()),
        ColumnData::F64(v) => v.map(|x| x.to_string()),
        ColumnData::Bit(v) => v.map(|b| if b { "t" } else { "f" }.to_string()),
        ColumnData::String(v) => v.as_ref().map(|s| s.to_string()),
        ColumnData::Guid(v) => v.map(|g| g.to_string().to_uppercase()),
        ColumnData::Binary(v) => v.as_ref().map(|b| hex(b)),
        ColumnData::Numeric(v) => v.map(|n| n.to_string()),
        ColumnData::Xml(v) => v.as_ref().map(|x| x.to_string()),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            NaiveDateTime::from_sql(d).ok().flatten().map(|v| v.format("%Y-%m-%d %H:%M:%S%.f").to_string())
        }
        ColumnData::Date(_) => NaiveDate::from_sql(d).ok().flatten().map(|v| v.format("%Y-%m-%d").to_string()),
        ColumnData::Time(_) => NaiveTime::from_sql(d).ok().flatten().map(|v| v.format("%H:%M:%S%.f").to_string()),
        ColumnData::DateTimeOffset(_) => DateTime::<FixedOffset>::from_sql(d).ok().flatten().map(|v| v.format("%Y-%m-%d %H:%M:%S%.f %:z").to_string()),
    }
}

pub(crate) fn row_text(row: &Row) -> Vec<Option<String>> {
    row.cells().map(|(_, d)| value_text(d)).collect()
}
