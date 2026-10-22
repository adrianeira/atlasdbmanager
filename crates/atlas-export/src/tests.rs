use super::*;
use std::io::Write;
use std::sync::{Arc, Mutex};

/// Destino em memória que continua legível depois de entregue ao gravador.
#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl Write for Buf {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Buf {
    fn bytes(&self) -> Vec<u8> {
        self.0.lock().unwrap().clone()
    }
    fn text(&self) -> String {
        String::from_utf8(self.bytes()).unwrap()
    }
}

fn col(name: &str, ty: &str) -> ExportColumn {
    ExportColumn { name: name.into(), type_name: Some(ty.into()) }
}

fn run(format: ExportFormat, columns: &[ExportColumn], rows: &[Vec<Option<&str>>]) -> Buf {
    let buf = Buf::default();
    let mut w = create_stream(&format, "\"public\".\"t\"", Box::new(buf.clone())).unwrap();
    w.begin(columns).unwrap();
    for r in rows {
        w.row(r).unwrap();
    }
    w.finish().unwrap();
    buf
}

fn csv(delimiter: &str, qualifier: &str, header: bool, crlf: bool, null_text: &str, encoding: Encoding) -> ExportFormat {
    ExportFormat::Delimited { delimiter: delimiter.into(), qualifier: qualifier.into(), header, crlf, null_text: null_text.into(), encoding }
}

#[test]
fn csv_quotes_only_when_needed() {
    let cols = [col("id", "int4"), col("nome", "text")];
    let rows = vec![
        vec![Some("1"), Some("simples")],
        vec![Some("2"), Some("com, vírgula")],
        vec![Some("3"), Some("diz \"oi\"")],
        vec![Some("4"), Some("duas\nlinhas")],
    ];
    let out = run(csv(",", "\"", true, false, "", Encoding::Utf8), &cols, &rows).text();
    assert_eq!(out, "id,nome\n1,simples\n2,\"com, vírgula\"\n3,\"diz \"\"oi\"\"\"\n4,\"duas\nlinhas\"\n");
}

#[test]
fn csv_distinguishes_null_from_empty_string() {
    let cols = [col("a", "text"), col("b", "text")];
    let rows = vec![vec![None, Some("")]];
    let out = run(csv(";", "\"", false, true, "", Encoding::Utf8), &cols, &rows).text();
    assert_eq!(out, ";\"\"\r\n");
    // Com texto próprio para NULL não há ambiguidade: o vazio não precisa de aspas.
    let out = run(csv(";", "\"", false, false, "NULL", Encoding::Utf8), &cols, &rows).text();
    assert_eq!(out, "NULL;\n");
}

#[test]
fn csv_without_qualifier_writes_raw_and_tab_delimiter_works() {
    let cols = [col("a", "text"), col("b", "text")];
    let rows = vec![vec![Some("x y"), Some("z")]];
    let out = run(csv("\t", "", true, false, "", Encoding::Utf8), &cols, &rows).text();
    assert_eq!(out, "a\tb\nx y\tz\n");
}

#[test]
fn csv_bom_and_windows_1252() {
    let cols = [col("t", "text")];
    let rows = vec![vec![Some("ação é")], vec![Some("😀")]];
    let bom = run(csv(",", "\"", false, false, "", Encoding::Utf8Bom), &cols, &rows).bytes();
    assert_eq!(&bom[..3], &[0xEF, 0xBB, 0xBF]);
    assert!(String::from_utf8(bom[3..].to_vec()).unwrap().starts_with("ação é"));

    let win = run(csv(",", "\"", false, false, "", Encoding::Windows1252), &cols, &rows).bytes();
    // "ação é": a, ç=0xE7, ã=0xE3, o, espaço, é=0xE9; o emoji não existe no Windows-1252.
    assert_eq!(win, [b'a', 0xE7, 0xE3, b'o', b' ', 0xE9, b'\n', b'?', b'\n']);
}

#[test]
fn json_uses_native_types_and_embeds_json() {
    let cols = [col("id", "int8"), col("ok", "bool"), col("total", "numeric"), col("meta", "jsonb"), col("nome", "text")];
    let rows = vec![
        vec![Some("1"), Some("t"), Some("123456789012345.1234"), Some("{\"a\": [1, 2]}"), Some("diz \"oi\"\n")],
        vec![Some("2"), Some("f"), Some("NaN"), None, None],
    ];
    let out = run(ExportFormat::Json { pretty: false }, &cols, &rows).text();
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("JSON válido");
    let a = &parsed[0];
    assert_eq!(a["id"], 1);
    assert_eq!(a["ok"], true);
    assert_eq!(a["meta"]["a"][1], 2);
    assert_eq!(a["nome"], "diz \"oi\"\n");
    // numeric sem perda: o texto original do servidor vai como número.
    assert!(out.contains("\"total\":123456789012345.1234"));
    // NaN não é número JSON: vai como texto. NULL vira null.
    assert_eq!(parsed[1]["total"], "NaN");
    assert_eq!(parsed[1]["ok"], false);
    assert!(parsed[1]["meta"].is_null() && parsed[1]["nome"].is_null());
}

#[test]
fn json_pretty_and_empty_result_are_valid() {
    let cols = [col("id", "int4")];
    let pretty = run(ExportFormat::Json { pretty: true }, &cols, &[vec![Some("1")], vec![Some("2")]]).text();
    assert!(serde_json::from_str::<serde_json::Value>(&pretty).is_ok());
    assert!(pretty.contains("\n  {\n    \"id\": 1\n  }"));
    let empty = run(ExportFormat::Json { pretty: true }, &cols, &[]).text();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&empty).unwrap(), serde_json::json!([]));
}

#[test]
fn sql_batches_rows_and_escapes_literals() {
    let cols = [col("id", "int4"), col("nome", "text"), col("ativo", "bool"), col("total", "numeric")];
    let rows = vec![
        vec![Some("1"), Some("O'Brien"), Some("t"), Some("10.50")],
        vec![Some("2"), None, Some("f"), Some("NaN")],
        vec![Some("3"), Some("c"), None, None],
    ];
    let out = run(ExportFormat::Sql { rows_per_insert: 2 }, &cols, &rows).text();
    assert_eq!(
        out,
        "INSERT INTO \"public\".\"t\" (\"id\", \"nome\", \"ativo\", \"total\") VALUES\n\
         (1, 'O''Brien', TRUE, 10.50),\n\
         (2, NULL, FALSE, 'NaN');\n\
         INSERT INTO \"public\".\"t\" (\"id\", \"nome\", \"ativo\", \"total\") VALUES\n\
         (3, 'c', NULL, NULL);\n"
    );
}

#[test]
fn sql_one_row_per_insert_by_default_and_zero_is_clamped() {
    let cols = [col("id", "int4")];
    let out = run(ExportFormat::Sql { rows_per_insert: 0 }, &cols, &[vec![Some("1")], vec![Some("2")]]).text();
    assert_eq!(out.matches("INSERT INTO").count(), 2);
}

#[test]
fn json_number_grammar() {
    use crate::text::is_json_number as n;
    for ok in ["0", "-1", "10", "0.5", "-0.25", "1e5", "1.5E-3", "123456789012345678901234567890.1234"] {
        assert!(n(ok), "{ok}");
    }
    for bad in ["", "-", ".5", "5.", "01", "NaN", "Infinity", "1e", "$1.00", "1,000", "+1", "1 "] {
        assert!(!n(bad), "{bad}");
    }
}

#[test]
fn xlsx_file_is_a_valid_zip_with_rollover_names() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saida.xlsx");
    let file = std::fs::File::create(&path).unwrap();
    let fmt = ExportFormat::Xlsx { header: true, sheet_name: "Dados: [2026]".into() };
    let mut w = create(&fmt, "", file).unwrap();
    w.begin(&[col("id", "int4"), col("nome", "text"), col("ok", "bool"), col("grande", "numeric")]).unwrap();
    w.row(&[Some("1"), Some("Ana"), Some("t"), Some("12345678901234567890")]).unwrap();
    w.row(&[Some("2"), None, Some("f"), None]).unwrap();
    w.finish().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..2], b"PK");
    assert!(bytes.len() > 1000);
    assert_eq!(crate::xlsx::sheet_name_for_tests("Dados: [2026]", 0), "Dados 2026");
    assert_eq!(crate::xlsx::sheet_name_for_tests(&"x".repeat(40), 1).chars().count(), 31);
    assert!(crate::xlsx::sheet_name_for_tests(&"x".repeat(40), 1).ends_with(" 2"));
}

#[test]
fn format_deserializes_from_the_ui_shape() {
    let f: ExportFormat = serde_json::from_str(
        r#"{"kind":"delimited","delimiter":";","qualifier":"\"","header":true,"crlf":true,"nullText":"","encoding":"utf8Bom"}"#,
    )
    .unwrap();
    assert!(matches!(f, ExportFormat::Delimited { encoding: Encoding::Utf8Bom, crlf: true, .. }));
    let f: ExportFormat = serde_json::from_str(r#"{"kind":"sql","rowsPerInsert":50}"#).unwrap();
    assert!(matches!(f, ExportFormat::Sql { rows_per_insert: 50 }));
    let f: ExportFormat = serde_json::from_str(r#"{"kind":"xlsx","header":false,"sheetName":"Dados"}"#).unwrap();
    assert!(matches!(f, ExportFormat::Xlsx { header: false, .. }));
}
