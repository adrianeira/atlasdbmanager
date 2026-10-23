//! XML: os filhos do elemento raiz são os registros; os filhos de cada registro (e os atributos dele) são os campos.
//! Entende o que o Atlas exporta (`<table><row><campo>valor</campo><campo null="true"/></row></table>`) e XML comum.

use crate::{RowSource, invalid};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::fs::File;
use std::io::{self, BufReader};
use std::path::Path;

const MAX_BYTES: u64 = 512 * 1024 * 1024;

pub(crate) struct XmlRows {
    columns: Vec<String>,
    rows: std::vec::IntoIter<Vec<Option<String>>>,
}

type Record = Vec<(String, Option<String>)>;

fn local(name: &[u8]) -> String {
    let s = String::from_utf8_lossy(name);
    s.rsplit(':').next().unwrap_or(&s).to_string()
}

fn xml_err(e: impl std::fmt::Display) -> io::Error {
    invalid(format!("XML inválido: {e}"))
}

/// Nome do campo: o atributo `name` (o Atlas guarda ali o nome original quando o elemento teve de ser ajustado) ou o do elemento.
fn field_name(e: &BytesStart<'_>) -> String {
    for a in e.attributes().flatten() {
        if a.key.as_ref() == b"name"
            && let Ok(v) = a.unescape_value()
        {
            return v.into_owned();
        }
    }
    local(e.name().as_ref())
}

fn is_null(e: &BytesStart<'_>) -> bool {
    e.attributes().flatten().any(|a| {
        let key = local(a.key.as_ref());
        (key == "null" || key == "nil") && a.unescape_value().map(|v| v == "true" || v == "1").unwrap_or(false)
    })
}

impl XmlRows {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        if file.metadata()?.len() > MAX_BYTES {
            return Err(invalid("arquivo XML grande demais para importar (limite de 512 MB)"));
        }
        let mut reader = Reader::from_reader(BufReader::new(file));
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        let mut records: Vec<Record> = Vec::new();
        // Profundidade: 0 = fora, 1 = raiz, 2 = registro, 3 = campo, 4+ = dentro do campo (texto achatado).
        let mut depth = 0usize;
        let mut record: Record = Vec::new();
        let mut field: Option<(String, String, bool)> = None;
        loop {
            match reader.read_event_into(&mut buf).map_err(xml_err)? {
                Event::Start(e) => {
                    depth += 1;
                    match depth {
                        2 => {
                            record = Vec::new();
                            // Atributos do registro também são campos.
                            for a in e.attributes().flatten() {
                                let key = local(a.key.as_ref());
                                if key != "name" {
                                    record.push((key, a.unescape_value().ok().map(|v| v.into_owned())));
                                }
                            }
                        }
                        3 => field = Some((field_name(&e), String::new(), is_null(&e))),
                        _ => {}
                    }
                }
                Event::Empty(e) => match depth + 1 {
                    2 => records.push(Vec::new()),
                    3 => record.push((field_name(&e), if is_null(&e) { None } else { Some(String::new()) })),
                    _ => {}
                },
                Event::Text(t) => {
                    if let (true, Some((_, text, _))) = (depth >= 3, field.as_mut()) {
                        text.push_str(&t.unescape().map_err(xml_err)?);
                    }
                }
                Event::CData(t) => {
                    if let (true, Some((_, text, _))) = (depth >= 3, field.as_mut()) {
                        text.push_str(&String::from_utf8_lossy(&t.into_inner()));
                    }
                }
                Event::End(_) => {
                    match depth {
                        3 => {
                            if let Some((name, text, null)) = field.take() {
                                record.push((name, if null { None } else { Some(text) }));
                            }
                        }
                        2 => records.push(std::mem::take(&mut record)),
                        _ => {}
                    }
                    depth = depth.saturating_sub(1);
                }
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        let mut columns: Vec<String> = Vec::new();
        for r in &records {
            for (name, _) in r {
                if !columns.contains(name) {
                    columns.push(name.clone());
                }
            }
        }
        let rows: Vec<Vec<Option<String>>> = records
            .into_iter()
            .map(|r| columns.iter().map(|c| r.iter().find(|(n, _)| n == c).and_then(|(_, v)| v.clone())).collect())
            .collect();
        if columns.is_empty() {
            return Err(invalid("o XML não tem registros (esperado: elementos dentro do elemento raiz)"));
        }
        Ok(Self { columns, rows: rows.into_iter() })
    }
}

impl RowSource for XmlRows {
    fn columns(&self) -> &[String] {
        &self.columns
    }
    fn next_row(&mut self) -> io::Result<Option<Vec<Option<String>>>> {
        Ok(self.rows.next())
    }
}
