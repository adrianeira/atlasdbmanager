//! Valores do MySQL como texto, no formato que a grade espera. O protocolo de texto já entrega quase tudo como texto; sobram o
//! binário (hexa `\x...`) e o `BIT` (`t`/`f` quando tem um bit).

use mysql_async::consts::ColumnType;
use mysql_async::{Column, Row, Value};

/// O texto de um binário: `\x` e os bytes em hexa.
fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push_str("\\x");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// A coluna guarda bytes (`BLOB`, `BINARY`, `VARBINARY`), não texto: o conjunto de caracteres é o "binary" (63).
fn is_binary(c: &Column) -> bool {
    c.character_set() == 63 && matches!(c.column_type(), ColumnType::MYSQL_TYPE_BLOB | ColumnType::MYSQL_TYPE_TINY_BLOB | ColumnType::MYSQL_TYPE_MEDIUM_BLOB | ColumnType::MYSQL_TYPE_LONG_BLOB | ColumnType::MYSQL_TYPE_VAR_STRING | ColumnType::MYSQL_TYPE_STRING | ColumnType::MYSQL_TYPE_VARCHAR)
}

/// O nome do tipo no MySQL para uma coluna de resultado (a grade decide número, texto, data... por ele).
pub(crate) fn type_name(c: &Column) -> String {
    use ColumnType::*;
    let binary = c.character_set() == 63;
    match c.column_type() {
        MYSQL_TYPE_TINY => "tinyint",
        MYSQL_TYPE_SHORT => "smallint",
        MYSQL_TYPE_INT24 => "mediumint",
        MYSQL_TYPE_LONG => "int",
        MYSQL_TYPE_LONGLONG => "bigint",
        MYSQL_TYPE_FLOAT => "float",
        MYSQL_TYPE_DOUBLE => "double",
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL => "decimal",
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE => "date",
        MYSQL_TYPE_DATETIME | MYSQL_TYPE_DATETIME2 => "datetime",
        MYSQL_TYPE_TIMESTAMP | MYSQL_TYPE_TIMESTAMP2 => "timestamp",
        MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2 => "time",
        MYSQL_TYPE_YEAR => "year",
        MYSQL_TYPE_BIT => "bit",
        MYSQL_TYPE_JSON => "json",
        MYSQL_TYPE_ENUM => "enum",
        MYSQL_TYPE_SET => "set",
        MYSQL_TYPE_GEOMETRY => "geometry",
        MYSQL_TYPE_TINY_BLOB | MYSQL_TYPE_MEDIUM_BLOB | MYSQL_TYPE_LONG_BLOB | MYSQL_TYPE_BLOB => if binary { "blob" } else { "text" },
        MYSQL_TYPE_VAR_STRING | MYSQL_TYPE_VARCHAR => if binary { "varbinary" } else { "varchar" },
        MYSQL_TYPE_STRING => if binary { "binary" } else { "char" },
        MYSQL_TYPE_NULL => "null",
        _ => "text",
    }
    .to_string()
}

fn value_text(v: &Value, c: Option<&Column>) -> Option<String> {
    match v {
        Value::NULL => None,
        Value::Bytes(b) => {
            if let Some(col) = c {
                if col.column_type() == ColumnType::MYSQL_TYPE_BIT {
                    // `BIT(1)` é o booleano do MySQL; mais bits viram o número.
                    let n = b.iter().fold(0u64, |acc, x| (acc << 8) | u64::from(*x));
                    return Some(if col.column_length() == 1 { if n == 0 { "f".into() } else { "t".into() } } else { n.to_string() });
                }
                if is_binary(col) {
                    return Some(hex(b));
                }
            }
            Some(String::from_utf8_lossy(b).into_owned())
        }
        Value::Int(i) => Some(i.to_string()),
        Value::UInt(u) => Some(u.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Double(d) => Some(d.to_string()),
        Value::Date(y, mo, d, h, mi, s, us) => {
            let date = format!("{y:04}-{mo:02}-{d:02}");
            Some(if *h == 0 && *mi == 0 && *s == 0 && *us == 0 { date } else if *us == 0 { format!("{date} {h:02}:{mi:02}:{s:02}") } else { format!("{date} {h:02}:{mi:02}:{s:02}.{us:06}") })
        }
        Value::Time(neg, days, h, mi, s, us) => {
            let hours = *days * 24 + u32::from(*h);
            let base = format!("{}{hours:02}:{mi:02}:{s:02}", if *neg { "-" } else { "" });
            Some(if *us == 0 { base } else { format!("{base}.{us:06}") })
        }
    }
}

pub(crate) fn row_text(row: &Row, cols: &[Column]) -> Vec<Option<String>> {
    (0..row.len()).map(|i| row.as_ref(i).and_then(|v| value_text(v, cols.get(i)))).collect()
}
