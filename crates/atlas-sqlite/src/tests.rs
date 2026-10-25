use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

/// Um arquivo SQLite novo, só deste teste, com a estrutura e os dados de exemplo.
fn sample() -> (SqliteSession, PathBuf) {
    let path = std::env::temp_dir().join(format!("atlas-sqlite-test-{}-{}.db", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_file(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE clients (id INTEGER PRIMARY KEY, name VARCHAR(20) NOT NULL, total NUMERIC(10,2) DEFAULT 0, active BOOLEAN, photo BLOB, code TEXT UNIQUE);
         CREATE TABLE orders (id INTEGER PRIMARY KEY, client_id INTEGER REFERENCES clients(id) ON DELETE CASCADE, note TEXT);
         CREATE INDEX orders_client ON orders(client_id);
         CREATE TABLE loose (a INTEGER, b TEXT);
         CREATE VIEW big_clients AS SELECT * FROM clients WHERE total > 100;
         CREATE TRIGGER clients_log AFTER INSERT ON clients BEGIN SELECT 1; END;
         INSERT INTO clients (name, total, active, photo, code) VALUES ('Ana', 150.5, 1, x'0aff', 'A'), ('Bia', 20, 0, NULL, 'B'), ('Cris', NULL, NULL, NULL, 'C');
         INSERT INTO orders (client_id, note) VALUES (1, 'first'), (1, NULL);
         INSERT INTO loose VALUES (7,'d'),(7,'d'),(7,'d'),(8,'k');",
    )
    .unwrap();
    drop(conn);
    (SqliteSession::open(path.to_str().unwrap()).unwrap(), path)
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
}

fn rows(out: &QueryOutcome, st: usize) -> Vec<Vec<Option<String>>> {
    out.statements[st].result.as_ref().unwrap().rows.clone()
}

#[test]
fn open_rejects_missing_files_and_files_that_are_not_sqlite() {
    let err = SqliteSession::open("C:/does/not/exist.db").err().unwrap();
    assert_eq!(err.kind, ErrorKind::DatabaseNotFound);
    assert_eq!(SqliteSession::open("  ").err().unwrap().kind, ErrorKind::InvalidInput);
    let junk = std::env::temp_dir().join(format!("atlas-sqlite-junk-{}.db", std::process::id()));
    std::fs::write(&junk, "this is not a database, only text that is long enough to be read as a header....").unwrap();
    assert!(SqliteSession::open(junk.to_str().unwrap()).is_err());
    cleanup(&junk);
}

#[test]
fn lists_tables_and_views_without_internal_tables() {
    let (s, p) = sample();
    let objs = s.list_objects().unwrap();
    assert_eq!(objs.tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["clients", "loose", "orders"]);
    assert_eq!(objs.views.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["big_clients"]);
    assert!(s.server_info().version_num >= 3_000_000);
    cleanup(&p);
}

#[test]
fn table_detail_reads_columns_keys_indexes_foreign_keys_and_triggers() {
    let (s, p) = sample();
    let d = s.table_detail("clients").unwrap();
    let names: Vec<_> = d.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "name", "total", "active", "photo", "code"]);
    let id = &d.columns[0];
    assert_eq!(id.key, Some(1));
    assert_eq!(id.identity.as_deref(), Some("d"), "INTEGER PRIMARY KEY numbera sozinho");
    let name = &d.columns[1];
    assert!(name.not_null && name.length == 20 && name.key.is_none());
    let total = &d.columns[2];
    assert_eq!((total.length, total.decimal, total.default.as_deref()), (10, 2, Some("0")));
    assert!(d.indexes.iter().any(|i| i.unique && i.fields == "code"));
    assert!(d.constraints.iter().any(|c| c.kind == "u" && c.fields == "code"));
    assert_eq!(d.triggers.len(), 1);
    assert_eq!(d.triggers[0].timing, "AFTER");

    let o = s.table_detail("orders").unwrap();
    let fk = o.constraints.iter().find(|c| c.kind == "f").unwrap();
    assert_eq!((fk.fields.as_str(), fk.ref_table.as_deref(), fk.ref_fields.as_deref(), fk.on_delete.as_deref()), ("client_id", Some("clients"), Some("id"), Some("CASCADE")));
    assert!(o.indexes.iter().any(|i| i.name == "orders_client" && i.fields == "client_id"));

    assert!(s.table_detail("nada").is_err());
    // Uma view também tem colunas (e nenhuma chave: fica só para leitura).
    let v = s.table_detail("big_clients").unwrap();
    assert!(v.columns.iter().all(|c| c.key.is_none()));
    cleanup(&p);
}

#[test]
fn rows_come_as_text_the_way_the_grid_reads_them() {
    let (s, p) = sample();
    let (sql, out) = s.table_rows("clients", &[OrderBy { column: "id".into(), descending: false }], None, Some(10), 0, 1000).unwrap();
    assert!(sql.starts_with("SELECT * FROM \"clients\" ORDER BY \"id\" ASC LIMIT 10 OFFSET 0"));
    let r = rows(&out, 0);
    assert_eq!(r[0], [Some("1".into()), Some("Ana".into()), Some("150.5".into()), Some("t".into()), Some("\\x0aff".into()), Some("A".into())]);
    assert_eq!(r[1][3].as_deref(), Some("f"));
    assert_eq!(r[2][2], None, "NULL continua NULL");
    let cols = &out.statements[0].result.as_ref().unwrap().columns;
    assert_eq!(cols[1].type_name.as_deref(), Some("VARCHAR(20)"));
    cleanup(&p);
}

#[test]
fn filter_order_limit_offset_and_count() {
    let (s, p) = sample();
    let filter = TableFilter { expr: "total IS NOT NULL".into() };
    assert_eq!(s.table_count("clients", Some(&filter)).unwrap(), 2);
    assert_eq!(s.table_count("clients", None).unwrap(), 3);
    let (_, out) = s.table_rows("clients", &[OrderBy { column: "name".into(), descending: true }], Some(&filter), Some(1), 1, 1000).unwrap();
    assert_eq!(rows(&out, 0)[0][1].as_deref(), Some("Ana"));
    let (_, none) = s.table_rows("clients", &[], None, None, 0, 2).unwrap();
    assert!(none.statements[0].result.as_ref().unwrap().truncated, "mais linhas que o limite de memória");
    cleanup(&p);
}

#[test]
fn execute_runs_several_statements_and_reports_each() {
    let (s, p) = sample();
    let out = s.execute("INSERT INTO loose VALUES (9,'z'); SELECT a, b FROM loose WHERE a = 9; SELECT 1 AS um", 100).unwrap();
    assert_eq!(out.statements.len(), 3);
    assert_eq!(out.statements[0].rows_affected, Some(1));
    assert!(out.statements[0].result.is_none());
    assert_eq!(rows(&out, 1), [[Some("9".into()), Some("z".into())]]);
    assert_eq!(out.statements[2].result.as_ref().unwrap().columns[0].name, "um");
    assert!(out.statements[1].sql.contains("SELECT a, b"));
    cleanup(&p);
}

#[test]
fn sql_errors_are_mapped_with_a_category() {
    let (s, p) = sample();
    let syntax = s.execute("SELEC 1", 10).err().unwrap();
    assert_eq!(syntax.kind, ErrorKind::SqlSyntaxError);
    let missing = s.execute("SELECT * FROM nao_existe", 10).err().unwrap();
    assert!(missing.message.contains("nao_existe"));
    let unique = s.execute("INSERT INTO clients (name, code) VALUES ('X', 'A')", 10).err().unwrap();
    assert_eq!(unique.kind, ErrorKind::ConstraintViolation);
    cleanup(&p);
}

#[test]
fn apply_is_all_or_nothing_and_checks_the_row_counts() {
    let (s, p) = sample();
    let ok = PlannedStatement { sql: "UPDATE clients SET name = ?1 WHERE id = ?2".into(), params: vec![Some("Ana B".into()), Some("1".into())], expect_one_row: true, ..Default::default() };
    assert_eq!(s.apply(&[ok]).unwrap(), [1]);
    assert_eq!(rows(&s.execute("SELECT name FROM clients WHERE id = 1", 5).unwrap(), 0)[0][0].as_deref(), Some("Ana B"));

    // O segundo comando não acha linha: o primeiro também é desfeito.
    let a = PlannedStatement { sql: "UPDATE clients SET name = 'Z' WHERE id = 2".into(), expect_one_row: true, ..Default::default() };
    let b = PlannedStatement { sql: "UPDATE clients SET name = 'Y' WHERE id = 999".into(), expect_one_row: true, ..Default::default() };
    let err = s.apply(&[a, b]).err().unwrap();
    assert_eq!(err.kind, ErrorKind::ConcurrencyConflict);
    assert_eq!(rows(&s.execute("SELECT name FROM clients WHERE id = 2", 5).unwrap(), 0)[0][0].as_deref(), Some("Bia"));
    cleanup(&p);
}

#[test]
fn keyless_tables_change_only_as_many_identical_rows_as_marked() {
    let (s, p) = sample();
    // Três linhas (7,'d'): marcar duas apaga duas.
    let del = PlannedStatement {
        sql: "DELETE FROM \"loose\" WHERE rowid IN (SELECT rowid FROM \"loose\" WHERE \"a\" = ?1 AND \"b\" = ?2 LIMIT 2)".into(),
        params: vec![Some("7".into()), Some("d".into())],
        expect_rows: Some(2),
        ..Default::default()
    };
    assert_eq!(s.apply(&[del]).unwrap(), [2]);
    assert_eq!(rows(&s.execute("SELECT count(*) FROM loose WHERE a = 7", 5).unwrap(), 0)[0][0].as_deref(), Some("1"));
    // Pedir duas de uma só é conflito, e nada muda.
    let too_many = PlannedStatement {
        sql: "DELETE FROM \"loose\" WHERE rowid IN (SELECT rowid FROM \"loose\" WHERE \"a\" = ?1 LIMIT 2)".into(),
        params: vec![Some("7".into())],
        expect_rows: Some(2),
        ..Default::default()
    };
    assert_eq!(s.apply(&[too_many]).err().unwrap().kind, ErrorKind::ConcurrencyConflict);
    assert_eq!(rows(&s.execute("SELECT count(*) FROM loose WHERE a = 7", 5).unwrap(), 0)[0][0].as_deref(), Some("1"));
    cleanup(&p);
}

#[test]
fn manual_transaction_commits_or_rolls_back() {
    let (s, p) = sample();
    let tx = s.begin().unwrap();
    tx.execute("INSERT INTO loose VALUES (50, 'tx')", 5).unwrap();
    // Dentro da transação a linha existe; fora (outra conexão) ainda não.
    assert_eq!(tx.table_count("loose", Some(&TableFilter { expr: "a = 50".into() })).unwrap(), 1);
    assert_eq!(s.table_count("loose", Some(&TableFilter { expr: "a = 50".into() })).unwrap(), 0);
    tx.finish(false).unwrap();
    assert_eq!(s.table_count("loose", Some(&TableFilter { expr: "a = 50".into() })).unwrap(), 0);

    let tx = s.begin().unwrap();
    tx.apply(&[PlannedStatement { sql: "INSERT INTO loose VALUES (51, 'ok')".into(), ..Default::default() }]).unwrap();
    tx.finish(true).unwrap();
    assert_eq!(s.table_count("loose", Some(&TableFilter { expr: "a = 51".into() })).unwrap(), 1);
    cleanup(&p);
}

#[test]
fn foreign_keys_are_enforced() {
    let (s, p) = sample();
    let err = s.execute("INSERT INTO orders (client_id, note) VALUES (999, 'x')", 5).err().unwrap();
    assert_eq!(err.kind, ErrorKind::ConstraintViolation);
    cleanup(&p);
}

#[test]
fn check_constraints_are_read_from_the_create_statement() {
    let checks = parse_checks("CREATE TABLE t (a INT CHECK (a > 0), b TEXT, CONSTRAINT \"b_ok\" CHECK (b IN ('(x)', 'y') AND length(b) < (3 + 1)), note TEXT DEFAULT 'check (z)')");
    assert_eq!(checks.len(), 2);
    assert_eq!(checks[0], (None, "a > 0".to_string()));
    assert_eq!(checks[1], (Some("b_ok".to_string()), "b IN ('(x)', 'y') AND length(b) < (3 + 1)".to_string()));
}

/// O que o designer faz para alterar a tabela: criar a nova, copiar, trocar. As chaves estrangeiras de quem aponta para ela seguem valendo.
#[test]
fn rebuilding_a_table_keeps_data_and_keeps_foreign_keys_working() {
    let (s, path) = sample();
    let plan = |sql: &str| PlannedStatement { sql: sql.into(), params: vec![], expect_one_row: false, expect_rows: None };
    s.apply(&[
        plan("PRAGMA defer_foreign_keys = ON"),
        plan("CREATE TABLE \"__atlas_new\" (\"id\" INTEGER PRIMARY KEY AUTOINCREMENT, \"name\" VARCHAR(20) NOT NULL, \"extra\" TEXT)"),
        plan("INSERT INTO \"__atlas_new\" (\"id\", \"name\") SELECT \"id\", \"name\" FROM \"clients\""),
        plan("DROP TABLE \"clients\""),
        // Com views apontando para a tabela, o RENAME comum recusa (a view ficaria sem tabela por um instante).
        plan("PRAGMA legacy_alter_table = ON"),
        plan("ALTER TABLE \"__atlas_new\" RENAME TO \"clients\""),
        plan("PRAGMA legacy_alter_table = OFF"),
        plan("CREATE TRIGGER clients_log AFTER INSERT ON clients BEGIN SELECT 1; END"),
    ])
    .unwrap();
    let d = s.table_detail("clients").unwrap();
    assert_eq!(d.columns.len(), 3);
    assert_eq!(d.triggers.len(), 1);
    assert_eq!(s.table_count("clients", None).unwrap(), 3);
    // `orders` ainda aponta para `clients` e a chave continua sendo conferida.
    assert!(s.execute("INSERT INTO orders (client_id) VALUES (999)", 10).is_err());
    assert!(s.execute("INSERT INTO orders (client_id) VALUES (2)", 10).is_ok());
    // O AUTOINCREMENT numera sozinho.
    s.execute("INSERT INTO clients (name) VALUES ('Dani')", 10).unwrap();
    let out = s.execute("SELECT max(id) FROM clients", 10).unwrap();
    assert_eq!(out.statements.last().unwrap().result.as_ref().unwrap().rows[0][0].as_deref(), Some("4"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn fk_labels_use_the_description_column_of_the_referenced_table() {
    let (s, path) = sample();
    let out = s.fk_labels("clients", "id", &["1".into(), "3".into(), "99".into()]).unwrap();
    assert_eq!(out.label_column.as_deref(), Some("name"));
    assert_eq!(out.labels, vec![["1".to_string(), "Ana".to_string()], ["3".to_string(), "Cris".to_string()]]);
    // Tabela sem coluna de texto que sirva: sem rótulo.
    assert!(s.fk_labels("loose", "a", &["7".into()]).unwrap().label_column.is_some_and(|c| c == "b"));
    assert!(s.fk_labels("orders", "id", &["1".into()]).unwrap().label_column.as_deref() == Some("note"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn data_profile_counts_nulls_distinct_unique_and_top_values() {
    let (s, path) = sample();
    let cols = vec!["id".to_string(), "total".to_string(), "photo".to_string()];
    let p = s.data_profile("clients", &cols, &[], None, None).unwrap();
    assert_eq!(p.rows, 3);
    let total = &p.columns[1];
    assert_eq!((total.nulls, total.distinct, total.unique), (1, Some(2), Some(2)));
    assert_eq!((total.min.as_deref(), total.max.as_deref()), (Some("20"), Some("150.5")));
    assert_eq!(p.columns[2].nulls, 2);
    // Sobre as linhas que o limite escolhe.
    let limited = s.data_profile("loose", &["a".to_string()], &[], None, Some(2)).unwrap();
    assert_eq!(limited.rows, 2);
    assert_eq!(limited.columns[0].top[0].count, 2);
    let _ = std::fs::remove_file(path);
}

#[test]
fn table_profile_counts_the_structure_and_the_rows() {
    let (s, path) = sample();
    let p = s.table_profile("clients").unwrap();
    assert_eq!((p.columns, p.primary_keys, p.uniques, p.triggers, p.estimated_rows), (6, 1, 1, 1, 3));
    let o = s.table_profile("orders").unwrap();
    assert_eq!((o.foreign_keys, o.indexes), (1, 1));
    let _ = std::fs::remove_file(path);
}

fn find_opts(text: &str, mode: find::FindMode) -> FindOptions {
    FindOptions { text: text.into(), schema: None, mode, case_insensitive: true, tables: true, views: true, matviews: false, functions: false, indexes: true, triggers: true }
}

#[test]
fn find_in_structure_reports_names_and_view_bodies() {
    let (s, path) = sample();
    let r = s.find(&find_opts("client", find::FindMode::Contains)).unwrap();
    let names: Vec<(&str, &str)> = r.objects.iter().map(|o| (o.kind.as_str(), o.name.as_str())).collect();
    // `clients` (nome), `orders` (campo client_id e índice orders_client), view big_clients (nome e corpo) e o gatilho clients_log.
    assert_eq!(names, vec![("table", "clients"), ("table", "orders"), ("view", "big_clients")]);
    let orders = &r.objects[1];
    assert_eq!(orders.matches.iter().map(|m| m.what.as_str()).collect::<Vec<_>>(), vec!["fieldName", "indexName"]);
    assert!(r.objects[2].matches.iter().any(|m| m.what == "viewBody"));
    assert!(s.find(&find_opts("zzz", find::FindMode::Contains)).unwrap().objects.is_empty());
    let _ = std::fs::remove_file(path);
}

#[test]
fn find_in_data_counts_the_rows_and_gives_a_query_that_runs() {
    let (s, path) = sample();
    let r = s.find_data(&find_opts("a", find::FindMode::Whole), &|| false).unwrap();
    assert_eq!(r.total, 3);
    assert!(r.tables.is_empty() || r.tables.iter().all(|t| t.count > 0));
    let r = s.find_data(&find_opts("cr", find::FindMode::Prefix), &|| false).unwrap();
    assert_eq!(r.tables.len(), 1);
    assert_eq!((r.tables[0].table.as_str(), r.tables[0].count), ("clients", 1));
    // A consulta devolvida roda na própria sessão (REGEXP é definida em toda conexão).
    let out = s.execute(&r.tables[0].sql, 10).unwrap();
    assert_eq!(out.statements[0].result.as_ref().unwrap().rows.len(), 1);
    assert!(s.find_data(&find_opts("cr", find::FindMode::Prefix), &|| true).unwrap().cancelled);
    let _ = std::fs::remove_file(path);
}

#[test]
fn schema_diagram_lists_columns_keys_and_relations() {
    let (s, path) = sample();
    let d = s.schema_diagram().unwrap();
    let names: Vec<&str> = d.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["clients", "loose", "orders"]);
    let orders = d.tables.iter().find(|t| t.name == "orders").unwrap();
    assert!(orders.columns[0].is_pk && orders.columns[1].is_fk && !orders.columns[2].is_fk);
    assert_eq!(d.relations.len(), 1);
    let r = &d.relations[0];
    assert_eq!((r.table.as_str(), r.ref_table.as_str(), r.columns.clone(), r.ref_columns.clone(), r.required), ("orders", "clients", vec!["client_id".to_string()], vec!["id".to_string()], false));
    let _ = std::fs::remove_file(path);
}

#[allow(clippy::too_many_arguments)]
fn import_rows(s: &SqliteSession, table: &str, cols: &[(&str, &str)], keys: &[usize], mode: ImportMode, create: bool, continue_on_error: bool, rows: Vec<Vec<Option<&str>>>) -> ImportStats {
    let spec = ImportSpec {
        table: table.into(),
        create: create.then(|| cols.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect()),
        columns: cols.iter().map(|(n, t)| TargetColumn { name: n.to_string(), declared_type: declared_type(t) }).collect(),
        key_columns: keys.to_vec(),
        mode,
        extended_insert: true,
        max_statement_bytes: 1024 * 1024,
        continue_on_error,
    };
    let mut it = rows.into_iter();
    let mut feed = move || match it.next() {
        Some(r) => ImportFeed::Row(r.into_iter().map(|v| v.map(String::from)).collect()),
        None => ImportFeed::End,
    };
    let mut log = |_line: String| {};
    let mut progress = |_s: &ImportStats| {};
    s.import(&spec, &mut feed, ImportHooks { log: &mut log, progress: &mut progress, cancelled: &|| false }).unwrap()
}

fn scalar(s: &SqliteSession, sql: &str) -> String {
    s.execute(sql, 10).unwrap().statements[0].result.as_ref().unwrap().rows[0][0].clone().unwrap_or_default()
}

#[test]
fn import_creates_a_table_and_runs_every_mode() {
    let (s, path) = sample();
    let cols = [("id", "integer"), ("name", "text"), ("ok", "boolean")];
    let st = import_rows(&s, "imp", &cols, &[], ImportMode::Append, true, false, vec![vec![Some("1"), Some("a"), Some("true")], vec![Some("2"), Some("b"), Some("false")], vec![Some("3"), None, Some("true")]]);
    assert_eq!((st.processed, st.added, st.errors, st.rolled_back), (3, 3, 0, false));
    assert_eq!(scalar(&s, "SELECT sum(ok) FROM imp"), "2");
    // Atualiza pela chave e acrescenta o que falta.
    let st = import_rows(&s, "imp", &cols, &[0], ImportMode::AppendUpdate, false, false, vec![vec![Some("2"), Some("B"), Some("true")], vec![Some("9"), Some("z"), Some("false")]]);
    assert_eq!((st.updated, st.added), (1, 1));
    assert_eq!(scalar(&s, "SELECT name FROM imp WHERE id = 2"), "B");
    // Só o que não existe.
    let st = import_rows(&s, "imp", &cols, &[0], ImportMode::AppendNoUpdate, false, false, vec![vec![Some("1"), Some("x"), None], vec![Some("10"), Some("y"), None]]);
    assert_eq!(st.added, 1);
    assert_eq!(scalar(&s, "SELECT name FROM imp WHERE id = 1"), "a");
    // Exclui pelas chaves.
    let st = import_rows(&s, "imp", &cols, &[0], ImportMode::Delete, false, false, vec![vec![Some("9"), None, None], vec![Some("10"), None, None]]);
    assert_eq!(st.deleted, 2);
    // Copy apaga tudo e repovoa.
    let st = import_rows(&s, "imp", &cols, &[], ImportMode::Copy, false, false, vec![vec![Some("5"), Some("only"), None]]);
    assert_eq!((st.deleted, st.added), (3, 1));
    assert_eq!(scalar(&s, "SELECT count(*) FROM imp"), "1");
    let _ = std::fs::remove_file(path);
}

#[test]
fn import_error_rolls_back_unless_it_continues() {
    let (s, path) = sample();
    let cols = [("id", "integer"), ("name", "text")];
    // `clients.id` é chave: o segundo `1` quebra.
    let bad = || vec![vec![Some("50"), Some("a")], vec![Some("1"), Some("dup")], vec![Some("51"), Some("c")]];
    let cols_clients = [("id", "integer"), ("name", "text")];
    let _ = cols;
    let st = import_rows(&s, "clients", &cols_clients, &[], ImportMode::Append, false, false, bad());
    assert!(st.rolled_back);
    assert_eq!(scalar(&s, "SELECT count(*) FROM clients"), "3");
    let st = import_rows(&s, "clients", &cols_clients, &[], ImportMode::Append, false, true, bad());
    assert_eq!((st.added, st.errors, st.rolled_back), (2, 1, false));
    assert_eq!(scalar(&s, "SELECT count(*) FROM clients"), "5");
    let _ = std::fs::remove_file(path);
}

#[test]
fn backup_copies_the_file_without_the_excluded_objects_and_restore_brings_it_back() {
    let (s, path) = sample();
    let dest = std::env::temp_dir().join(format!("atlas-sqlite-backup-{}.dump", std::process::id()));
    let _ = std::fs::remove_file(&dest);
    let mut lines = Vec::new();
    s.backup_to(&dest, &["main.loose".to_string()], &mut |l| lines.push(l)).unwrap();
    assert!(lines.iter().any(|l| l.contains("main.clients")) && !lines.iter().any(|l| l.contains("main.loose")));
    let copy = SqliteSession::open(dest.to_str().unwrap()).unwrap();
    let names: Vec<String> = copy.list_objects().unwrap().tables.iter().map(|o| o.name.clone()).collect();
    assert_eq!(names, vec!["clients".to_string(), "orders".to_string()]);
    assert_eq!(copy.table_count("clients", None).unwrap(), 3);
    // Muda o original e restaura: volta ao que o backup tinha.
    s.execute("DELETE FROM clients", 10).unwrap();
    assert_eq!(s.table_count("clients", None).unwrap(), 0);
    s.restore_from(&dest).unwrap();
    assert_eq!(s.table_count("clients", None).unwrap(), 3);
    assert!(s.restore_from(Path::new("Cargo.toml")).is_err());
    drop(copy);
    let _ = std::fs::remove_file(dest);
    let _ = std::fs::remove_file(path);
}
