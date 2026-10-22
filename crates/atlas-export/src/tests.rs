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
    let mut w = create_stream(&format, &WriterTarget { sql: "\"public\".\"t\"", title: "t" }, Box::new(buf.clone())).unwrap();
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
    let mut w = create(&fmt, &WriterTarget { sql: "", title: "t" }, file).unwrap();
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

#[test]
fn xml_escapes_values_and_fixes_element_names() {
    let cols = [col("id", "int4"), col("nome do cliente", "text"), col("1a", "text")];
    let rows = vec![vec![Some("1"), Some("a < b & \"c\"\u{1}"), None]];
    let out = run(ExportFormat::Xml, &cols, &rows).text();
    assert_eq!(
        out,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<table name=\"t\">\n  <row>\n    <id>1</id>\n    \
         <nome_do_cliente name=\"nome do cliente\">a &lt; b &amp; &quot;c&quot;</nome_do_cliente>\n    \
         <_1a name=\"1a\" null=\"true\"/>\n  </row>\n</table>\n"
    );
}

#[test]
fn html_has_header_numeric_alignment_and_escaping() {
    let cols = [col("id", "int4"), col("nome", "text")];
    let rows = vec![vec![Some("1"), Some("<b>x</b> & y")], vec![Some("2"), None]];
    let out = run(ExportFormat::Html { header: true }, &cols, &rows).text();
    assert!(out.starts_with("<!DOCTYPE html>"));
    assert!(out.contains("<th>id</th><th>nome</th>"));
    assert!(out.contains("<tr><td class=\"num\">1</td><td>&lt;b&gt;x&lt;/b&gt; &amp; y</td></tr>"));
    assert!(out.contains("<tr><td class=\"num\">2</td><td></td></tr>"));
    let no_header = run(ExportFormat::Html { header: false }, &cols, &rows).text();
    assert!(!no_header.contains("<thead>"));
}

#[test]
fn xml_and_html_formats_deserialize_from_the_ui_shape() {
    assert!(matches!(serde_json::from_str::<ExportFormat>(r#"{"kind":"xml"}"#).unwrap(), ExportFormat::Xml));
    assert!(matches!(serde_json::from_str::<ExportFormat>(r#"{"kind":"html","header":true}"#).unwrap(), ExportFormat::Html { header: true }));
}

fn write_file(format: ExportFormat, columns: &[ExportColumn], rows: &[Vec<Option<&str>>], name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    let mut w = create(&format, &WriterTarget { sql: "\"public\".\"t\"", title: "t" }, File::create(&path).unwrap()).unwrap();
    w.begin(columns).unwrap();
    for r in rows {
        w.row(r).unwrap();
    }
    w.finish().unwrap();
    (dir, path)
}

#[test]
fn xls_round_trips_text_numbers_booleans_and_nulls() {
    use calamine::{Data, Reader, open_workbook_auto};
    let cols = [col("id", "int4"), col("nome", "text"), col("ativo", "bool"), col("valor", "numeric")];
    let rows = vec![
        vec![Some("1"), Some("ação"), Some("t"), Some("1500.5")],
        vec![Some("2"), Some("emoji ✓ e 日本 😀"), Some("f"), None],
        vec![Some("3"), None, None, Some("12345678901234567890")],
    ];
    let (_dir, path) = write_file(ExportFormat::Xls { header: true, sheet_name: "Clientes".into() }, &cols, &rows, "t.xls");
    let mut wb = open_workbook_auto(&path).expect("opens as xls");
    assert_eq!(wb.sheet_names(), vec!["Clientes".to_string()]);
    let range = wb.worksheet_range("Clientes").unwrap();
    assert_eq!(range.height(), 4);
    assert_eq!(range.width(), 4);
    let cell = |r: usize, c: usize| range.get((r, c)).cloned().unwrap_or(Data::Empty);
    assert_eq!(cell(0, 1), Data::String("nome".into()));
    assert_eq!(cell(1, 0), Data::Float(1.0));
    assert_eq!(cell(1, 1), Data::String("ação".into()));
    assert_eq!(cell(1, 2), Data::Bool(true));
    assert_eq!(cell(1, 3), Data::Float(1500.5));
    assert_eq!(cell(2, 1), Data::String("emoji ✓ e 日本 😀".into()));
    assert_eq!(cell(2, 2), Data::Bool(false));
    assert_eq!(cell(2, 3), Data::Empty);
    assert_eq!(cell(3, 1), Data::Empty);
    // Acima de 15 dígitos vai como texto para não perder precisão.
    assert_eq!(cell(3, 3), Data::String("12345678901234567890".into()));
}

#[test]
fn xls_without_header_and_with_many_rows_spans_blocks() {
    use calamine::{Data, Reader, open_workbook_auto};
    let cols = [col("n", "int4")];
    let texts: Vec<String> = (0..100).map(|i| i.to_string()).collect();
    let rows: Vec<Vec<Option<&str>>> = texts.iter().map(|t| vec![Some(t.as_str())]).collect();
    let (_dir, path) = write_file(ExportFormat::Xls { header: false, sheet_name: "x".into() }, &cols, &rows, "t.xls");
    let mut wb = open_workbook_auto(&path).unwrap();
    let range = wb.worksheet_range("x").unwrap();
    assert_eq!(range.height(), 100);
    assert_eq!(range.get((99, 0)), Some(&Data::Float(99.0)));
}

#[test]
fn xls_refuses_what_the_format_cannot_hold() {
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        create(&ExportFormat::Xls { header: false, sheet_name: "x".into() }, &WriterTarget { sql: "", title: "t" }, File::create(dir.path().join(name)).unwrap()).unwrap()
    };
    let wide: Vec<ExportColumn> = (0..257).map(|i| col(&format!("c{i}"), "text")).collect();
    assert!(make("a.xls").begin(&wide).is_err());
    let mut w = make("b.xls");
    w.begin(&[col("a", "text")]).unwrap();
    for _ in 0..65_536 {
        w.row(&[Some("x")]).unwrap();
    }
    let err = w.row(&[Some("x")]).expect_err("65537th row");
    assert!(err.to_string().contains("65536"), "{err}");
}

/// Leitor mínimo de dBase III para conferir o que foi gravado.
struct Dbf {
    count: u32,
    fields: Vec<(String, u8, usize, usize)>,
    records: Vec<Vec<String>>,
}

fn read_dbf(path: &std::path::Path) -> Dbf {
    let b = std::fs::read(path).unwrap();
    assert_eq!(b[0], 0x03);
    let count = u32::from_le_bytes(b[4..8].try_into().unwrap());
    let header_len = u16::from_le_bytes(b[8..10].try_into().unwrap()) as usize;
    let record_len = u16::from_le_bytes(b[10..12].try_into().unwrap()) as usize;
    assert_eq!(b[header_len - 1], 0x0D);
    let mut fields = Vec::new();
    let mut at = 32;
    while b[at] != 0x0D {
        let name = String::from_utf8_lossy(&b[at..at + 11]).trim_end_matches('\0').to_string();
        fields.push((name, b[at + 11], b[at + 16] as usize, b[at + 17] as usize));
        at += 32;
    }
    assert_eq!(1 + fields.iter().map(|f| f.2).sum::<usize>(), record_len);
    assert_eq!(b.len(), header_len + count as usize * record_len + 1);
    assert_eq!(*b.last().unwrap(), 0x1A);
    let mut records = Vec::new();
    for r in 0..count as usize {
        let rec = &b[header_len + r * record_len..header_len + (r + 1) * record_len];
        assert_eq!(rec[0], b' ');
        let mut off = 1;
        let mut vals = Vec::new();
        for f in &fields {
            // Windows-1252 coincide com Latin-1 para o que o teste usa.
            vals.push(rec[off..off + f.2].iter().map(|c| *c as char).collect::<String>());
            off += f.2;
        }
        records.push(vals);
    }
    Dbf { count, fields, records }
}

#[test]
fn dbf_writes_fixed_width_typed_fields() {
    let cols = [
        col("id", "int4"),
        col("Nome do cliente", "text"),
        col("ativo", "bool"),
        col("valor", "numeric"),
        col("nasc", "date"),
        col("criado", "timestamptz"),
        col("id_externo", "uuid"),
    ];
    let long = "x".repeat(300);
    let rows = vec![
        vec![Some("7"), Some("João ação"), Some("t"), Some("1500.5"), Some("2026-01-15"), Some("2026-01-15 07:30:00-03"), Some("11111111-1111-4111-8111-111111111111")],
        vec![Some("-12"), None, Some("f"), Some("-0.005"), None, None, None],
        vec![None, Some(long.as_str()), None, None, Some("infinity"), None, None],
    ];
    let (_dir, path) = write_file(ExportFormat::Dbf { decimals: 2, text_length: 20 }, &cols, &rows, "t.dbf");
    let d = read_dbf(&path);
    assert_eq!(d.count, 3);
    let names: Vec<&str> = d.fields.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(names, ["ID", "NOME_DO_CL", "ATIVO", "VALOR", "NASC", "CRIADO", "ID_EXTERNO"]);
    let kinds: Vec<u8> = d.fields.iter().map(|f| f.1).collect();
    assert_eq!(kinds, b"NCLNDCC".to_vec());
    assert_eq!(d.fields[3], ("VALOR".into(), b'N', 20, 2));
    assert_eq!(d.fields[1].2, 20);
    let r0 = &d.records[0];
    assert_eq!(r0[0], "          7");
    assert_eq!(r0[1].trim_end(), "João ação");
    assert_eq!(r0[2], "T");
    assert_eq!(r0[3].trim(), "1500.50");
    assert_eq!(r0[4], "20260115");
    let r1 = &d.records[1];
    assert_eq!(r1[0].trim(), "-12");
    assert_eq!(r1[1].trim(), "");
    assert_eq!(r1[2], "F");
    assert_eq!(r1[3].trim(), "-0.01");
    assert_eq!(r1[4], "        ");
    let r2 = &d.records[2];
    assert_eq!(r2[0].trim(), "");
    assert_eq!(r2[1], "x".repeat(20));
    assert_eq!(r2[2], "?");
    assert_eq!(r2[4], "        ");
}

#[test]
fn dbf_rejects_numbers_that_do_not_fit() {
    let dir = tempfile::tempdir().unwrap();
    let mut w = create(&ExportFormat::Dbf { decimals: 4, text_length: 50 }, &WriterTarget { sql: "", title: "t" }, File::create(dir.path().join("a.dbf")).unwrap()).unwrap();
    w.begin(&[col("v", "numeric")]).unwrap();
    let err = w.row(&[Some("123456789012345678.12345")]).expect_err("too wide");
    assert!(err.to_string().contains("\"v\""), "{err}");
    // Casa decimal arredondada que ainda cabe é aceita.
    w.row(&[Some("123456789012345.12345")]).unwrap();
}
