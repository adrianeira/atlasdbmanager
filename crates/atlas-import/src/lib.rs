//! Leitura de arquivos para o assistente de importação: TXT/CSV, JSON, XML, Excel (`.xls`/`.xlsx`) e dBase.
//!
//! Toda fonte devolve as linhas como texto (`None` = NULL). A conversão para o tipo da coluna de destino e a
//! inferência de tipos para tabelas novas ficam em [`convert`] e [`infer`]. Nada aqui fala com o banco.

pub mod convert;
mod dbf;
mod delimited;
mod excel;
pub mod infer;
mod json;
mod xml;

use serde::Deserialize;
use std::io;
use std::path::{Path, PathBuf};

/// Como ler um arquivo de texto delimitado.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SourceEncoding {
    /// UTF-8 (a marca BOM, se houver, é reconhecida e descartada).
    Utf8,
    Windows1252,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ImportFormat {
    /// TXT/CSV: `delimiter` pode ter mais de um caractere; `qualifier` vazio = sem aspas de texto.
    Delimited { delimiter: String, qualifier: String, encoding: SourceEncoding },
    /// Lista de objetos (ou um objeto com uma lista dentro).
    Json,
    /// Registros = elementos filhos do elemento raiz; campos = elementos filhos (e atributos) de cada registro.
    Xml,
    Excel,
    Dbf,
}

/// Quais linhas da planilha/texto são o cabeçalho e os dados (1 = primeira linha; 0 = não se aplica).
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SourceOptions {
    /// Linha com os nomes dos campos; 0 = não há (os campos viram F1, F2...).
    pub field_name_row: u32,
    pub first_data_row: u32,
    /// Última linha de dados; 0 = até o fim.
    pub last_data_row: u32,
}

impl Default for SourceOptions {
    fn default() -> Self {
        Self { field_name_row: 1, first_data_row: 2, last_data_row: 0 }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSpec {
    pub path: PathBuf,
    pub format: ImportFormat,
    /// Tabela do arquivo (planilha do Excel; nos demais formatos só há uma).
    #[serde(default)]
    pub table: String,
    #[serde(default)]
    pub options: SourceOptions,
}

/// Quem entrega as linhas: sempre `columns().len()` valores por linha.
pub trait RowSource: Send {
    fn columns(&self) -> &[String];
    fn next_row(&mut self) -> io::Result<Option<Vec<Option<String>>>>;
}

/// Linhas "cruas" de uma grade (texto delimitado, planilha), a partir da linha 1; cada uma pode ter largura diferente.
pub(crate) trait RawRows: Send {
    fn next_raw(&mut self) -> io::Result<Option<Vec<Option<String>>>>;
}

/// Aplica as linhas do cabeçalho e dos dados a uma grade e a transforma em [`RowSource`].
struct GridSource {
    raw: Box<dyn RawRows>,
    columns: Vec<String>,
    /// Primeira linha de dados já lida (quando foi preciso espiá-la para saber a largura).
    pending: Option<Vec<Option<String>>>,
    next_number: u32,
    last: u32,
}

fn io_err(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

impl GridSource {
    fn open(mut raw: Box<dyn RawRows>, o: SourceOptions) -> io::Result<Self> {
        let first = o.first_data_row.max(1);
        let header_row = if o.field_name_row == 0 { None } else { Some(o.field_name_row) };
        let mut number = 1u32;
        let mut names: Option<Vec<Option<String>>> = None;
        let mut pending = None;
        // Lê até a primeira linha de dados, guardando o cabeçalho no caminho.
        while pending.is_none() {
            let Some(row) = raw.next_raw()? else { break };
            if header_row == Some(number) {
                names = Some(row);
            } else if number >= first {
                pending = Some(row);
            }
            number += 1;
        }
        let width = names.as_ref().map(|n| n.len()).max(pending.as_ref().map(|p| p.len())).unwrap_or(0);
        let width = width.max(pending.as_ref().map_or(0, |p| p.len()));
        let columns = (0..width)
            .map(|i| match names.as_ref().and_then(|n| n.get(i)).and_then(|c| c.as_deref()).map(str::trim) {
                Some(name) if !name.is_empty() => name.to_string(),
                _ => format!("F{}", i + 1),
            })
            .collect();
        Ok(Self { raw, columns, pending, next_number: first, last: o.last_data_row })
    }
}

impl RowSource for GridSource {
    fn columns(&self) -> &[String] {
        &self.columns
    }

    fn next_row(&mut self) -> io::Result<Option<Vec<Option<String>>>> {
        if self.last != 0 && self.next_number > self.last {
            return Ok(None);
        }
        let row = match self.pending.take() {
            Some(r) => Some(r),
            None => self.raw.next_raw()?,
        };
        let Some(mut row) = row else { return Ok(None) };
        self.next_number += 1;
        row.resize(self.columns.len(), None);
        Ok(Some(row))
    }
}

/// Primeiras linhas e nomes dos campos (para o mapeamento e para escolher o tipo de uma tabela nova).
#[derive(Debug, Clone)]
pub struct Preview {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "dados".into())
}

/// Tabelas que o arquivo oferece: as planilhas do Excel; nos outros formatos, uma (o nome do arquivo).
pub fn list_tables(path: &Path, format: &ImportFormat) -> io::Result<Vec<String>> {
    match format {
        ImportFormat::Excel => excel::sheet_names(path),
        _ => Ok(vec![stem(path)]),
    }
}

pub fn open(spec: &SourceSpec) -> io::Result<Box<dyn RowSource>> {
    let o = spec.options;
    Ok(match &spec.format {
        ImportFormat::Delimited { delimiter, qualifier, encoding } => {
            let raw = delimited::DelimitedRows::open(&spec.path, delimiter, qualifier, *encoding)?;
            Box::new(GridSource::open(Box::new(raw), o)?)
        }
        ImportFormat::Excel => Box::new(GridSource::open(Box::new(excel::SheetRows::open(&spec.path, &spec.table)?), o)?),
        ImportFormat::Json => Box::new(json::JsonRows::open(&spec.path)?),
        ImportFormat::Xml => Box::new(xml::XmlRows::open(&spec.path)?),
        ImportFormat::Dbf => Box::new(dbf::DbfRows::open(&spec.path)?),
    })
}

/// Nomes dos campos e até `limit` linhas.
pub fn preview(spec: &SourceSpec, limit: usize) -> io::Result<Preview> {
    let mut src = open(spec)?;
    let mut rows = Vec::new();
    while rows.len() < limit {
        match src.next_row()? {
            Some(r) => rows.push(r),
            None => break,
        }
    }
    Ok(Preview { columns: src.columns().to_vec(), rows })
}

pub(crate) use io_err as invalid;
