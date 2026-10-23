//! dBase III/IV/FoxPro (`.dbf`) sem memo: campos C, N, F, L, D e (como texto) os demais.

use crate::{RowSource, invalid};
use encoding_rs::{Encoding, WINDOWS_1252};
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

struct Field {
    name: String,
    kind: u8,
    len: usize,
}

pub(crate) struct DbfRows {
    columns: Vec<String>,
    fields: Vec<Field>,
    reader: BufReader<File>,
    record_len: usize,
    remaining: u64,
    encoding: &'static Encoding,
}

/// Página de código do arquivo (byte 29 do cabeçalho): só o que o dBase usa de verdade no Windows e no Ocidente.
fn encoding_for(ldid: u8) -> &'static Encoding {
    match ldid {
        0x65 => encoding_rs::IBM866,
        0x66 => encoding_rs::IBM866,
        0x78 => encoding_rs::BIG5,
        0x7A => encoding_rs::GBK,
        0xC8 => encoding_rs::WINDOWS_1250,
        0xC9 => encoding_rs::WINDOWS_1251,
        _ => WINDOWS_1252,
    }
}

impl DbfRows {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let mut reader = BufReader::new(File::open(path)?);
        let mut head = [0u8; 32];
        reader.read_exact(&mut head).map_err(|_| invalid("arquivo dBase curto demais"))?;
        if !matches!(head[0], 0x02 | 0x03 | 0x04 | 0x05 | 0x30 | 0x31 | 0x32 | 0x83 | 0x8B | 0xF5) {
            return Err(invalid("não é um arquivo dBase (.dbf)"));
        }
        let count = u32::from_le_bytes(head[4..8].try_into().unwrap()) as u64;
        let header_len = u16::from_le_bytes(head[8..10].try_into().unwrap()) as usize;
        let record_len = u16::from_le_bytes(head[10..12].try_into().unwrap()) as usize;
        let encoding = encoding_for(head[29]);
        let mut fields = Vec::new();
        let mut consumed = 32;
        while consumed + 32 < header_len {
            let mut d = [0u8; 32];
            reader.read_exact(&mut d).map_err(|_| invalid("cabeçalho dBase incompleto"))?;
            consumed += 32;
            if d[0] == 0x0D {
                break;
            }
            let raw_name = &d[..11];
            let end = raw_name.iter().position(|b| *b == 0).unwrap_or(11);
            fields.push(Field { name: encoding.decode_without_bom_handling(&raw_name[..end]).0.into_owned(), kind: d[11], len: d[16] as usize });
        }
        // Pula o que sobrar do cabeçalho (terminador 0x0D e bytes extras do Visual FoxPro).
        let mut skip = header_len.saturating_sub(consumed);
        let mut sink = [0u8; 256];
        while skip > 0 {
            let n = reader.read(&mut sink[..skip.min(256)])?;
            if n == 0 {
                break;
            }
            skip -= n;
        }
        if fields.is_empty() || record_len == 0 {
            return Err(invalid("arquivo dBase sem campos"));
        }
        let columns = fields.iter().map(|f| f.name.clone()).collect();
        Ok(Self { columns, fields, reader, record_len, remaining: count, encoding })
    }

    fn text(&self, bytes: &[u8]) -> String {
        self.encoding.decode_without_bom_handling(bytes).0.trim_end_matches([' ', '\0']).to_string()
    }
}

impl RowSource for DbfRows {
    fn columns(&self) -> &[String] {
        &self.columns
    }

    fn next_row(&mut self) -> io::Result<Option<Vec<Option<String>>>> {
        let mut rec = vec![0u8; self.record_len];
        loop {
            if self.remaining == 0 {
                return Ok(None);
            }
            self.remaining -= 1;
            if self.reader.read_exact(&mut rec).is_err() {
                return Ok(None);
            }
            // '*' marca o registro excluído; 0x1A é o fim do arquivo.
            if rec[0] == b'*' {
                continue;
            }
            if rec[0] == 0x1A {
                return Ok(None);
            }
            let mut at = 1;
            let mut row = Vec::with_capacity(self.fields.len());
            for f in &self.fields {
                let raw = &rec[at..(at + f.len).min(rec.len())];
                at += f.len;
                let text = self.text(raw);
                row.push(match f.kind {
                    b'C' => Some(text),
                    b'N' | b'F' | b'I' | b'B' => (!text.trim().is_empty()).then(|| text.trim().to_string()),
                    b'L' => match text.trim() {
                        "T" | "t" | "Y" | "y" => Some("true".into()),
                        "F" | "f" | "N" | "n" => Some("false".into()),
                        _ => None,
                    },
                    b'D' => {
                        let t = text.trim();
                        (t.len() == 8 && t.bytes().all(|b| b.is_ascii_digit()) && t != "00000000").then(|| format!("{}-{}-{}", &t[..4], &t[4..6], &t[6..8]))
                    }
                    // Memo e outros: o conteúdo do campo como texto, se houver.
                    _ => (!text.trim().is_empty()).then(|| text.trim().to_string()),
                });
            }
            return Ok(Some(row));
        }
    }
}
