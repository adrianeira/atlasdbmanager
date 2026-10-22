//! XML: `<table name="..."><row><coluna>valor</coluna>...</row>...</table>`.

use crate::{ExportColumn, RowWriter};
use std::io::{self, Write};

pub struct XmlWriter {
    out: Box<dyn Write + Send>,
    title: String,
    /// (nome do elemento, nome original quando precisou ser ajustado).
    names: Vec<(String, Option<String>)>,
}

impl XmlWriter {
    pub fn new(out: Box<dyn Write + Send>, title: String) -> Self {
        Self { out, title, names: Vec::new() }
    }
}

/// Remove os caracteres que o XML 1.0 não admite e escapa `& < > "`.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

/// Nome de elemento XML válido a partir do nome da coluna (`None` se já servia).
pub fn element_name(name: &str) -> (String, Option<String>) {
    let mut el: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '_' | '-' | '.') { c } else { '_' }).collect();
    if !el.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') {
        el.insert(0, '_');
    }
    let changed = el != name;
    (el, changed.then(|| name.to_string()))
}

impl RowWriter for XmlWriter {
    fn begin(&mut self, columns: &[ExportColumn]) -> io::Result<()> {
        self.names = columns.iter().map(|c| element_name(&c.name)).collect();
        write!(self.out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<table name=\"{}\">\n", escape(&self.title))
    }

    fn row(&mut self, values: &[Option<&str>]) -> io::Result<()> {
        let mut s = String::with_capacity(256);
        s.push_str("  <row>\n");
        for (i, v) in values.iter().enumerate() {
            let (el, orig) = &self.names[i];
            let attr = orig.as_ref().map(|o| format!(" name=\"{}\"", escape(o))).unwrap_or_default();
            match v {
                Some(text) => s.push_str(&format!("    <{el}{attr}>{}</{el}>\n", escape(text))),
                None => s.push_str(&format!("    <{el}{attr} null=\"true\"/>\n")),
            }
        }
        s.push_str("  </row>\n");
        self.out.write_all(s.as_bytes())
    }

    fn finish(mut self: Box<Self>) -> io::Result<()> {
        self.out.write_all(b"</table>\n")?;
        self.out.flush()
    }
}
