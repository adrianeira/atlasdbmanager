//! Valores e tipos das colunas de resultado. O cliente (uma cópia ajustada em `vendor/`) guarda em `Column::raw_type` o tipo que o
//! servidor descreveu: bits 0-15 o código do tipo, 16-23 a escala (com sinal) e 24-31 o subtipo do blob.

use chrono::{Datelike, NaiveDateTime, Timelike};
use rsfbclient::{Column, SqlType};

const SQL_VARYING: u32 = 448;
const SQL_TEXT: u32 = 452;
const SQL_DOUBLE: u32 = 480;
const SQL_FLOAT: u32 = 482;
const SQL_LONG: u32 = 496;
const SQL_SHORT: u32 = 500;
const SQL_TIMESTAMP: u32 = 510;
const SQL_BLOB: u32 = 520;
const SQL_TYPE_TIME: u32 = 560;
const SQL_TYPE_DATE: u32 = 570;
const SQL_INT64: u32 = 580;
const SQL_INT128: u32 = 32752;
const SQL_BOOLEAN: u32 = 32764;

fn code(raw: u32) -> u32 {
    (raw & 0xFFFF) & !1
}

fn scale(raw: u32) -> i32 {
    i32::from(((raw >> 16) & 0xFF) as u8 as i8)
}

/// Inteiro com escala (`NUMERIC(10,2)` guarda 1050 com escala -2) como o decimal exato "10.50".
fn scaled(v: i128, scale: i32) -> String {
    if scale >= 0 {
        return if scale == 0 { v.to_string() } else { format!("{v}{}", "0".repeat(scale as usize)) };
    }
    let places = (-scale) as usize;
    let digits = v.unsigned_abs().to_string();
    let padded = if digits.len() <= places { format!("{}{digits}", "0".repeat(places + 1 - digits.len())) } else { digits };
    let (int, frac) = padded.split_at(padded.len() - places);
    format!("{}{int}.{frac}", if v < 0 { "-" } else { "" })
}

fn date(ts: &NaiveDateTime) -> String {
    format!("{:04}-{:02}-{:02}", ts.year(), ts.month(), ts.day())
}

/// A hora com décimos de milésimo (a precisão do Firebird) só quando há fração.
fn time(ts: &NaiveDateTime) -> String {
    let frac = ts.nanosecond() / 100_000;
    let base = format!("{:02}:{:02}:{:02}", ts.hour(), ts.minute(), ts.second());
    if frac == 0 { base } else { format!("{base}.{frac:04}") }
}

pub(crate) fn format(c: &Column) -> Option<String> {
    let raw = c.raw_type;
    match &c.value {
        SqlType::Null => None,
        SqlType::Integer(i) => Some(scaled(i128::from(*i), scale(raw))),
        SqlType::Int128(i) => Some(scaled(*i, scale(raw))),
        SqlType::Floating(f) => Some(if code(raw) == SQL_FLOAT { (*f as f32).to_string() } else { f.to_string() }),
        SqlType::Text(t) => Some(t.clone()),
        SqlType::Boolean(b) => Some(if *b { "t".into() } else { "f".into() }),
        SqlType::Binary(b) => {
            let mut s = String::with_capacity(2 + b.len() * 2);
            s.push_str("\\x");
            for byte in b {
                s.push_str(&format!("{byte:02x}"));
            }
            Some(s)
        }
        SqlType::Timestamp(ts) => Some(match code(raw) {
            SQL_TYPE_DATE => date(ts),
            SQL_TYPE_TIME => time(ts),
            _ => format!("{} {}", date(ts), time(ts)),
        }),
    }
}

/// O nome do tipo no Firebird para uma coluna de resultado (a grade decide número, texto, data... por ele).
pub fn type_name(raw: u32) -> String {
    let numeric = scale(raw) != 0;
    match code(raw) {
        SQL_VARYING => "varchar",
        SQL_TEXT => "char",
        SQL_SHORT if numeric => "numeric",
        SQL_LONG if numeric => "numeric",
        SQL_INT64 if numeric => "numeric",
        SQL_INT128 if numeric => "numeric",
        SQL_SHORT => "smallint",
        SQL_LONG => "integer",
        SQL_INT64 => "bigint",
        SQL_INT128 => "int128",
        SQL_FLOAT => "float",
        SQL_DOUBLE => "double precision",
        SQL_TIMESTAMP => "timestamp",
        SQL_TYPE_DATE => "date",
        SQL_TYPE_TIME => "time",
        SQL_BOOLEAN => "boolean",
        SQL_BLOB => {
            if (raw >> 24) as u8 == 1 { "blob sub_type text" } else { "blob" }
        }
        _ => "text",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_values() {
        assert_eq!(scaled(1050, -2), "10.50");
        assert_eq!(scaled(5, -2), "0.05");
        assert_eq!(scaled(-5, -2), "-0.05");
        assert_eq!(scaled(-12345, -2), "-123.45");
        assert_eq!(scaled(7, 0), "7");
    }
}
