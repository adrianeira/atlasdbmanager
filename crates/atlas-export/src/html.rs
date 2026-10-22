//! HTML: uma página com uma tabela.

use crate::text::is_numeric_type;
use crate::{ExportColumn, RowWriter};
use std::io::{self, Write};

pub struct HtmlWriter {
    out: Box<dyn Write + Send>,
    title: String,
    header: bool,
    numeric: Vec<bool>,
}

impl HtmlWriter {
    pub fn new(out: Box<dyn Write + Send>, title: String, header: bool) -> Self {
        Self { out, title, header, numeric: Vec::new() }
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

impl RowWriter for HtmlWriter {
    fn begin(&mut self, columns: &[ExportColumn]) -> io::Result<()> {
        self.numeric = columns.iter().map(|c| is_numeric_type(c.type_name.as_deref())).collect();
        let mut s = String::new();
        s.push_str("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
        s.push_str(&format!("<title>{}</title>\n", escape(&self.title)));
        s.push_str(
            "<style>\n\
             table { border-collapse: collapse; font: 13px \"Segoe UI\", Arial, sans-serif; }\n\
             th, td { border: 1px solid #c8ccd0; padding: 3px 8px; text-align: left; white-space: pre-wrap; vertical-align: top; }\n\
             th { background: #f0f2f4; }\n\
             td.num { text-align: right; }\n\
             </style>\n</head>\n<body>\n<table>\n",
        );
        if self.header {
            s.push_str("<thead>\n<tr>");
            for c in columns {
                s.push_str(&format!("<th>{}</th>", escape(&c.name)));
            }
            s.push_str("</tr>\n</thead>\n");
        }
        s.push_str("<tbody>\n");
        self.out.write_all(s.as_bytes())
    }

    fn row(&mut self, values: &[Option<&str>]) -> io::Result<()> {
        let mut s = String::with_capacity(256);
        s.push_str("<tr>");
        for (i, v) in values.iter().enumerate() {
            let class = if self.numeric.get(i).copied().unwrap_or(false) { " class=\"num\"" } else { "" };
            // NULL fica como célula vazia.
            s.push_str(&format!("<td{class}>{}</td>", v.map(escape).unwrap_or_default()));
        }
        s.push_str("</tr>\n");
        self.out.write_all(s.as_bytes())
    }

    fn finish(mut self: Box<Self>) -> io::Result<()> {
        self.out.write_all(b"</tbody>\n</table>\n</body>\n</html>\n")?;
        self.out.flush()
    }
}
