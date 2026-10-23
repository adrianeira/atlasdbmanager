//! Ida e volta: o que os gravadores de exportação do Atlas escrevem, os leitores de importação entendem.

use atlas_export::{Encoding, ExportColumn, ExportFormat, WriterTarget, create};
use atlas_import::{ImportFormat, SourceEncoding, SourceOptions, SourceSpec, list_tables, open, preview};
use std::fs::File;
use std::path::{Path, PathBuf};

fn columns() -> Vec<ExportColumn> {
    let c = |n: &str, t: &str| ExportColumn { name: n.into(), type_name: Some(t.into()) };
    vec![c("id", "int4"), c("nome", "text"), c("ativo", "bool"), c("valor", "numeric"), c("criado", "timestamptz"), c("meta", "jsonb")]
}

fn rows() -> Vec<Vec<Option<&'static str>>> {
    vec![
        vec![Some("1"), Some("Ana Souza"), Some("t"), Some("1500.5"), Some("2026-01-15 07:30:00-03"), Some("{\"tags\": [\"a\", \"b\"]}")],
        vec![Some("2"), Some("João \"Zé\", da Silva\nlinha 2"), Some("f"), None, Some("2026-02-20 20:59:59.123456-03"), None],
        vec![Some("3"), Some("açúcar & café <b>"), None, Some("0"), None, Some("{}")],
    ]
}

fn write(format: ExportFormat, dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let mut w = create(&format, &WriterTarget { sql: "\"public\".\"clientes\"", title: "clientes" }, File::create(&path).unwrap()).unwrap();
    w.begin(&columns()).unwrap();
    for r in rows() {
        w.row(&r).unwrap();
    }
    w.finish().unwrap();
    path
}

fn spec(path: &Path, format: ImportFormat, table: &str) -> SourceSpec {
    SourceSpec { path: path.to_path_buf(), format, table: table.into(), options: SourceOptions::default() }
}

fn read(spec: &SourceSpec) -> (Vec<String>, Vec<Vec<Option<String>>>) {
    let p = preview(spec, 100).unwrap();
    (p.columns, p.rows)
}

fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

#[test]
fn csv_with_bom_quotes_and_newlines() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        ExportFormat::Delimited { delimiter: ";".into(), qualifier: "\"".into(), header: true, crlf: true, null_text: "".into(), encoding: Encoding::Utf8Bom },
        dir.path(),
        "a.csv",
    );
    let (cols, rows) = read(&spec(&path, ImportFormat::Delimited { delimiter: ";".into(), qualifier: "\"".into(), encoding: SourceEncoding::Utf8 }, ""));
    assert_eq!(cols, ["id", "nome", "ativo", "valor", "criado", "meta"]);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0], [s("1"), s("Ana Souza"), s("t"), s("1500.5"), s("2026-01-15 07:30:00-03"), s("{\"tags\": [\"a\", \"b\"]}")]);
    assert_eq!(rows[1][1], s("João \"Zé\", da Silva\nlinha 2"));
    assert_eq!(rows[1][3], s(""), "NULL exportado como vazio volta vazio");
    assert_eq!(rows[2][1], s("açúcar & café <b>"));
}

#[test]
fn txt_tab_without_header_windows_1252_and_row_options() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        ExportFormat::Delimited { delimiter: "\t".into(), qualifier: "".into(), header: false, crlf: false, null_text: "NULL".into(), encoding: Encoding::Windows1252 },
        dir.path(),
        "a.txt",
    );
    let format = ImportFormat::Delimited { delimiter: "\t".into(), qualifier: "".into(), encoding: SourceEncoding::Windows1252 };
    // Sem cabeçalho: campos F1.., dados desde a linha 1.
    let mut sp = spec(&path, format, "");
    sp.options = SourceOptions { field_name_row: 0, first_data_row: 1, last_data_row: 0 };
    let (cols, rows) = read(&sp);
    assert_eq!(cols[..3], ["F1", "F2", "F3"]);
    assert_eq!(rows[0][1], s("Ana Souza"));
    assert_eq!(rows[0][3], s("1500.5"));
    // Última linha de dados: só as duas primeiras.
    sp.options.last_data_row = 2;
    assert_eq!(read(&sp).1.len(), 2);
    // Primeira linha de dados 2: pula a primeira.
    sp.options = SourceOptions { field_name_row: 0, first_data_row: 2, last_data_row: 0 };
    assert_eq!(read(&sp).1[0][0], s("2"));
}

#[test]
fn json_keeps_native_types_and_nested_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(ExportFormat::Json { pretty: true }, dir.path(), "a.json");
    let (cols, rows) = read(&spec(&path, ImportFormat::Json, ""));
    assert_eq!(cols, ["id", "nome", "ativo", "valor", "criado", "meta"]);
    assert_eq!(rows[0][0], s("1"));
    assert_eq!(rows[0][2], s("true"));
    assert_eq!(rows[0][3], s("1500.5"));
    assert_eq!(rows[0][5], s("{\"tags\":[\"a\",\"b\"]}"), "jsonb embutido volta como JSON compacto");
    assert_eq!(rows[1][3], None, "null do JSON é NULL");
    assert_eq!(rows[1][1], s("João \"Zé\", da Silva\nlinha 2"));
    assert_eq!(rows[2][2], None);
}

#[test]
fn xml_round_trips_nulls_entities_and_original_names() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(ExportFormat::Xml, dir.path(), "a.xml");
    let (cols, rows) = read(&spec(&path, ImportFormat::Xml, ""));
    assert_eq!(cols, ["id", "nome", "ativo", "valor", "criado", "meta"]);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0][1], s("Ana Souza"));
    assert_eq!(rows[1][3], None, "null=\"true\" é NULL");
    assert_eq!(rows[2][1], s("açúcar & café <b>"), "entidades escapadas voltam como estavam");
    assert_eq!(rows[1][1], s("João \"Zé\", da Silva\nlinha 2"));
    // XML comum: atributos do registro também são campos.
    let plain = dir.path().join("b.xml");
    std::fs::write(&plain, "<?xml version=\"1.0\"?><lista><item id=\"7\"><nome>X</nome><nome2><![CDATA[a < b]]></nome2></item><item id=\"8\"><nome>Y</nome></item></lista>").unwrap();
    let (cols, rows) = read(&spec(&plain, ImportFormat::Xml, ""));
    assert_eq!(cols, ["id", "nome", "nome2"]);
    assert_eq!(rows[0], [s("7"), s("X"), s("a < b")]);
    assert_eq!(rows[1], [s("8"), s("Y"), None]);
}

#[test]
fn excel_xlsx_and_xls_come_back_typed() {
    let dir = tempfile::tempdir().unwrap();
    for (name, format) in [
        ("a.xlsx", ExportFormat::Xlsx { header: true, sheet_name: "Clientes".into() }),
        ("a.xls", ExportFormat::Xls { header: true, sheet_name: "Clientes".into() }),
    ] {
        let path = write(format, dir.path(), name);
        assert_eq!(list_tables(&path, &ImportFormat::Excel).unwrap(), ["Clientes"], "{name}");
        let (cols, rows) = read(&spec(&path, ImportFormat::Excel, "Clientes"));
        assert_eq!(cols, ["id", "nome", "ativo", "valor", "criado", "meta"], "{name}");
        assert_eq!(rows[0][0], s("1"), "{name}: número inteiro sem .0");
        assert_eq!(rows[0][2], s("true"), "{name}");
        assert_eq!(rows[0][3], s("1500.5"), "{name}");
        assert_eq!(rows[0][4], s("2026-01-15 07:30:00-03"), "{name}: texto fica texto");
        assert_eq!(rows[1][3], None, "{name}: célula vazia é NULL");
        assert_eq!(rows[2][1], s("açúcar & café <b>"), "{name}");
        // Linha dos nomes e primeira de dados configuráveis.
        let mut sp = spec(&path, ImportFormat::Excel, "Clientes");
        sp.options = SourceOptions { field_name_row: 0, first_data_row: 1, last_data_row: 2 };
        let (cols, rows) = read(&sp);
        assert_eq!(cols[0], "F1", "{name}");
        assert_eq!(rows.len(), 2, "{name}: cabeçalho + 1ª linha");
        assert_eq!(rows[0][1], s("nome"), "{name}");
    }
}

#[test]
fn dbf_comes_back_with_types_and_cut_field_names() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(ExportFormat::Dbf { decimals: 2, text_length: 60 }, dir.path(), "a.dbf");
    let (cols, rows) = read(&spec(&path, ImportFormat::Dbf, ""));
    assert_eq!(cols, ["ID", "NOME", "ATIVO", "VALOR", "CRIADO", "META"]);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0][0], s("1"));
    assert_eq!(rows[0][1], s("Ana Souza"));
    assert_eq!(rows[0][2], s("true"));
    assert_eq!(rows[0][3], s("1500.50"));
    assert_eq!(rows[1][2], s("false"));
    assert_eq!(rows[1][3], None, "campo numérico em branco é NULL");
    assert_eq!(rows[2][2], None, "lógico '?' é NULL");
    assert_eq!(rows[2][1], s("açúcar & café <b>"));
}

#[test]
fn errors_are_readable() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("x.json");
    std::fs::write(&bad, "{ not json").unwrap();
    assert!(open(&spec(&bad, ImportFormat::Json, "")).err().unwrap().to_string().contains("JSON inválido"));
    std::fs::write(&bad, "42").unwrap();
    assert!(open(&spec(&bad, ImportFormat::Json, "")).is_err());
    std::fs::write(&bad, "isto não é dbf").unwrap();
    assert!(open(&spec(&bad, ImportFormat::Dbf, "")).is_err());
    assert!(open(&spec(&dir.path().join("nao-existe.csv"), ImportFormat::Delimited { delimiter: ",".into(), qualifier: "\"".into(), encoding: SourceEncoding::Utf8 }, "")).is_err());
    std::fs::write(&bad, "<a/>").unwrap();
    assert!(open(&spec(&bad, ImportFormat::Xml, "")).is_err());
}
