//! Gravação de resultados em arquivo, linha a linha (streaming): a memória não cresce com o
//! tamanho do resultado. Os valores chegam em texto, exatamente como o servidor os formata.

mod dbf;
mod delimited;
mod html;
mod json;
mod sql;
mod text;
mod xls;
mod xlsx;
mod xml;

use serde::Deserialize;
use std::fs::File;
use std::io;

/// Coluna do resultado. `type_name` é o nome do tipo no servidor (ex.: `int4`, `bool`), quando conhecido.
#[derive(Debug, Clone)]
pub struct ExportColumn {
    pub name: String,
    pub type_name: Option<String>,
}

/// Formato e opções, no mesmo desenho do assistente da interface.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ExportFormat {
    /// CSV / TXT: texto delimitado.
    Delimited {
        /// Separador de campos (um ou mais caracteres; `"\t"` para tabulação).
        delimiter: String,
        /// Qualificador de texto (`"` ou `'`); vazio = nenhum.
        qualifier: String,
        header: bool,
        /// `true` = CRLF, `false` = LF.
        crlf: bool,
        /// Texto gravado no lugar de NULL.
        null_text: String,
        encoding: Encoding,
    },
    Json {
        pretty: bool,
    },
    /// Comandos `INSERT`; `rows_per_insert` linhas por comando.
    Sql {
        rows_per_insert: u32,
    },
    Xlsx {
        header: bool,
        sheet_name: String,
    },
    /// Excel 97-2003 (BIFF8): até 65.536 linhas e 256 colunas.
    Xls {
        header: bool,
        sheet_name: String,
    },
    /// dBase III: campos de largura fixa (texto em Windows-1252).
    Dbf {
        /// Casas decimais dos campos numéricos (`numeric`, `float`).
        decimals: u32,
        /// Largura dos campos de texto (1 a 254 bytes; o excedente é cortado).
        text_length: u32,
    },
    /// `<table name="...<row><coluna>valor</coluna></row></table>`.
    Xml,
    /// Página HTML com uma tabela.
    Html {
        header: bool,
    },
}

/// Como o arquivo se identifica: `sql` é o destino do `INSERT` (`"schema"."tabela"`, já com aspas) e
/// `title` o nome mostrado no XML e no HTML.
pub struct WriterTarget<'a> {
    pub sql: &'a str,
    pub title: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Encoding {
    Utf8,
    /// UTF-8 com marca de ordem de bytes (o Excel reconhece os acentos).
    Utf8Bom,
    /// Caracteres que não existem no Windows-1252 viram `?`.
    Windows1252,
}

/// Quem recebe as linhas. `values[i]` é o valor da coluna `i` (`None` = NULL).
pub trait RowWriter: Send {
    fn begin(&mut self, columns: &[ExportColumn]) -> io::Result<()>;
    fn row(&mut self, values: &[Option<&str>]) -> io::Result<()>;
    /// Termina o arquivo (rodapé, descarga do buffer, fechamento do XLSX).
    fn finish(self: Box<Self>) -> io::Result<()>;
}

/// Cria o gravador do formato.
pub fn create(format: &ExportFormat, target: &WriterTarget, file: File) -> io::Result<Box<dyn RowWriter>> {
    Ok(match format {
        ExportFormat::Xlsx { header, sheet_name } => Box::new(xlsx::XlsxWriter::new(file, *header, sheet_name)),
        ExportFormat::Xls { header, sheet_name } => Box::new(xls::XlsWriter::new(file, *header, sheet_name)?),
        ExportFormat::Dbf { decimals, text_length } => Box::new(dbf::DbfWriter::new(file, *decimals, *text_length)),
        other => create_stream(other, target, Box::new(io::BufWriter::with_capacity(256 * 1024, file)))?,
    })
}

/// Gravador para os formatos de texto sobre qualquer destino (usado também nos testes).
pub fn create_stream(format: &ExportFormat, target: &WriterTarget, out: Box<dyn io::Write + Send>) -> io::Result<Box<dyn RowWriter>> {
    Ok(match format {
        ExportFormat::Delimited { delimiter, qualifier, header, crlf, null_text, encoding } => Box::new(delimited::DelimitedWriter::new(
            out,
            delimiter.clone(),
            qualifier.chars().next(),
            *header,
            *crlf,
            null_text.clone(),
            *encoding,
        )),
        ExportFormat::Json { pretty } => Box::new(json::JsonWriter::new(out, *pretty)),
        ExportFormat::Sql { rows_per_insert } => Box::new(sql::SqlWriter::new(out, target.sql.to_string(), (*rows_per_insert).max(1) as usize)),
        ExportFormat::Xml => Box::new(xml::XmlWriter::new(out, target.title.to_string())),
        ExportFormat::Html { header } => Box::new(html::HtmlWriter::new(out, target.title.to_string(), *header)),
        ExportFormat::Xlsx { .. } | ExportFormat::Xls { .. } | ExportFormat::Dbf { .. } => {
            return Err(io::Error::other("este formato precisa de um arquivo"));
        }
    })
}

/// Extensão sugerida para o formato.
pub fn default_extension(format: &ExportFormat) -> &'static str {
    match format {
        ExportFormat::Delimited { .. } => "csv",
        ExportFormat::Json { .. } => "json",
        ExportFormat::Sql { .. } => "sql",
        ExportFormat::Xlsx { .. } => "xlsx",
        ExportFormat::Xls { .. } => "xls",
        ExportFormat::Dbf { .. } => "dbf",
        ExportFormat::Xml => "xml",
        ExportFormat::Html { .. } => "html",
    }
}

#[cfg(test)]
mod tests;
