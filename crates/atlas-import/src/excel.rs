//! Excel (`.xls` e `.xlsx`) pelo `calamine`: uma planilha vira uma grade de linhas em texto.

use crate::{RawRows, io_err};
use calamine::{Data, Range, Reader, open_workbook_auto};
use std::io;
use std::path::Path;

pub(crate) fn sheet_names(path: &Path) -> io::Result<Vec<String>> {
    let wb = open_workbook_auto(path).map_err(|e| io_err(format!("não foi possível abrir a planilha: {e}")))?;
    Ok(wb.sheet_names())
}

/// Valor da célula como o texto que a pessoa vê: inteiros sem `.0`, datas em ISO, booleanos como `true`/`false`.
fn cell_text(c: &Data) -> Option<String> {
    match c {
        Data::Empty | Data::Error(_) => None,
        Data::String(s) => Some(s.clone()),
        Data::Int(i) => Some(i.to_string()),
        Data::Float(f) => Some(if f.fract() == 0.0 && f.abs() < 1e15 { format!("{}", *f as i64) } else { f.to_string() }),
        Data::Bool(b) => Some(b.to_string()),
        Data::DateTime(d) => Some(d.as_datetime().map(|dt| dt.to_string()).unwrap_or_else(|| d.as_f64().to_string())),
        Data::DateTimeIso(s) | Data::DurationIso(s) => Some(s.clone()),
    }
}

pub(crate) struct SheetRows {
    range: Range<Data>,
    /// Linha (começando em 0) que o próximo `next_raw` entrega.
    next: usize,
    /// Primeira linha usada da planilha e primeira coluna usada: as linhas antes dela saem vazias.
    start_row: usize,
    start_col: usize,
    height: usize,
}

impl SheetRows {
    pub(crate) fn open(path: &Path, sheet: &str) -> io::Result<Self> {
        let mut wb = open_workbook_auto(path).map_err(|e| io_err(format!("não foi possível abrir a planilha: {e}")))?;
        let name = if sheet.is_empty() { wb.sheet_names().into_iter().next().unwrap_or_default() } else { sheet.to_string() };
        let range = wb.worksheet_range(&name).map_err(|e| io_err(format!("planilha \"{name}\": {e}")))?;
        let (start_row, start_col) = range.start().map(|(r, c)| (r as usize, c as usize)).unwrap_or((0, 0));
        let height = range.height();
        Ok(Self { range, next: 0, start_row, start_col, height })
    }
}

impl RawRows for SheetRows {
    fn next_raw(&mut self) -> io::Result<Option<Vec<Option<String>>>> {
        // Número de linhas da grade a partir da linha 1 da planilha: as vazias antes da área usada + a área usada.
        if self.next >= self.start_row + self.height {
            return Ok(None);
        }
        let row = if self.next < self.start_row {
            Vec::new()
        } else {
            let cells: Vec<Option<String>> = self.range.rows().nth(self.next - self.start_row).map(|r| r.iter().map(cell_text).collect()).unwrap_or_default();
            let mut out = vec![None; self.start_col];
            out.extend(cells);
            out
        };
        self.next += 1;
        Ok(Some(row))
    }
}
