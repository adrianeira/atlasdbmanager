//! TXT/CSV: leitura em streaming, com qualificador de texto (aspas duplicadas, quebras de linha dentro do campo),
//! delimitador de um ou mais caracteres, fim de linha CRLF/LF/CR e decodificação UTF-8 (com BOM) ou Windows-1252.

use crate::{RawRows, SourceEncoding};
use encoding_rs::{Decoder, UTF_8, WINDOWS_1252};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

const CHUNK: usize = 256 * 1024;

pub(crate) struct DelimitedRows<R: Read + Send> {
    src: R,
    decoder: Decoder,
    text: String,
    pos: usize,
    eof: bool,
    delimiter: String,
    qualifier: Option<char>,
}

/// O texto ainda não tem o suficiente para decidir (falta o que vem depois do que já foi lido).
struct NeedMore;

impl DelimitedRows<File> {
    pub(crate) fn open(path: &Path, delimiter: &str, qualifier: &str, encoding: SourceEncoding) -> io::Result<Self> {
        Ok(Self::new(File::open(path)?, delimiter, qualifier, encoding))
    }
}

impl<R: Read + Send> DelimitedRows<R> {
    pub(crate) fn new(src: R, delimiter: &str, qualifier: &str, encoding: SourceEncoding) -> Self {
        let decoder = match encoding {
            SourceEncoding::Utf8 => UTF_8.new_decoder_with_bom_removal(),
            SourceEncoding::Windows1252 => WINDOWS_1252.new_decoder_with_bom_removal(),
        };
        Self {
            src,
            decoder,
            text: String::new(),
            pos: 0,
            eof: false,
            delimiter: if delimiter.is_empty() { ",".into() } else { delimiter.replace("\\t", "\t") },
            qualifier: qualifier.chars().next(),
        }
    }

    /// Lê mais um pedaço do arquivo e o acrescenta ao texto (descartando o que já foi consumido).
    fn fill(&mut self) -> io::Result<()> {
        if self.pos > 0 {
            self.text.drain(..self.pos);
            self.pos = 0;
        }
        let mut buf = vec![0u8; CHUNK];
        let n = self.src.read(&mut buf)?;
        let last = n == 0;
        self.text.reserve(n * 2 + 8);
        // Decodifica sem perder bytes de um caractere cortado no fim do pedaço.
        let mut input = &buf[..n];
        loop {
            let (result, read, _) = self.decoder.decode_to_string(input, &mut self.text, last);
            input = &input[read..];
            match result {
                encoding_rs::CoderResult::InputEmpty => break,
                encoding_rs::CoderResult::OutputFull => self.text.reserve(input.len() * 2 + 16),
            }
        }
        if last {
            self.eof = true;
        }
        Ok(())
    }

    /// Lê um registro a partir de `self.pos`: os campos e a posição logo depois do fim da linha.
    fn parse(&self) -> Result<Option<(Vec<String>, usize)>, NeedMore> {
        let t = &self.text;
        let d = self.delimiter.as_str();
        let mut i = self.pos;
        if i >= t.len() {
            return if self.eof { Ok(None) } else { Err(NeedMore) };
        }
        let mut fields = Vec::new();
        loop {
            let mut field = String::new();
            let quoted = self.qualifier.is_some_and(|q| t[i..].starts_with(q));
            if quoted {
                let q = self.qualifier.unwrap();
                let ql = q.len_utf8();
                i += ql;
                loop {
                    match t[i..].find(q) {
                        None => {
                            if !self.eof {
                                return Err(NeedMore);
                            }
                            // Aspas que nunca fecham: o resto do arquivo é o campo.
                            field.push_str(&t[i..]);
                            i = t.len();
                            break;
                        }
                        Some(off) => {
                            field.push_str(&t[i..i + off]);
                            let after = i + off + ql;
                            if t[after..].starts_with(q) {
                                field.push(q);
                                i = after + ql;
                            } else if after >= t.len() && !self.eof {
                                return Err(NeedMore);
                            } else {
                                i = after;
                                break;
                            }
                        }
                    }
                }
                // Depois das aspas, o que sobrar até o delimitador ou o fim da linha entra no campo.
            }
            // Campo sem aspas (ou o resto de um com aspas): até o delimitador ou o fim da linha.
            loop {
                if i >= t.len() {
                    if !self.eof {
                        return Err(NeedMore);
                    }
                    fields.push(field);
                    return Ok(Some((fields, i)));
                }
                let rest = &t[i..];
                if rest.starts_with(d) {
                    i += d.len();
                    fields.push(std::mem::take(&mut field));
                    break;
                }
                if !self.eof && rest.len() < d.len().max(2) {
                    // Pode ser o começo de um delimitador (ou um \r antes de \n) cortado no fim do pedaço.
                    return Err(NeedMore);
                }
                let c = rest.chars().next().unwrap();
                if c == '\r' || c == '\n' {
                    i += if rest.starts_with("\r\n") { 2 } else { 1 };
                    fields.push(field);
                    return Ok(Some((fields, i)));
                }
                field.push(c);
                i += c.len_utf8();
            }
        }
    }
}

impl<R: Read + Send> RawRows for DelimitedRows<R> {
    fn next_raw(&mut self) -> io::Result<Option<Vec<Option<String>>>> {
        loop {
            match self.parse() {
                Ok(None) => return Ok(None),
                Ok(Some((fields, end))) => {
                    self.pos = end;
                    // Linha em branco: não é registro.
                    if fields.len() == 1 && fields[0].is_empty() {
                        continue;
                    }
                    return Ok(Some(fields.into_iter().map(Some).collect()));
                }
                Err(NeedMore) => self.fill()?,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(data: &[u8], delimiter: &str, qualifier: &str, encoding: SourceEncoding) -> Vec<Vec<String>> {
        let mut r = DelimitedRows::new(data, delimiter, qualifier, encoding);
        let mut out = Vec::new();
        while let Some(row) = r.next_raw().unwrap() {
            out.push(row.into_iter().map(|c| c.unwrap()).collect());
        }
        out
    }

    #[test]
    fn quotes_doubled_quotes_and_newlines_inside_fields() {
        let rows = read_all(b"a,b\r\n\"x, y\",\"diz \"\"oi\"\"\"\r\n\"duas\nlinhas\",z\r\n", ",", "\"", SourceEncoding::Utf8);
        assert_eq!(rows, [["a", "b"], ["x, y", "diz \"oi\""], ["duas\nlinhas", "z"]]);
    }

    #[test]
    fn line_endings_blank_lines_and_missing_final_newline() {
        assert_eq!(read_all(b"1;2\n3;4\r5;6\n\n7;8", ";", "\"", SourceEncoding::Utf8), [["1", "2"], ["3", "4"], ["5", "6"], ["7", "8"]]);
        assert_eq!(read_all(b"a,\n,b\n,\n", ",", "", SourceEncoding::Utf8), [["a", ""], ["", "b"], ["", ""]]);
    }

    #[test]
    fn multi_character_delimiter_tab_and_no_qualifier() {
        assert_eq!(read_all(b"a||b||c\nd||e||f", "||", "", SourceEncoding::Utf8), [["a", "b", "c"], ["d", "e", "f"]]);
        assert_eq!(read_all(b"a\tb\n\"q\"\tc", "\t", "", SourceEncoding::Utf8), [["a", "b"], ["\"q\"", "c"]]);
    }

    #[test]
    fn utf8_bom_and_windows_1252() {
        assert_eq!(read_all(b"\xEF\xBB\xBFnome\nJo\xC3\xA3o", ",", "\"", SourceEncoding::Utf8), [["nome"], ["João"]]);
        assert_eq!(read_all(b"a\xE7\xE3o;\xE9", ";", "\"", SourceEncoding::Windows1252), [["ação", "é"]]);
    }

    #[test]
    fn works_across_read_boundaries() {
        // Um leitor que entrega 1 byte por vez: cada decisão de "falta ler mais" é exercitada.
        struct OneByte<'a>(&'a [u8]);
        impl Read for OneByte<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.0.is_empty() || buf.is_empty() {
                    return Ok(0);
                }
                buf[0] = self.0[0];
                self.0 = &self.0[1..];
                Ok(1)
            }
        }
        let data = "id||nome\r\n1||\"Jo\"\"ão\nSilva\"\r\n2||Zé\r\n".as_bytes();
        let mut r = DelimitedRows::new(OneByte(data), "||", "\"", SourceEncoding::Utf8);
        let mut out = Vec::new();
        while let Some(row) = r.next_raw().unwrap() {
            out.push(row.into_iter().map(|c| c.unwrap()).collect::<Vec<_>>());
        }
        assert_eq!(out, [vec!["id", "nome"], vec!["1", "Jo\"ão\nSilva"], vec!["2", "Zé"]]);
    }
}
