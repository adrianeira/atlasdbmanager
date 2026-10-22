//! JSON: um array de objetos, um objeto por linha da consulta.

use crate::text::{is_bool_type, is_json_number, is_json_type, is_numeric_type};
use crate::{ExportColumn, RowWriter};
use std::io::{self, Write};

pub struct JsonWriter {
    out: Box<dyn Write + Send>,
    pretty: bool,
    /// Nome da coluna já escapado como texto JSON.
    names: Vec<String>,
    types: Vec<Option<String>>,
    first: bool,
}

impl JsonWriter {
    pub fn new(out: Box<dyn Write + Send>, pretty: bool) -> Self {
        Self { out, pretty, names: Vec::new(), types: Vec::new(), first: true }
    }
}

fn json_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// Valor JSON de uma célula pelo tipo da coluna: números e booleanos nativos, `json`/`jsonb`
/// embutidos como estão, o resto como texto.
pub fn json_value(type_name: Option<&str>, v: Option<&str>) -> String {
    let Some(s) = v else { return "null".into() };
    if is_bool_type(type_name) {
        return match s {
            "t" | "true" => "true".into(),
            "f" | "false" => "false".into(),
            _ => json_string(s),
        };
    }
    if is_numeric_type(type_name) && is_json_number(s) {
        return s.to_string();
    }
    if is_json_type(type_name) && serde_json::from_str::<serde::de::IgnoredAny>(s).is_ok() {
        return s.to_string();
    }
    json_string(s)
}

impl RowWriter for JsonWriter {
    fn begin(&mut self, columns: &[ExportColumn]) -> io::Result<()> {
        self.names = columns.iter().map(|c| json_string(&c.name)).collect();
        self.types = columns.iter().map(|c| c.type_name.clone()).collect();
        self.out.write_all(b"[")
    }

    fn row(&mut self, values: &[Option<&str>]) -> io::Result<()> {
        let mut s = String::with_capacity(256);
        if !self.first {
            s.push(',');
        }
        self.first = false;
        if self.pretty {
            s.push_str("\n  {");
        } else {
            s.push('{');
        }
        for (i, v) in values.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            if self.pretty {
                s.push_str("\n    ");
            }
            s.push_str(&self.names[i]);
            s.push_str(if self.pretty { ": " } else { ":" });
            s.push_str(&json_value(self.types.get(i).and_then(|t| t.as_deref()), *v));
        }
        s.push_str(if self.pretty { "\n  }" } else { "}" });
        self.out.write_all(s.as_bytes())
    }

    fn finish(mut self: Box<Self>) -> io::Result<()> {
        self.out.write_all(if self.pretty && !self.first { b"\n]\n" } else { b"]\n" })?;
        self.out.flush()
    }
}
