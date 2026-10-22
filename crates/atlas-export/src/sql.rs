//! Comandos `INSERT`.

use crate::text::{is_bool_type, is_json_number, is_numeric_type, quote_ident};
use crate::{ExportColumn, RowWriter};
use std::io::{self, Write};

pub struct SqlWriter {
    out: Box<dyn Write + Send>,
    target: String,
    rows_per_insert: usize,
    prefix: String,
    types: Vec<Option<String>>,
    pending: Vec<String>,
}

impl SqlWriter {
    pub fn new(out: Box<dyn Write + Send>, target: String, rows_per_insert: usize) -> Self {
        Self { out, target, rows_per_insert, prefix: String::new(), types: Vec::new(), pending: Vec::new() }
    }

    fn flush_pending(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let mut s = String::with_capacity(self.prefix.len() + self.pending.iter().map(|r| r.len() + 3).sum::<usize>() + 4);
        s.push_str(&self.prefix);
        s.push('\n');
        s.push_str(&self.pending.join(",\n"));
        s.push_str(";\n");
        self.pending.clear();
        self.out.write_all(s.as_bytes())
    }
}

/// Literal SQL: NULL, TRUE/FALSE, número sem aspas quando válido, texto com aspas simples dobradas.
pub fn sql_literal(type_name: Option<&str>, v: Option<&str>) -> String {
    let Some(s) = v else { return "NULL".into() };
    if is_bool_type(type_name) {
        match s {
            "t" | "true" => return "TRUE".into(),
            "f" | "false" => return "FALSE".into(),
            _ => {}
        }
    }
    if is_numeric_type(type_name) && is_json_number(s) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "''"))
}

impl RowWriter for SqlWriter {
    fn begin(&mut self, columns: &[ExportColumn]) -> io::Result<()> {
        let cols: Vec<String> = columns.iter().map(|c| quote_ident(&c.name)).collect();
        self.prefix = format!("INSERT INTO {} ({}) VALUES", self.target, cols.join(", "));
        self.types = columns.iter().map(|c| c.type_name.clone()).collect();
        Ok(())
    }

    fn row(&mut self, values: &[Option<&str>]) -> io::Result<()> {
        let lits: Vec<String> = values.iter().enumerate().map(|(i, v)| sql_literal(self.types.get(i).and_then(|t| t.as_deref()), *v)).collect();
        self.pending.push(format!("({})", lits.join(", ")));
        if self.pending.len() >= self.rows_per_insert {
            self.flush_pending()?;
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> io::Result<()> {
        self.flush_pending()?;
        self.out.flush()
    }
}
