//! Excel 97-2003 (`.xls`, BIFF8) gravado em streaming dentro de um arquivo composto (OLE2).
//!
//! As partes fixas do fluxo `Workbook` (fontes, estilos, configuração de página) ficam em `xls/*.bin`, extraídas de
//! um arquivo de referência que o Excel abre; aqui só se escreve o que varia: nome da planilha, dimensões, larguras
//! e as células. Os limites são os do formato: 65.536 linhas, 256 colunas e texto curto por célula.

use crate::text::{is_bool_type, is_json_number, is_numeric_type};
use crate::xlsx::sheet_name;
use crate::{ExportColumn, RowWriter};
use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};

/// Globais até antes do `BOUNDSHEET`.
const HEAD: &[u8] = include_bytes!("xls/head.bin");
/// `BOF` da planilha até `WSBOOL`.
const SHEET_A: &[u8] = include_bytes!("xls/sheet_a.bin");
/// `PRINTHEADERS` até `PASSWORD` (configuração de página).
const SHEET_B: &[u8] = include_bytes!("xls/sheet_b.bin");
/// `WINDOW2` e `EOF`.
const SHEET_END: &[u8] = include_bytes!("xls/sheet_end.bin");

const MAX_ROWS: u32 = 65_536;
const MAX_COLS: usize = 256;
/// Um registro BIFF8 carrega até 8.224 bytes; 4.000 caracteres em UTF-16 cabem com folga (sem registros CONTINUE).
const MAX_TEXT_CHARS: usize = 4_000;
/// Índices dos estilos (XF) do modelo: texto normal e negrito.
const XF_PLAIN: u16 = 18;
const XF_BOLD: u16 = 17;
/// Linhas por bloco: os registros `ROW` do bloco vêm antes das células, como o Excel grava.
const BLOCK_ROWS: usize = 32;

const REC_BOUNDSHEET: u16 = 0x0085;
const REC_EOF: u16 = 0x000A;
const REC_DIMENSIONS: u16 = 0x0200;
const REC_COLINFO: u16 = 0x007D;
const REC_ROW: u16 = 0x0208;
const REC_NUMBER: u16 = 0x0203;
const REC_LABEL: u16 = 0x0204;
const REC_BOOLERR: u16 = 0x0205;

pub struct XlsWriter {
    /// Destino final: o arquivo composto só é montado no fim (`cfb::Stream` não atravessa threads).
    file: Option<File>,
    /// O fluxo `Workbook` vai para um arquivo temporário anônimo (memória constante) e é copiado no fim.
    out: BufWriter<File>,
    header: bool,
    base_name: String,
    columns: Vec<ExportColumn>,
    /// Posição (no fluxo) dos dados do registro `DIMENSIONS`, ajustado no fim.
    dims_pos: u64,
    next_row: u32,
    block_rows: Vec<u8>,
    block_cells: Vec<u8>,
    block_count: usize,
}

fn record(buf: &mut Vec<u8>, id: u16, body: &[u8]) {
    buf.extend_from_slice(&id.to_le_bytes());
    buf.extend_from_slice(&(body.len() as u16).to_le_bytes());
    buf.extend_from_slice(body);
}

/// `XLUnicodeString` com contagem de 16 bits: comprimido (1 byte por caractere) quando tudo cabe em Latin-1; senão
/// UTF-16, e então a contagem é de unidades de 16 bits (um emoji vale duas). `max_units` corta o texto sem partir um
/// par substituto.
fn unicode_string(s: &str, max_units: usize) -> Vec<u8> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    if chars.iter().all(|c| (*c as u32) < 256) {
        let kept = &chars[..chars.len().min(max_units)];
        out.extend_from_slice(&(kept.len() as u16).to_le_bytes());
        out.push(0);
        out.extend(kept.iter().map(|c| *c as u8));
    } else {
        let mut units: Vec<u16> = s.encode_utf16().collect();
        if units.len() > max_units {
            units.truncate(max_units);
            // Cortou entre as duas metades de um par substituto: descarta a metade solta.
            if units.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
                units.pop();
            }
        }
        out.extend_from_slice(&(units.len() as u16).to_le_bytes());
        out.push(1);
        for u in units {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }
    out
}

impl XlsWriter {
    pub fn new(file: File, header: bool, base_name: &str) -> io::Result<Self> {
        Ok(Self {
            file: Some(file),
            out: BufWriter::with_capacity(256 * 1024, tempfile::tempfile()?),
            header,
            base_name: base_name.to_string(),
            columns: Vec::new(),
            dims_pos: 0,
            next_row: 0,
            block_rows: Vec::new(),
            block_cells: Vec::new(),
            block_count: 0,
        })
    }

    fn flush_block(&mut self) -> io::Result<()> {
        self.out.write_all(&self.block_rows)?;
        self.out.write_all(&self.block_cells)?;
        self.block_rows.clear();
        self.block_cells.clear();
        self.block_count = 0;
        Ok(())
    }

    /// Acrescenta uma linha ao bloco: o registro `ROW` e as células (`None` = célula vazia).
    fn push_row(&mut self, cells: &[Option<Cell>], bold: bool) {
        let row = self.next_row as u16;
        let mut body = Vec::with_capacity(16);
        body.extend_from_slice(&row.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&(self.columns.len() as u16).to_le_bytes());
        body.extend_from_slice(&0x00FFu16.to_le_bytes());
        body.extend_from_slice(&[0, 0, 0, 0]);
        body.extend_from_slice(&0x0100u16.to_le_bytes());
        body.extend_from_slice(&0x000Fu16.to_le_bytes());
        record(&mut self.block_rows, REC_ROW, &body);
        let xf = if bold { XF_BOLD } else { XF_PLAIN };
        for (i, cell) in cells.iter().enumerate() {
            let Some(cell) = cell else { continue };
            let mut head = Vec::with_capacity(6);
            head.extend_from_slice(&row.to_le_bytes());
            head.extend_from_slice(&(i as u16).to_le_bytes());
            head.extend_from_slice(&xf.to_le_bytes());
            match cell {
                Cell::Number(n) => {
                    head.extend_from_slice(&n.to_le_bytes());
                    record(&mut self.block_cells, REC_NUMBER, &head);
                }
                Cell::Bool(b) => {
                    head.extend_from_slice(&[u8::from(*b), 0]);
                    record(&mut self.block_cells, REC_BOOLERR, &head);
                }
                Cell::Text(s) => {
                    head.extend_from_slice(&unicode_string(s, MAX_TEXT_CHARS));
                    record(&mut self.block_cells, REC_LABEL, &head);
                }
            }
        }
        self.next_row += 1;
        self.block_count += 1;
    }
}

enum Cell<'a> {
    Number(f64),
    Bool(bool),
    Text(&'a str),
}

impl RowWriter for XlsWriter {
    fn begin(&mut self, columns: &[ExportColumn]) -> io::Result<()> {
        if columns.len() > MAX_COLS {
            return Err(io::Error::other(format!("the .xls format holds at most {MAX_COLS} columns; use .xlsx")));
        }
        self.columns = columns.to_vec();

        // Globais: modelo + BOUNDSHEET (com o deslocamento do BOF da planilha) + EOF.
        let name = sheet_name(&self.base_name, 0);
        let mut bound = Vec::new();
        let name_bytes = unicode_string(&name, 31);
        // `unicode_string` começa com a contagem de 16 bits; o BOUNDSHEET usa uma de 8 bits.
        let (cch, flags_and_chars) = (name_bytes[0], &name_bytes[2..]);
        let boundsheet_body = 4 + 2 + 1 + flags_and_chars.len();
        let sheet_offset = (HEAD.len() + 4 + boundsheet_body + 4) as u32;
        bound.extend_from_slice(&sheet_offset.to_le_bytes());
        bound.extend_from_slice(&[0, 0]); // visível, planilha
        bound.push(cch);
        bound.extend_from_slice(flags_and_chars);
        let mut globals = HEAD.to_vec();
        record(&mut globals, REC_BOUNDSHEET, &bound);
        record(&mut globals, REC_EOF, &[]);
        self.out.write_all(&globals)?;

        // Planilha: BOF..WSBOOL, larguras, DIMENSIONS (ajustado no fim), configuração de página.
        let mut sheet = SHEET_A.to_vec();
        for (i, c) in self.columns.iter().enumerate() {
            let chars = (c.name.chars().count() as f64 + 4.0).clamp(10.0, 40.0);
            let mut body = Vec::with_capacity(12);
            body.extend_from_slice(&(i as u16).to_le_bytes());
            body.extend_from_slice(&(i as u16).to_le_bytes());
            body.extend_from_slice(&((chars * 256.0) as u16).to_le_bytes());
            body.extend_from_slice(&0x000Fu16.to_le_bytes());
            body.extend_from_slice(&[0, 0, 0, 0]);
            record(&mut sheet, REC_COLINFO, &body);
        }
        self.dims_pos = (globals.len() + sheet.len() + 4) as u64;
        record(&mut sheet, REC_DIMENSIONS, &[0u8; 14]);
        sheet.extend_from_slice(SHEET_B);
        self.out.write_all(&sheet)?;

        if self.header {
            let cells: Vec<Option<Cell>> = columns.iter().map(|c| Some(Cell::Text(&c.name))).collect();
            self.push_row(&cells, true);
        }
        Ok(())
    }

    fn row(&mut self, values: &[Option<&str>]) -> io::Result<()> {
        if self.next_row >= MAX_ROWS {
            return Err(io::Error::other(format!("the .xls format holds at most {MAX_ROWS} rows; use .xlsx")));
        }
        let cells: Vec<Option<Cell>> = values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let s = (*v)?;
                let ty = self.columns.get(i).and_then(|c| c.type_name.as_deref());
                Some(if is_bool_type(ty) && matches!(s, "t" | "f") {
                    Cell::Bool(s == "t")
                } else if is_numeric_type(ty) && is_json_number(s) && digits(s) <= 15 {
                    match s.parse::<f64>() {
                        Ok(n) => Cell::Number(n),
                        Err(_) => Cell::Text(s),
                    }
                } else {
                    Cell::Text(s)
                })
            })
            .collect();
        self.push_row(&cells, false);
        if self.block_count >= BLOCK_ROWS {
            self.flush_block()?;
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> io::Result<()> {
        self.flush_block()?;
        self.out.write_all(SHEET_END)?;
        // DIMENSIONS: linhas [0, n), colunas [0, c).
        let mut dims = Vec::with_capacity(14);
        dims.extend_from_slice(&0u32.to_le_bytes());
        dims.extend_from_slice(&self.next_row.to_le_bytes());
        dims.extend_from_slice(&0u16.to_le_bytes());
        dims.extend_from_slice(&(self.columns.len() as u16).to_le_bytes());
        dims.extend_from_slice(&0u16.to_le_bytes());
        self.out.seek(SeekFrom::Start(self.dims_pos))?;
        self.out.write_all(&dims)?;
        let mut temp = self.out.into_inner().map_err(|e| e.into_error())?;
        temp.seek(SeekFrom::Start(0))?;
        let file = self.file.take().ok_or_else(|| io::Error::other("arquivo já fechado"))?;
        let mut comp = cfb::CompoundFile::create_with_version(cfb::Version::V3, file)?;
        let mut stream = comp.create_stream("/Workbook")?;
        io::copy(&mut temp, &mut stream)?;
        drop(stream);
        comp.flush()
    }
}

/// Dígitos significativos do número (sem sinal, ponto e expoente).
fn digits(s: &str) -> usize {
    s.split(['e', 'E']).next().unwrap_or(s).chars().filter(|c| c.is_ascii_digit()).count()
}
