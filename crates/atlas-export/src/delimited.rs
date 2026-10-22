//! CSV / TXT.

use crate::{Encoding, ExportColumn, RowWriter};
use std::io::{self, Write};

pub struct DelimitedWriter {
    out: Box<dyn Write + Send>,
    delimiter: String,
    qualifier: Option<char>,
    header: bool,
    eol: &'static str,
    null_text: String,
    encoding: Encoding,
    line: String,
}

impl DelimitedWriter {
    pub fn new(
        out: Box<dyn Write + Send>,
        delimiter: String,
        qualifier: Option<char>,
        header: bool,
        crlf: bool,
        null_text: String,
        encoding: Encoding,
    ) -> Self {
        let delimiter = if delimiter.is_empty() { ",".to_string() } else { delimiter };
        Self { out, delimiter, qualifier, header, eol: if crlf { "\r\n" } else { "\n" }, null_text, encoding, line: String::with_capacity(512) }
    }

    fn push_field(&mut self, value: Option<&str>) {
        match value {
            None => self.line.push_str(&self.null_text),
            Some(v) => match self.qualifier {
                Some(q) => {
                    // Vazio com NULL também vazio: as aspas distinguem texto vazio de NULL.
                    let needs = (v.is_empty() && self.null_text.is_empty())
                        || v.contains(self.delimiter.as_str())
                        || v.contains(q)
                        || v.contains('\n')
                        || v.contains('\r');
                    if needs {
                        self.line.push(q);
                        for ch in v.chars() {
                            if ch == q {
                                self.line.push(q);
                            }
                            self.line.push(ch);
                        }
                        self.line.push(q);
                    } else {
                        self.line.push_str(v);
                    }
                }
                None => self.line.push_str(v),
            },
        }
    }

    fn flush_line(&mut self) -> io::Result<()> {
        self.line.push_str(self.eol);
        match self.encoding {
            Encoding::Utf8 | Encoding::Utf8Bom => self.out.write_all(self.line.as_bytes())?,
            Encoding::Windows1252 => self.out.write_all(&encode_windows_1252(&self.line))?,
        }
        self.line.clear();
        Ok(())
    }
}

/// Windows-1252; o que não existe na tabela vira `?` (em vez de referências numéricas).
pub(crate) fn encode_windows_1252(s: &str) -> Vec<u8> {
    let (bytes, _, had_errors) = encoding_rs::WINDOWS_1252.encode(s);
    if !had_errors {
        return bytes.into_owned();
    }
    let mut out = Vec::with_capacity(s.len());
    let mut buf = [0u8; 4];
    for ch in s.chars() {
        let (b, _, bad) = encoding_rs::WINDOWS_1252.encode(ch.encode_utf8(&mut buf));
        if bad {
            out.push(b'?');
        } else {
            out.extend_from_slice(&b);
        }
    }
    out
}

impl RowWriter for DelimitedWriter {
    fn begin(&mut self, columns: &[ExportColumn]) -> io::Result<()> {
        if self.encoding == Encoding::Utf8Bom {
            self.out.write_all(&[0xEF, 0xBB, 0xBF])?;
        }
        if self.header {
            for (i, c) in columns.iter().enumerate() {
                if i > 0 {
                    self.line.push_str(&self.delimiter);
                }
                self.push_field(Some(&c.name));
            }
            self.flush_line()?;
        }
        Ok(())
    }

    fn row(&mut self, values: &[Option<&str>]) -> io::Result<()> {
        for (i, v) in values.iter().enumerate() {
            if i > 0 {
                self.line.push_str(&self.delimiter);
            }
            self.push_field(*v);
        }
        self.flush_line()
    }

    fn finish(mut self: Box<Self>) -> io::Result<()> {
        self.out.flush()
    }
}
