//! dBase III (`.dbf`) em streaming: registros de largura fixa, texto em Windows-1252, sem arquivo de memo.
//!
//! Como o cabeçalho (tamanho dos campos) vem antes dos dados e o resultado não cabe na memória, as larguras
//! saem do tipo da coluna: inteiros e booleanos têm largura própria, números decimais usam `decimals` casas e texto
//! usa `text_length` bytes (máximo 254; o que passar disso é cortado). Valor que não cabe no campo numérico é erro.

use crate::delimited::encode_windows_1252;
use crate::{ExportColumn, RowWriter};
use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};

/// Campos por tabela (limite do dBase IV/FoxPro; o dBase III clássico para em 128).
const MAX_FIELDS: usize = 255;
/// Largura total de um campo numérico decimal.
const DECIMAL_WIDTH: usize = 20;
const LANGUAGE_DRIVER_WINDOWS_ANSI: u8 = 0x03;

#[derive(Clone, Copy)]
enum Kind {
    Text,
    Number,
    Logical,
    Date,
}

struct Field {
    name: [u8; 11],
    kind: Kind,
    len: usize,
    dec: usize,
    column: String,
}

pub struct DbfWriter {
    out: BufWriter<File>,
    decimals: usize,
    text_length: usize,
    fields: Vec<Field>,
    record_len: usize,
    rows: u32,
    buf: Vec<u8>,
}

impl DbfWriter {
    pub fn new(file: File, decimals: u32, text_length: u32) -> Self {
        Self {
            out: BufWriter::with_capacity(256 * 1024, file),
            decimals: (decimals as usize).min(DECIMAL_WIDTH - 6),
            text_length: (text_length as usize).clamp(1, 254),
            fields: Vec::new(),
            record_len: 0,
            rows: 0,
            buf: Vec::new(),
        }
    }
}

/// Nome do campo: até 10 caracteres ASCII (letras, dígitos e `_`), maiúsculos e únicos.
fn field_names(columns: &[ExportColumn]) -> Vec<[u8; 11]> {
    let mut used: Vec<String> = Vec::new();
    columns
        .iter()
        .map(|c| {
            let mut base: String = c.name.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_uppercase() } else { '_' }).collect();
            if base.is_empty() {
                base.push_str("FIELD");
            }
            base.truncate(10);
            let mut name = base.clone();
            let mut n = 1;
            while used.contains(&name) {
                n += 1;
                let suffix = n.to_string();
                name = format!("{}{suffix}", &base[..base.len().min(10 - suffix.len())]);
            }
            used.push(name.clone());
            let mut out = [0u8; 11];
            out[..name.len()].copy_from_slice(name.as_bytes());
            out
        })
        .collect()
}

fn field_for(ty: Option<&str>, decimals: usize, text_length: usize) -> (Kind, usize, usize) {
    match ty {
        Some("int2") => (Kind::Number, 6, 0),
        Some("int4") => (Kind::Number, 11, 0),
        Some("int8") => (Kind::Number, 20, 0),
        Some("oid") => (Kind::Number, 10, 0),
        Some("float4" | "float8" | "numeric") => (Kind::Number, DECIMAL_WIDTH, decimals),
        Some("bool") => (Kind::Logical, 1, 0),
        Some("date") => (Kind::Date, 8, 0),
        Some("uuid") => (Kind::Text, 36, 0),
        Some("timestamp") => (Kind::Text, 26, 0),
        Some("timestamptz") => (Kind::Text, 35, 0),
        Some("time") => (Kind::Text, 15, 0),
        Some("timetz") => (Kind::Text, 21, 0),
        _ => (Kind::Text, text_length, 0),
    }
}

/// Data de hoje (UTC) como (ano, mês, dia), sem dependência de calendário.
fn today() -> (i64, u32, u32) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    civil_from_days(secs.div_euclid(86_400))
}

/// Algoritmo de Howard Hinnant: dias desde 1970-01-01 → data civil.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Texto de um número com exatamente `dec` casas (arredondamento "meio para cima" no decimal, sem passar por
/// `f64` quando o texto já é decimal simples). `None` = não numérico (NaN, Infinity): vira campo em branco.
fn format_number(s: &str, dec: usize) -> Option<String> {
    let t = s.trim();
    let (neg, rest) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let plain = !rest.is_empty()
        && rest.chars().all(|c| c.is_ascii_digit() || c == '.')
        && rest.matches('.').count() <= 1
        && rest != ".";
    let (int_part, frac_part) = if plain {
        let (i, f) = rest.split_once('.').unwrap_or((rest, ""));
        (i.trim_start_matches('0').to_string(), f.to_string())
    } else {
        // Notação científica (floats): passa por f64.
        let v: f64 = t.parse().ok().filter(|v: &f64| v.is_finite())?;
        let text = format!("{:.*}", dec, v.abs());
        let (i, f) = text.split_once('.').unwrap_or((text.as_str(), ""));
        return Some(compose(v.is_sign_negative(), i, f));
    };
    // Corta/arredonda a parte fracionária para `dec` dígitos.
    let mut digits: Vec<u8> = int_part.bytes().chain(frac_part.bytes().take(dec)).map(|b| b - b'0').collect();
    let kept_frac = frac_part.len().min(dec);
    digits.extend(std::iter::repeat_n(0, dec - kept_frac));
    if frac_part.as_bytes().get(dec).is_some_and(|b| *b >= b'5') {
        let mut i = digits.len();
        loop {
            if i == 0 {
                digits.insert(0, 1);
                break;
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    let total_int = digits.len() - dec;
    let text: String = digits.iter().map(|d| (b'0' + d) as char).collect();
    let (i, f) = text.split_at(total_int);
    Some(compose(neg, i, f))
}

/// Junta sinal, parte inteira e fração; `-0` vira `0`.
fn compose(neg: bool, int_part: &str, frac: &str) -> String {
    let i = int_part.trim_start_matches('0');
    let i = if i.is_empty() { "0" } else { i };
    let zero = i == "0" && frac.chars().all(|c| c == '0');
    format!("{}{}{}{}", if neg && !zero { "-" } else { "" }, i, if frac.is_empty() { "" } else { "." }, frac)
}

impl RowWriter for DbfWriter {
    fn begin(&mut self, columns: &[ExportColumn]) -> io::Result<()> {
        if columns.is_empty() || columns.len() > MAX_FIELDS {
            return Err(io::Error::other(format!("the .dbf format needs between 1 and {MAX_FIELDS} fields")));
        }
        let names = field_names(columns);
        self.fields = columns
            .iter()
            .zip(names)
            .map(|(c, name)| {
                let (kind, len, dec) = field_for(c.type_name.as_deref(), self.decimals, self.text_length);
                Field { name, kind, len, dec, column: c.name.clone() }
            })
            .collect();
        self.record_len = 1 + self.fields.iter().map(|f| f.len).sum::<usize>();
        if self.record_len > u16::MAX as usize {
            return Err(io::Error::other("the record is too wide for the .dbf format; lower the text field length"));
        }
        let header_len = 32 + 32 * self.fields.len() + 1;

        let (y, m, d) = today();
        let mut head = vec![0u8; 32];
        head[0] = 0x03;
        head[1] = (y - 1900).clamp(0, 255) as u8;
        head[2] = m as u8;
        head[3] = d as u8;
        head[8..10].copy_from_slice(&(header_len as u16).to_le_bytes());
        head[10..12].copy_from_slice(&(self.record_len as u16).to_le_bytes());
        head[29] = LANGUAGE_DRIVER_WINDOWS_ANSI;
        self.out.write_all(&head)?;
        for f in &self.fields {
            let mut desc = [0u8; 32];
            desc[..11].copy_from_slice(&f.name);
            desc[11] = match f.kind {
                Kind::Text => b'C',
                Kind::Number => b'N',
                Kind::Logical => b'L',
                Kind::Date => b'D',
            };
            desc[16] = f.len as u8;
            desc[17] = f.dec as u8;
            self.out.write_all(&desc)?;
        }
        self.out.write_all(&[0x0D])
    }

    fn row(&mut self, values: &[Option<&str>]) -> io::Result<()> {
        self.buf.clear();
        self.buf.push(b' '); // registro não excluído
        for (f, v) in self.fields.iter().zip(values.iter().copied().chain(std::iter::repeat(None))) {
            let start = self.buf.len();
            match (f.kind, v) {
                (Kind::Logical, Some("t")) => self.buf.push(b'T'),
                (Kind::Logical, Some("f")) => self.buf.push(b'F'),
                (Kind::Logical, _) => self.buf.push(b'?'),
                (Kind::Date, Some(s)) if s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' => {
                    self.buf.extend(s.bytes().filter(|b| *b != b'-'));
                }
                (Kind::Number, Some(s)) => {
                    if let Some(text) = format_number(s, f.dec) {
                        if text.len() > f.len {
                            return Err(io::Error::other(format!(
                                "the value {s} of column \"{}\" does not fit in a .dbf numeric field ({} characters)",
                                f.column, f.len
                            )));
                        }
                        self.buf.resize(start + f.len - text.len(), b' ');
                        self.buf.extend_from_slice(text.as_bytes());
                    }
                }
                (Kind::Text, Some(s)) => {
                    let mut bytes = encode_windows_1252(s);
                    bytes.truncate(f.len);
                    self.buf.extend_from_slice(&bytes);
                }
                _ => {}
            }
            // Tudo que não preencheu o campo (NULL, valor vazio, texto curto) fica em branco.
            self.buf.resize(start + f.len, b' ');
        }
        self.out.write_all(&self.buf)?;
        self.rows += 1;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> io::Result<()> {
        self.out.write_all(&[0x1A])?;
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&self.rows.to_le_bytes())?;
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_keep_exact_decimals() {
        assert_eq!(format_number("1500.5000", 4).as_deref(), Some("1500.5000"));
        assert_eq!(format_number("1.005", 2).as_deref(), Some("1.01"));
        assert_eq!(format_number("0.994", 2).as_deref(), Some("0.99"));
        assert_eq!(format_number("9.995", 2).as_deref(), Some("10.00"));
        assert_eq!(format_number("-0.001", 2).as_deref(), Some("0.00"));
        assert_eq!(format_number("-12", 0).as_deref(), Some("-12"));
        assert_eq!(format_number("12.5", 0).as_deref(), Some("13"));
        assert_eq!(format_number("3", 2).as_deref(), Some("3.00"));
        assert_eq!(format_number("1e3", 1).as_deref(), Some("1000.0"));
        assert_eq!(format_number("NaN", 2), None);
        assert_eq!(format_number("Infinity", 2), None);
    }

    #[test]
    fn field_names_are_ascii_short_and_unique() {
        let cols: Vec<ExportColumn> = ["nome completo", "nome_completo_2", "nome_completo_3", "ação", ""]
            .iter()
            .map(|n| ExportColumn { name: (*n).into(), type_name: None })
            .collect();
        let names: Vec<String> = field_names(&cols).iter().map(|n| String::from_utf8_lossy(&n[..n.iter().position(|b| *b == 0).unwrap()]).into_owned()).collect();
        assert_eq!(names[0], "NOME_COMPL");
        assert_eq!(names[1], "NOME_COMP2");
        assert_eq!(names[2], "NOME_COMP3");
        assert_eq!(names[3], "A__O");
        assert_eq!(names[4], "FIELD");
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }
}
