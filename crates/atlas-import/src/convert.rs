//! Conversão do texto lido do arquivo para o que a coluna de destino entende (números, booleanos, datas e horas).
//! O servidor recebe sempre texto e faz o `cast` final; aqui só se normaliza o que o formato do arquivo tem de próprio.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum DateOrder {
    #[serde(rename = "DMY")]
    Dmy,
    #[serde(rename = "MDY")]
    Mdy,
    #[serde(rename = "YMD")]
    Ymd,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum DateTimeOrder {
    #[serde(rename = "DateTime")]
    DateTime,
    #[serde(rename = "TimeDate")]
    TimeDate,
}

/// Formatos do arquivo de origem (a etapa "Opções da origem" do assistente).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConvertOptions {
    pub date_order: DateOrder,
    pub date_time_order: DateTimeOrder,
    pub date_delimiter: String,
    pub time_delimiter: String,
    /// Símbolo decimal dos números (`.` ou `,`).
    pub decimal_symbol: String,
    /// Texto vazio vira NULL também em colunas de texto (nas demais colunas o vazio já é NULL).
    pub empty_as_null: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            date_order: DateOrder::Dmy,
            date_time_order: DateTimeOrder::DateTime,
            date_delimiter: "/".into(),
            time_delimiter: ":".into(),
            decimal_symbol: ".".into(),
            empty_as_null: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Int,
    Numeric,
    Bool,
    Date,
    Time,
    Timestamp,
    Text,
}

/// Categoria do tipo da coluna a partir do nome do tipo (`format_type`: `integer`, `numeric(19,4)`, `timestamp with time zone`...).
pub fn kind_of(type_name: &str) -> TargetKind {
    let t = type_name.trim().to_lowercase();
    let base = t.split('(').next().unwrap_or(&t).trim();
    if t.ends_with("[]") {
        return TargetKind::Text;
    }
    match base {
        "smallint" | "integer" | "bigint" | "int" | "int2" | "int4" | "int8" | "serial" | "bigserial" | "smallserial" => TargetKind::Int,
        "numeric" | "decimal" | "real" | "double precision" | "float" | "float4" | "float8" | "money" => TargetKind::Numeric,
        "boolean" | "bool" => TargetKind::Bool,
        "date" => TargetKind::Date,
        "time" | "time without time zone" | "time with time zone" | "timetz" => TargetKind::Time,
        _ if base.starts_with("timestamp") => TargetKind::Timestamp,
        _ => TargetKind::Text,
    }
}

const MONTHS: [(&str, u32); 22] = [
    ("jan", 1), ("fev", 2), ("feb", 2), ("mar", 3), ("abr", 4), ("apr", 4), ("mai", 5), ("may", 5), ("jun", 6), ("jul", 7), ("ago", 8),
    ("aug", 8), ("set", 9), ("sep", 9), ("out", 10), ("oct", 10), ("nov", 11), ("dez", 12), ("dec", 12), ("sept", 9), ("agosto", 8), ("marco", 3),
];

fn month_number(token: &str) -> Option<u32> {
    if let Ok(n) = token.parse::<u32>() {
        return (1..=12).contains(&n).then_some(n);
    }
    let lower = token.to_lowercase();
    let key: String = lower.chars().take(3).collect();
    MONTHS.iter().find(|(k, _)| *k == key.as_str()).map(|(_, n)| *n)
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        2 if (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn is_iso_date_prefix(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10 && b[..4].iter().all(u8::is_ascii_digit) && b[4] == b'-' && b[5..7].iter().all(u8::is_ascii_digit) && b[7] == b'-' && b[8..10].iter().all(u8::is_ascii_digit)
}

/// Data no formato do arquivo → `AAAA-MM-DD`.
fn parse_date(s: &str, o: &ConvertOptions) -> Result<String, String> {
    let bad = || format!("data inválida: \"{s}\"");
    let delimiters: Vec<&str> = std::iter::once(o.date_delimiter.as_str()).chain(["/", "-", ".", " "]).filter(|d| !d.is_empty()).collect();
    let tokens: Vec<&str> = delimiters
        .iter()
        .map(|d| s.split(d).map(str::trim).filter(|t| !t.is_empty()).collect::<Vec<_>>())
        .find(|t| t.len() == 3)
        .ok_or_else(bad)?;
    let (d, m, y) = match o.date_order {
        DateOrder::Dmy => (tokens[0], tokens[1], tokens[2]),
        DateOrder::Mdy => (tokens[1], tokens[0], tokens[2]),
        DateOrder::Ymd => (tokens[2], tokens[1], tokens[0]),
    };
    let day: u32 = d.parse().map_err(|_| bad())?;
    let month = month_number(m).ok_or_else(bad)?;
    let mut year: u32 = y.parse().map_err(|_| bad())?;
    if y.len() <= 2 {
        year += if year < 70 { 2000 } else { 1900 };
    }
    if day == 0 || day > days_in_month(year, month) || !(1..=9999).contains(&year) {
        return Err(bad());
    }
    Ok(format!("{year:04}-{month:02}-{day:02}"))
}

/// Hora (`H:M`, `H:M:S`, `H:M:S.fração`, com AM/PM opcional) → `HH:MM:SS[.fração]`.
fn parse_time(s: &str, o: &ConvertOptions) -> Result<String, String> {
    let bad = || format!("hora inválida: \"{s}\"");
    let mut text = s.trim().to_string();
    let lower = text.to_lowercase();
    let pm = lower.ends_with("pm");
    let am = lower.ends_with("am");
    if am || pm {
        text = text[..text.len() - 2].trim().to_string();
    }
    let d = if o.time_delimiter.is_empty() { ":" } else { o.time_delimiter.as_str() };
    let tokens: Vec<&str> = text.split(d).map(str::trim).collect();
    if !(2..=3).contains(&tokens.len()) {
        return Err(bad());
    }
    let mut hour: u32 = tokens[0].parse().map_err(|_| bad())?;
    let minute: u32 = tokens[1].parse().map_err(|_| bad())?;
    let (sec_text, frac) = match tokens.get(2) {
        Some(t) => match t.split_once(['.', ',']) {
            Some((s, f)) if f.bytes().all(|b| b.is_ascii_digit()) && !f.is_empty() => (s, Some(f)),
            Some(_) => return Err(bad()),
            None => (*t, None),
        },
        None => ("0", None),
    };
    let second: u32 = sec_text.parse().map_err(|_| bad())?;
    if am || pm {
        if !(1..=12).contains(&hour) {
            return Err(bad());
        }
        hour = hour % 12 + if pm { 12 } else { 0 };
    }
    if hour > 23 || minute > 59 || second > 59 {
        return Err(bad());
    }
    Ok(match frac {
        Some(f) => format!("{hour:02}:{minute:02}:{second:02}.{f}"),
        None => format!("{hour:02}:{minute:02}:{second:02}"),
    })
}

/// Separa o texto em (data, hora) conforme a ordem; `T` entre as partes também vale.
fn split_parts<'a>(s: &'a str, o: &ConvertOptions) -> (Option<&'a str>, Option<&'a str>) {
    // O primeiro espaço (ou `T`) divide as duas partes; um AM/PM no fim fica com a hora.
    let (first, second) = match s.split_once(|c: char| c.is_whitespace() || c == 'T') {
        Some((a, b)) if !b.trim().is_empty() => (a.trim(), Some(b.trim())),
        _ => (s.trim(), None),
    };
    match (o.date_time_order, second) {
        (DateTimeOrder::DateTime, Some(t)) => (Some(first), Some(t)),
        (DateTimeOrder::DateTime, None) => (Some(first), None),
        (DateTimeOrder::TimeDate, Some(d)) => (Some(d), Some(first)),
        (DateTimeOrder::TimeDate, None) => (Some(first), None),
    }
}

const TRUE_WORDS: [&str; 10] = ["true", "t", "yes", "y", "s", "sim", "1", "verdadeiro", "v", "on"];
const FALSE_WORDS: [&str; 10] = ["false", "f", "no", "n", "nao", "não", "0", "falso", "off", "-"];

/// `true`/`false` a partir das formas comuns (`t`, `sim`, `não`, `1`...).
pub fn parse_bool(s: &str) -> Option<bool> {
    let l = s.trim().to_lowercase();
    if TRUE_WORDS.contains(&l.as_str()) {
        Some(true)
    } else if FALSE_WORDS.contains(&l.as_str()) {
        Some(false)
    } else {
        None
    }
}

/// Número com o símbolo decimal do arquivo → texto numérico padrão (`1234.5`).
fn normalize_number(s: &str, o: &ConvertOptions) -> Result<String, String> {
    let bad = || format!("número inválido: \"{s}\"");
    let mut t: String = s.chars().filter(|c| !c.is_whitespace() && *c != '\u{a0}').collect();
    if o.decimal_symbol == "," {
        t = t.replace('.', "").replace(',', ".");
    } else {
        t = t.replace(',', "");
    }
    if t.is_empty() || !t.chars().all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E')) || t.parse::<f64>().is_err() {
        return Err(bad());
    }
    Ok(t.strip_prefix('+').unwrap_or(&t).to_string())
}

/// Converte um valor lido (`value`) para a coluna. `Ok(None)` = NULL; `Err` = o valor não serve para o tipo.
pub fn convert(value: &str, kind: TargetKind, o: &ConvertOptions) -> Result<Option<String>, String> {
    if kind == TargetKind::Text {
        return Ok(if value.is_empty() && o.empty_as_null { None } else { Some(value.to_string()) });
    }
    let v = value.trim();
    if v.is_empty() {
        return Ok(None);
    }
    match kind {
        TargetKind::Text => unreachable!(),
        TargetKind::Bool => parse_bool(v).map(|b| Some(if b { "true" } else { "false" }.to_string())).ok_or_else(|| format!("valor lógico inválido: \"{v}\"")),
        TargetKind::Numeric => normalize_number(v, o).map(Some),
        TargetKind::Int => {
            let n = normalize_number(v, o)?;
            match n.split_once('.') {
                Some((int, frac)) if frac.bytes().all(|b| b == b'0') && !n.contains(['e', 'E']) => Ok(Some(int.to_string())),
                Some(_) => Err(format!("número inteiro inválido: \"{v}\"")),
                None if n.contains(['e', 'E']) => Err(format!("número inteiro inválido: \"{v}\"")),
                None => Ok(Some(n)),
            }
        }
        TargetKind::Date => {
            if is_iso_date_prefix(v) {
                return Ok(Some(v[..10].to_string()));
            }
            let (date, _) = split_parts(v, o);
            parse_date(date.unwrap_or(v), o).map(Some)
        }
        TargetKind::Time => {
            let (date, time) = split_parts(v, o);
            // Só hora: a única parte é a hora; data e hora: fica a hora.
            let t = if is_iso_date_prefix(v) || time.is_some() { time.or(date) } else { Some(v) };
            parse_time(t.unwrap_or(v), o).map(Some)
        }
        TargetKind::Timestamp => {
            if is_iso_date_prefix(v) {
                // Já é ISO (pode trazer fuso): o servidor entende.
                return Ok(Some(v.to_string()));
            }
            let (date, time) = split_parts(v, o);
            let d = parse_date(date.unwrap_or(v), o)?;
            match time {
                Some(t) => Ok(Some(format!("{d} {}", parse_time(t, o)?))),
                None => Ok(Some(d)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(v: &str, kind: TargetKind) -> Result<Option<String>, String> {
        convert(v, kind, &ConvertOptions::default())
    }

    #[test]
    fn kinds_from_postgres_type_names() {
        for (name, kind) in [
            ("integer", TargetKind::Int), ("bigint", TargetKind::Int), ("numeric(19,4)", TargetKind::Numeric), ("double precision", TargetKind::Numeric),
            ("boolean", TargetKind::Bool), ("date", TargetKind::Date), ("timestamp with time zone", TargetKind::Timestamp),
            ("timestamp(3) without time zone", TargetKind::Timestamp), ("time without time zone", TargetKind::Time), ("interval", TargetKind::Text),
            ("daterange", TargetKind::Text), ("character varying(150)", TargetKind::Text), ("uuid", TargetKind::Text), ("integer[]", TargetKind::Text),
            ("jsonb", TargetKind::Text),
        ] {
            assert_eq!(kind_of(name), kind, "{name}");
        }
    }

    #[test]
    fn numbers_respect_the_decimal_symbol() {
        assert_eq!(conv("1,234.50", TargetKind::Numeric).unwrap().as_deref(), Some("1234.50"));
        let br = ConvertOptions { decimal_symbol: ",".into(), ..Default::default() };
        assert_eq!(convert("1.234,50", TargetKind::Numeric, &br).unwrap().as_deref(), Some("1234.50"));
        assert_eq!(convert("-0,5", TargetKind::Numeric, &br).unwrap().as_deref(), Some("-0.5"));
        assert_eq!(conv("1e3", TargetKind::Numeric).unwrap().as_deref(), Some("1e3"));
        assert!(conv("abc", TargetKind::Numeric).is_err());
        assert!(conv("12x", TargetKind::Numeric).is_err());
        assert_eq!(conv("  ", TargetKind::Numeric).unwrap(), None);
    }

    #[test]
    fn integers_accept_trailing_zero_fractions_only() {
        assert_eq!(conv("42", TargetKind::Int).unwrap().as_deref(), Some("42"));
        assert_eq!(conv("42.0", TargetKind::Int).unwrap().as_deref(), Some("42"));
        assert_eq!(conv("+7", TargetKind::Int).unwrap().as_deref(), Some("7"));
        assert!(conv("42.5", TargetKind::Int).is_err());
        assert!(conv("1e3", TargetKind::Int).is_err());
    }

    #[test]
    fn booleans_in_portuguese_and_english() {
        for (v, want) in [("t", true), ("TRUE", true), ("Sim", true), ("1", true), ("f", false), ("Não", false), ("nao", false), ("0", false)] {
            assert_eq!(conv(v, TargetKind::Bool).unwrap().as_deref(), Some(if want { "true" } else { "false" }), "{v}");
        }
        assert!(conv("talvez", TargetKind::Bool).is_err());
    }

    #[test]
    fn dates_follow_order_delimiter_and_month_names() {
        assert_eq!(conv("22/10/26", TargetKind::Date).unwrap().as_deref(), Some("2026-10-22"));
        assert_eq!(conv("22/10/2026", TargetKind::Date).unwrap().as_deref(), Some("2026-10-22"));
        assert_eq!(conv("22/Oct/26", TargetKind::Date).unwrap().as_deref(), Some("2026-10-22"));
        assert_eq!(conv("22/Outubro/2026", TargetKind::Date).unwrap().as_deref(), Some("2026-10-22"));
        assert_eq!(conv("01/02/99", TargetKind::Date).unwrap().as_deref(), Some("1999-02-01"));
        let mdy = ConvertOptions { date_order: DateOrder::Mdy, date_delimiter: "-".into(), ..Default::default() };
        assert_eq!(convert("10-22-2026", TargetKind::Date, &mdy).unwrap().as_deref(), Some("2026-10-22"));
        let ymd = ConvertOptions { date_order: DateOrder::Ymd, date_delimiter: ".".into(), ..Default::default() };
        assert_eq!(convert("2026.10.22", TargetKind::Date, &ymd).unwrap().as_deref(), Some("2026-10-22"));
        // ISO passa direto, e um carimbo de tempo numa coluna de data fica só com a data.
        assert_eq!(conv("2026-01-15 07:30:00", TargetKind::Date).unwrap().as_deref(), Some("2026-01-15"));
        assert!(conv("31/02/2026", TargetKind::Date).is_err());
        assert!(conv("22/13/2026", TargetKind::Date).is_err());
        assert!(conv("ontem", TargetKind::Date).is_err());
    }

    #[test]
    fn timestamps_and_times() {
        assert_eq!(conv("22/10/26 21:32:09", TargetKind::Timestamp).unwrap().as_deref(), Some("2026-10-22 21:32:09"));
        assert_eq!(conv("22/10/2026 9:05", TargetKind::Timestamp).unwrap().as_deref(), Some("2026-10-22 09:05:00"));
        assert_eq!(conv("22/10/2026 9:05:03.250", TargetKind::Timestamp).unwrap().as_deref(), Some("2026-10-22 09:05:03.250"));
        assert_eq!(conv("22/10/2026 9:05 PM", TargetKind::Timestamp).unwrap().as_deref(), Some("2026-10-22 21:05:00"));
        assert_eq!(conv("22/10/2026", TargetKind::Timestamp).unwrap().as_deref(), Some("2026-10-22"));
        // ISO com fuso passa inteiro.
        assert_eq!(conv("2026-01-15 07:30:00-03", TargetKind::Timestamp).unwrap().as_deref(), Some("2026-01-15 07:30:00-03"));
        let td = ConvertOptions { date_time_order: DateTimeOrder::TimeDate, ..Default::default() };
        assert_eq!(convert("21:32:09 22/10/2026", TargetKind::Timestamp, &td).unwrap().as_deref(), Some("2026-10-22 21:32:09"));
        assert_eq!(conv("7:45", TargetKind::Time).unwrap().as_deref(), Some("07:45:00"));
        assert_eq!(conv("2026-01-15 07:30:00", TargetKind::Time).unwrap().as_deref(), Some("07:30:00"));
        assert!(conv("25:00", TargetKind::Time).is_err());
        assert!(conv("22/10/2026 99:00", TargetKind::Timestamp).is_err());
    }

    #[test]
    fn text_keeps_empty_unless_asked() {
        assert_eq!(conv("", TargetKind::Text).unwrap().as_deref(), Some(""));
        let nulls = ConvertOptions { empty_as_null: true, ..Default::default() };
        assert_eq!(convert("", TargetKind::Text, &nulls).unwrap(), None);
        assert_eq!(convert(" x ", TargetKind::Text, &nulls).unwrap().as_deref(), Some(" x "));
    }
}
