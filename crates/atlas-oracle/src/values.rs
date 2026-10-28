//! Valores do Oracle como texto, no formato que a grade espera: números sem perda (o `NUMBER` volta como o servidor o escreve), datas
//! em `AAAA-MM-DD HH:MM:SS`, binários em hexa (`\x...`), `CLOB` como texto e booleano como `t`/`f`.

use oracle_rs::{ColumnInfo, Connection, OracleType, Value};
use oracle_rs::types::LobValue;

/// Quanto de um LOB vai para a grade (o resto fica no servidor): 1 milhão de caracteres, 64 KB de binário.
const CLOB_CAP: usize = 1_000_000;
const BLOB_CAP: usize = 65_536;

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push_str("\\x");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// O nome do tipo de uma coluna de resultado (a grade decide número, texto, data... por ele).
pub(crate) fn type_name(c: &ColumnInfo) -> String {
    use OracleType::*;
    match c.oracle_type {
        Varchar => "varchar2".into(),
        Number => "number".into(),
        BinaryInteger => "binary_integer".into(),
        Long => "long".into(),
        Rowid => "rowid".into(),
        Date => "date".into(),
        Raw => "raw".into(),
        LongRaw => "long raw".into(),
        Char => "char".into(),
        BinaryFloat => "binary_float".into(),
        BinaryDouble => "binary_double".into(),
        Clob => if c.csfrm == 2 { "nclob" } else { "clob" }.into(),
        Blob => "blob".into(),
        Bfile => "bfile".into(),
        Json => "json".into(),
        Timestamp => "timestamp".into(),
        TimestampTz => "timestamp with time zone".into(),
        IntervalYm => "interval year to month".into(),
        IntervalDs => "interval day to second".into(),
        other => format!("{other:?}").to_lowercase(),
    }
}

fn plain(v: &Value) -> Option<String> {
    Some(match v {
        Value::Null => return None,
        Value::String(s) => s.clone(),
        Value::Bytes(b) => hex(b),
        Value::Integer(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Number(n) => n.as_str().to_string(),
        Value::Date(d) => format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", d.year, d.month, d.day, d.hour, d.minute, d.second),
        Value::Timestamp(t) => {
            let mut s = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second);
            if t.microsecond != 0 {
                s.push_str(format!(".{:06}", t.microsecond).trim_end_matches('0'));
            }
            if t.has_timezone() {
                let sign = if t.tz_hour_offset < 0 || t.tz_minute_offset < 0 { '-' } else { '+' };
                s.push_str(&format!(" {sign}{:02}:{:02}", t.tz_hour_offset.abs(), t.tz_minute_offset.abs()));
            }
            s
        }
        Value::RowId(r) => format!("{r}"),
        Value::Boolean(b) => if *b { "t" } else { "f" }.into(),
        Value::Json(j) => j.to_string(),
        other => other.to_string(),
    })
}

fn cap(mut s: String, max: usize) -> String {
    if let Some((i, _)) = s.char_indices().nth(max) {
        s.truncate(i);
    }
    s
}

/// O texto de uma célula; LOBs grandes são lidos do servidor (por isso é assíncrono).
pub(crate) async fn cell(conn: &Connection, col: &ColumnInfo, v: &Value) -> Option<String> {
    let text_lob = matches!(col.oracle_type, OracleType::Clob);
    match v {
        Value::Lob(LobValue::Null) => None,
        Value::Lob(LobValue::Empty) => Some(String::new()),
        Value::Lob(LobValue::Inline(data)) => Some(if text_lob { cap(String::from_utf8_lossy(data).into_owned(), CLOB_CAP) } else { hex(&data[..data.len().min(BLOB_CAP)]) }),
        Value::Lob(LobValue::Locator(loc)) => {
            if text_lob {
                conn.read_clob(loc).await.ok().map(|s| cap(s, CLOB_CAP))
            } else {
                conn.read_blob(loc).await.ok().map(|b| hex(&b[..b.len().min(BLOB_CAP)]))
            }
        }
        Value::String(s) if text_lob => Some(cap(s.clone(), CLOB_CAP)),
        other => plain(other),
    }
}
