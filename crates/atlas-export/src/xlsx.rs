//! Excel (.xlsx) em modo de memória constante: cada linha vai para o arquivo ao ser escrita.

use crate::text::{is_bool_type, is_json_number, is_numeric_type};
use crate::{ExportColumn, RowWriter};
use rust_xlsxwriter::{Format, Workbook};
use std::fs::File;
use std::io::{self, BufWriter};

/// Limite do formato: 1.048.576 linhas por planilha (a última é a de índice 1.048.575).
const MAX_ROWS: u32 = 1_048_576;
/// Limite de caracteres por célula.
const MAX_CELL_CHARS: usize = 32_767;

pub struct XlsxWriter {
    file: Option<File>,
    wb: Workbook,
    header: bool,
    base_name: String,
    columns: Vec<ExportColumn>,
    sheet: usize,
    next_row: u32,
}

fn io_err(e: rust_xlsxwriter::XlsxError) -> io::Error {
    io::Error::other(e.to_string())
}

/// Nome de planilha válido: até 31 caracteres, sem `[ ] : * ? / \`.
fn sheet_name(base: &str, index: usize) -> String {
    let mut name: String = base.chars().filter(|c| !"[]:*?/\\".contains(*c)).collect();
    name = name.trim().trim_matches('\'').to_string();
    if name.is_empty() {
        name = "Sheet".into();
    }
    if index > 0 {
        let suffix = format!(" {}", index + 1);
        let keep = 31 - suffix.chars().count();
        name = name.chars().take(keep).collect::<String>() + &suffix;
    }
    name.chars().take(31).collect()
}

#[cfg(test)]
pub fn sheet_name_for_tests(base: &str, index: usize) -> String {
    sheet_name(base, index)
}

impl XlsxWriter {
    pub fn new(file: File, header: bool, base_name: &str) -> Self {
        Self { file: Some(file), wb: Workbook::new(), header, base_name: base_name.to_string(), columns: Vec::new(), sheet: 0, next_row: 0 }
    }

    /// Abre uma planilha nova (a primeira, ou a seguinte quando a atual enche) com o cabeçalho.
    fn open_sheet(&mut self) -> io::Result<()> {
        let index = if self.wb.worksheets().is_empty() { 0 } else { self.sheet + 1 };
        let name = sheet_name(&self.base_name, index);
        let ws = self.wb.add_worksheet_with_constant_memory().map_err(io_err)?;
        ws.set_name(&name).map_err(io_err)?;
        self.sheet = index;
        self.next_row = 0;
        if self.header {
            let bold = Format::new().set_bold();
            for (i, c) in self.columns.iter().enumerate() {
                let col = i as u16;
                let width = (c.name.chars().count() as f64 + 4.0).clamp(10.0, 40.0);
                ws.set_column_width(col, width).map_err(io_err)?;
                ws.write_string_with_format(0, col, truncate(&c.name), &bold).map_err(io_err)?;
            }
            ws.set_freeze_panes(1, 0).map_err(io_err)?;
            self.next_row = 1;
        }
        Ok(())
    }
}

fn truncate(s: &str) -> &str {
    match s.char_indices().nth(MAX_CELL_CHARS) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

impl RowWriter for XlsxWriter {
    fn begin(&mut self, columns: &[ExportColumn]) -> io::Result<()> {
        self.columns = columns.to_vec();
        self.open_sheet()
    }

    fn row(&mut self, values: &[Option<&str>]) -> io::Result<()> {
        if self.next_row >= MAX_ROWS {
            self.open_sheet()?;
        }
        let row = self.next_row;
        let ws = self.wb.worksheet_from_index(self.sheet).map_err(io_err)?;
        for (i, v) in values.iter().enumerate() {
            let Some(s) = v else { continue };
            let col = i as u16;
            let ty = self.columns.get(i).and_then(|c| c.type_name.as_deref());
            if is_bool_type(ty) && matches!(*s, "t" | "f") {
                ws.write_boolean(row, col, *s == "t").map_err(io_err)?;
            } else if is_numeric_type(ty) && is_json_number(s) && digits(s) <= 15 {
                // Até 15 dígitos o double do Excel é exato; acima disso vai como texto para não perder precisão.
                match s.parse::<f64>() {
                    Ok(n) => ws.write_number(row, col, n).map_err(io_err)?,
                    Err(_) => ws.write_string(row, col, truncate(s)).map_err(io_err)?,
                };
            } else {
                ws.write_string(row, col, truncate(s)).map_err(io_err)?;
            }
        }
        self.next_row += 1;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> io::Result<()> {
        if self.wb.worksheets().is_empty() {
            self.open_sheet()?;
        }
        let file = self.file.take().ok_or_else(|| io::Error::other("arquivo já fechado"))?;
        self.wb.save_to_writer(BufWriter::new(file)).map_err(io_err)
    }
}

/// Dígitos significativos do número (sem sinal, ponto e expoente).
fn digits(s: &str) -> usize {
    s.split(['e', 'E']).next().unwrap_or(s).chars().filter(|c| c.is_ascii_digit()).count()
}
