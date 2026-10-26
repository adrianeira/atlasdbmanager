//! Testes contra um Firebird de verdade. Rodam só com as variáveis `ATLAS_TEST_FIREBIRD_*` (senão passam sem fazer nada).

use super::*;
use atlas_core::{ConnectionParams, OrderBy, Secret, SslMode, TableFilter};

/// Os testes ao vivo mexem no catálogo (criam e apagam tabelas): um de cada vez, para não brigar por travas de metadados.
static LIVE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn params() -> Option<ConnectionParams> {
    Some(ConnectionParams {
        host: std::env::var("ATLAS_TEST_FIREBIRD_HOST").ok()?,
        port: std::env::var("ATLAS_TEST_FIREBIRD_PORT").ok()?.parse().ok()?,
        database: std::env::var("ATLAS_TEST_FIREBIRD_DATABASE").ok()?,
        user: std::env::var("ATLAS_TEST_FIREBIRD_USER").ok()?,
        password: Secret::new(std::env::var("ATLAS_TEST_FIREBIRD_PASSWORD").ok()?),
        ssl_mode: SslMode::Disable,
        connect_timeout_secs: 15,
        ssh: None,
    })
}

#[test]
fn quoting_doubles_the_quote() {
    assert_eq!(quote_ident("CLIENTS"), "\"CLIENTS\"");
    assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
}

#[test]
fn select_orders_and_filters() {
    let o = [OrderBy { column: "ID".into(), descending: true }];
    let f = TableFilter { expr: "QTY > 1".into() };
    assert_eq!(select_sql("T", &o, Some(&f)), "SELECT * FROM \"T\" WHERE QTY > 1 ORDER BY \"ID\" DESC");
    assert_eq!(version_number("4.0.2"), 4_000_002);
    assert_eq!(version_number("2.5.3"), 2_005_003);
}

#[test]
fn live_server_lists_reads_and_writes() {
    let Some(p) = params() else { return };
    let _guard = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    let s = FirebirdSession::open(Target::from_params(&p)).unwrap();
    assert!(s.version_num >= 2_005_000);
    let name = format!("ATLAS_T_{}", std::process::id());
    let q = quote_ident(&name);
    let _ = s.execute(&format!("DROP TABLE {q}"), 10);
    s.execute(
        &format!(
            "CREATE TABLE {q} (ID INTEGER NOT NULL PRIMARY KEY, NAME VARCHAR(30) DEFAULT 'x' NOT NULL, TOTAL NUMERIC(10,2), BORN DATE, AT_TIME TIME, STAMP TIMESTAMP, BIG BIGINT, RATIO DOUBLE PRECISION, NOTE BLOB SUB_TYPE TEXT, CODE CHAR(4), CONSTRAINT {c} CHECK (TOTAL >= 0))",
            c = quote_ident(&format!("CK_{name}"))
        ),
        10,
    )
    .unwrap();
    let out = s
        .apply(&[
            PlannedStatement {
                sql: format!("INSERT INTO {q} (ID, NAME, TOTAL, BORN, AT_TIME, STAMP, BIG, RATIO, NOTE, CODE) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"),
                params: ["1", "Ana", "10.50", "2024-03-05", "13:45:10", "2024-03-05 13:45:10", "123456789012", "1.5", "um texto", "ab"].map(|v| Some(v.to_string())).to_vec(),
                ..Default::default()
            },
            PlannedStatement { sql: format!("INSERT INTO {q} (ID, NAME) VALUES (?, ?)"), params: vec![Some("2".into()), Some("Bia".into())], ..Default::default() },
        ])
        .unwrap();
    assert_eq!(out, vec![1, 1]);
    let (_, rows) = s.table_rows(&name, &[OrderBy { column: "ID".into(), descending: false }], None, Some(10), 0, 100).unwrap();
    let set = rows.statements[0].result.as_ref().unwrap();
    assert_eq!(set.columns.len(), 10);
    assert_eq!(set.rows[0][0].as_deref(), Some("1"));
    assert_eq!(set.rows[0][2].as_deref(), Some("10.50"));
    assert_eq!(set.rows[0][3].as_deref(), Some("2024-03-05"));
    assert_eq!(set.rows[0][4].as_deref(), Some("13:45:10"));
    assert_eq!(set.rows[0][5].as_deref(), Some("2024-03-05 13:45:10"));
    assert_eq!(set.rows[0][6].as_deref(), Some("123456789012"));
    assert_eq!(set.rows[0][7].as_deref(), Some("1.5"));
    assert_eq!(set.rows[0][8].as_deref(), Some("um texto"));
    assert_eq!(set.rows[0][9].as_deref(), Some("ab  "));
    assert_eq!(set.rows[1][2], None);
    assert_eq!(s.table_count(&name, None).unwrap(), 2);
    // Página: a segunda linha.
    let (_, page) = s.table_rows(&name, &[OrderBy { column: "ID".into(), descending: false }], None, Some(1), 1, 100).unwrap();
    assert_eq!(page.statements[0].result.as_ref().unwrap().rows[0][0].as_deref(), Some("2"));
    // Resultado vazio ainda traz as colunas e os tipos.
    let empty = s.execute(&format!("SELECT ID, TOTAL, BORN FROM {q} WHERE ID < 0"), 10).unwrap();
    let cols = &empty.statements[0].result.as_ref().unwrap().columns;
    assert_eq!(cols.iter().map(|c| c.type_name.clone().unwrap()).collect::<Vec<_>>(), vec!["integer", "numeric", "date"]);

    let d = s.table_detail(&name).unwrap();
    assert_eq!(d.columns.len(), 10);
    assert_eq!(d.columns[0].key, Some(1));
    assert!(d.columns[0].not_null);
    assert_eq!(d.columns[1].default.as_deref(), Some("'x'"));
    assert_eq!(d.columns[2].formatted_type, "numeric(10,2)");
    assert_eq!(d.columns[1].formatted_type, "varchar(30)");
    assert!(d.constraints.iter().any(|c| c.kind == "c" && c.definition.contains("TOTAL >= 0")));
    assert!(d.constraints.iter().any(|c| c.kind == "p" && c.fields == "ID"));
    assert!(s.list_objects().unwrap().tables.iter().any(|t| t.name == name));

    // Transação manual: o que não foi gravado some.
    let tx = s.begin().unwrap();
    tx.execute(&format!("DELETE FROM {q}"), 10).unwrap();
    assert_eq!(tx.table_count(&name, None).unwrap(), 0);
    tx.finish(false).unwrap();
    assert_eq!(s.table_count(&name, None).unwrap(), 2);

    // Vários comandos num texto; o erro do segundo é mostrado.
    let e = s.execute(&format!("SELECT 1 FROM RDB$DATABASE; INSERT INTO {q} (ID, NAME) VALUES (1, 'dup')"), 10).unwrap_err();
    assert_eq!(e.kind, ErrorKind::ConstraintViolation);
    s.execute(&format!("DROP TABLE {q}"), 10).unwrap();
}

#[test]
fn live_auto_increment_by_generator_and_trigger() {
    let Some(p) = params() else { return };
    let _guard = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    let s = FirebirdSession::open(Target::from_params(&p)).unwrap();
    let name = format!("ATLAS_A_{}", std::process::id());
    let q = quote_ident(&name);
    let generator = quote_ident(&format!("GEN_{name}"));
    let trg = quote_ident(&format!("{name}_BI"));
    let _ = s.execute(&format!("DROP TABLE {q}"), 10);
    let _ = s.execute(&format!("DROP GENERATOR {generator}"), 10);
    s.execute(&format!("CREATE TABLE {q} (ID INTEGER NOT NULL PRIMARY KEY, NAME VARCHAR(20))"), 10).unwrap();
    s.execute(&format!("CREATE GENERATOR {generator}"), 10).unwrap();
    // O gatilho tem corpo com `;`: o separador de comandos não pode cortá-lo.
    s.execute(&format!("CREATE TRIGGER {trg} FOR {q}\nACTIVE BEFORE INSERT POSITION 0\nAS\nBEGIN\n  IF (NEW.ID IS NULL) THEN\n    NEW.ID = GEN_ID({generator}, 1);\nEND"), 10).unwrap();
    let st = |sql: String| PlannedStatement { sql, ..Default::default() };
    s.apply(&[st(format!("INSERT INTO {q} (NAME) VALUES ('a')")), st(format!("INSERT INTO {q} (NAME) VALUES ('b')"))]).unwrap();
    let out = s.execute(&format!("SELECT ID, NAME FROM {q} ORDER BY ID"), 10).unwrap();
    let rows = &out.statements[0].result.as_ref().unwrap().rows;
    assert_eq!(rows[0][0].as_deref(), Some("1"));
    assert_eq!(rows[1][0].as_deref(), Some("2"));
    let d = s.table_detail(&name).unwrap();
    assert_eq!(d.columns[0].identity.as_deref(), Some("d"));
    assert_eq!(d.columns[0].auto_source.as_deref(), Some(format!("{name}_BI|GEN_{name}").as_str()));
    assert!(d.constraints.iter().any(|c| c.kind == "p" && c.fields == "ID"));
    // O índice da chave primária aparece como restrição, não como índice.
    assert!(d.indexes.is_empty());
    s.execute(&format!("DROP TRIGGER {trg}"), 10).unwrap();
    s.execute(&format!("DROP GENERATOR {generator}"), 10).unwrap();
    s.execute(&format!("DROP TABLE {q}"), 10).unwrap();
}

#[test]
fn live_identity_column_on_firebird_3_and_later() {
    let Some(p) = params() else { return };
    let _guard = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    let s = FirebirdSession::open(Target::from_params(&p)).unwrap();
    if s.version_num < 3_000_000 {
        return;
    }
    let name = format!("ATLAS_I_{}", std::process::id());
    let q = quote_ident(&name);
    let _ = s.execute(&format!("DROP TABLE {q}"), 10);
    s.execute(&format!("CREATE TABLE {q} (ID INTEGER GENERATED BY DEFAULT AS IDENTITY NOT NULL PRIMARY KEY, NAME VARCHAR(20))"), 10).unwrap();
    s.apply(&[PlannedStatement { sql: format!("INSERT INTO {q} (NAME) VALUES ('a')"), ..Default::default() }]).unwrap();
    let d = s.table_detail(&name).unwrap();
    assert_eq!(d.columns[0].identity.as_deref(), Some("d"));
    assert_eq!(d.columns[0].auto_source, None);
    s.execute(&format!("DROP TABLE {q}"), 10).unwrap();
}

#[test]
fn live_routines_views_round_trip() {
    let Some(p) = params() else { return };
    let _guard = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    let s = FirebirdSession::open(Target::from_params(&p)).unwrap();
    for sql in ["DROP VIEW ATLAS_V", "DROP PROCEDURE ATLAS_P", "DROP FUNCTION ATLAS_F", "DROP TABLE ATLAS_R"] {
        let _ = s.execute(sql, 10);
    }
    s.execute("CREATE TABLE ATLAS_R (ID INTEGER NOT NULL PRIMARY KEY, N VARCHAR(10))", 10).unwrap();
    s.execute("CREATE OR ALTER VIEW ATLAS_V AS SELECT ID, N FROM ATLAS_R WHERE ID > 1", 10).unwrap();
    s.execute("CREATE OR ALTER VIEW ATLAS_V AS SELECT ID, N FROM ATLAS_R WHERE ID > 2", 10).unwrap();
    assert!(s.view_detail("ATLAS_V").unwrap().definition.contains("ID > 2"));
    let create = "CREATE OR ALTER PROCEDURE ATLAS_P (A INTEGER, B VARCHAR(5))\nRETURNS (X INTEGER, Y VARCHAR(10))\nAS\nBEGIN\n  X = A + 1;\n  Y = B;\n  SUSPEND;\nEND";
    s.execute(create, 10).unwrap();
    let def = s.routine_definition("ATLAS_P").unwrap();
    assert!(def.contains("CREATE OR ALTER PROCEDURE \"ATLAS_P\"") && def.contains("\"A\" integer") && def.contains("RETURNS") && def.contains("SUSPEND;"), "{def}");
    // O texto lido grava de volta sem erro.
    s.execute(&def, 10).unwrap();
    let out = s.execute("SELECT X, Y FROM ATLAS_P(4, 'ab')", 10).unwrap();
    assert_eq!(out.statements[0].result.as_ref().unwrap().rows[0][0].as_deref(), Some("5"));
    if s.version_num >= 3_000_000 {
        s.execute("CREATE OR ALTER FUNCTION ATLAS_F (V INTEGER) RETURNS INTEGER\nAS\nBEGIN\n  RETURN V * 2;\nEND", 10).unwrap();
        let f = s.routine_definition("ATLAS_F").unwrap();
        assert!(f.contains("CREATE OR ALTER FUNCTION \"ATLAS_F\"") && f.contains("RETURNS integer") && f.contains("RETURN V * 2"), "{f}");
        s.execute(&f, 10).unwrap();
        let objs = s.list_objects().unwrap();
        assert!(objs.functions.iter().any(|o| o.name == "ATLAS_F" && !o.procedure));
        s.execute("DROP FUNCTION ATLAS_F", 10).unwrap();
    }
    let objs = s.list_objects().unwrap();
    assert!(objs.functions.iter().any(|o| o.name == "ATLAS_P" && o.procedure));
    assert!(objs.views.iter().any(|o| o.name == "ATLAS_V"));
    for sql in ["DROP VIEW ATLAS_V", "DROP PROCEDURE ATLAS_P", "DROP TABLE ATLAS_R"] {
        s.execute(sql, 10).unwrap();
    }
}

#[test]
fn live_dump_restore_profile_find_diagram() {
    let Some(p) = params() else { return };
    let _guard = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    let s = FirebirdSession::open(Target::from_params(&p)).unwrap();
    let boolean_ok = s.version_num >= 3_000_000;
    for sql in ["DROP VIEW DMP_V", "DROP TABLE DMP_B", "DROP TABLE DMP_A", "DROP GENERATOR DMP_G", "DROP PROCEDURE DMP_P"] {
        let _ = s.execute(sql, 10);
    }
    s.execute("CREATE TABLE DMP_A (ID INTEGER NOT NULL PRIMARY KEY, NAME VARCHAR(20) DEFAULT 'x' NOT NULL, TOTAL NUMERIC(10,2), BORN DATE, TWICE COMPUTED BY (ID * 2))", 10).unwrap();
    s.execute("CREATE TABLE DMP_B (ID INTEGER NOT NULL PRIMARY KEY, A_ID INTEGER, NOTE VARCHAR(30), CONSTRAINT FK_DMP_B FOREIGN KEY (A_ID) REFERENCES DMP_A (ID))", 10).unwrap();
    s.execute("CREATE INDEX IX_DMP_B ON DMP_B (NOTE)", 10).unwrap();
    s.execute("CREATE GENERATOR DMP_G", 10).unwrap();
    s.execute("SELECT GEN_ID(DMP_G, 41) FROM RDB$DATABASE", 10).unwrap();
    s.execute("CREATE VIEW DMP_V AS SELECT ID, NAME FROM DMP_A", 10).unwrap();
    s.execute("CREATE PROCEDURE DMP_P RETURNS (X INTEGER) AS BEGIN X = 1; SUSPEND; END", 10).unwrap();
    s.execute("INSERT INTO DMP_A (ID, NAME, TOTAL, BORN) VALUES (1, 'it''s', 10.50, '2024-03-05')", 10).unwrap();
    s.execute("INSERT INTO DMP_A (ID) VALUES (2)", 10).unwrap();
    s.execute("INSERT INTO DMP_B (ID, A_ID, NOTE) VALUES (1, 1, 'n1')", 10).unwrap();

    let only: Vec<String> = ["DMP_A", "DMP_B"].map(String::from).to_vec();
    let views: Vec<String> = vec!["DMP_V".into()];
    let funcs: Vec<String> = vec!["DMP_P".into()];
    let plan = s.dump_plan(Some(&only), Some(&views), Some(&funcs), true).unwrap();
    assert_eq!(plan.data_tables.len(), 2);
    let mut script = String::new();
    for st in &plan.pre_data {
        script.push_str(&format!("{st}\n"));
    }
    for t in &plan.data_tables {
        let list = t.columns.iter().map(|(n, _)| quote_ident(n)).collect::<Vec<_>>().join(", ");
        let mut rows: Vec<String> = Vec::new();
        stream_rows(&s.target, &format!("SELECT {list} FROM {}", quote_ident(&t.name)), |_| Ok(()), |v| {
            let cells: Vec<String> = v.iter().zip(&t.columns).map(|(x, (_, ty))| literal(*x, ty, boolean_ok)).collect();
            rows.push(format!("INSERT INTO {} ({list}) VALUES ({});", quote_ident(&t.name), cells.join(", ")));
            Ok(true)
        })
        .unwrap();
        script.push_str(&rows.join("\n"));
        script.push('\n');
    }
    for st in &plan.post_data {
        script.push_str(&format!("{st}\n"));
    }
    assert!(script.contains("COMPUTED BY") && script.contains("SET TERM ^ ;") && script.contains("FOREIGN KEY"), "{script}");
    let mut log = Vec::new();
    let done = s.run_script(&script, &mut |l| log.push(l)).unwrap();
    assert!(done > 10, "{done} {log:?}");
    let out = s.execute("SELECT ID, NAME, TOTAL, BORN, TWICE FROM DMP_A ORDER BY ID", 10).unwrap();
    let rows = &out.statements[0].result.as_ref().unwrap().rows;
    assert_eq!(rows[0][1].as_deref(), Some("it's"));
    assert_eq!(rows[0][2].as_deref(), Some("10.50"));
    assert_eq!(rows[0][3].as_deref(), Some("2024-03-05"));
    assert_eq!(rows[1][1].as_deref(), Some("x"));
    assert_eq!(rows[1][4].as_deref(), Some("4"));
    let g = s.execute("SELECT GEN_ID(DMP_G, 0) FROM RDB$DATABASE", 10).unwrap();
    assert_eq!(g.statements[0].result.as_ref().unwrap().rows[0][0].as_deref(), Some("41"));
    let d = s.table_detail("DMP_B").unwrap();
    assert!(d.constraints.iter().any(|c| c.kind == "f" && c.ref_table.as_deref() == Some("DMP_A")));

    // Perfil, rótulos, diagrama e busca.
    let prof = s.data_profile("DMP_A", &["NAME".to_string(), "TOTAL".to_string()], &[], None, None).unwrap();
    assert_eq!(prof.rows, 2);
    assert_eq!(prof.columns[0].distinct, Some(2));
    assert_eq!(prof.columns[1].nulls, 1);
    let labels = s.fk_labels("DMP_A", "ID", &["1".into(), "2".into()]).unwrap();
    assert_eq!(labels.label_column.as_deref(), Some("NAME"));
    assert!(labels.labels.iter().any(|l| l[0] == "1" && l[1] == "it's"));
    let diagram = s.schema_diagram().unwrap();
    assert!(diagram.relations.iter().any(|r| r.table == "DMP_B" && r.ref_table == "DMP_A"));
    let opts = |text: &str, mode| FindOptions { text: text.into(), schema: None, mode, case_insensitive: true, tables: true, views: true, matviews: false, functions: true, indexes: true, triggers: true };
    let found = s.find(&opts("dmp_", find::FindMode::Contains)).unwrap();
    assert!(found.objects.iter().any(|o| o.name == "DMP_A") && found.objects.iter().any(|o| o.kind == "function" && o.name == "DMP_P"));
    let data = s.find_data(&opts("it's", find::FindMode::Contains), &|| false).unwrap();
    assert!(data.tables.iter().any(|t| t.table == "DMP_A" && t.count == 1), "{data:?}");
    let by_prefix = s.find_data(&opts("N1", find::FindMode::Prefix), &|| false).unwrap();
    assert!(by_prefix.tables.iter().any(|t| t.table == "DMP_B"));

    // Importação nos modos do assistente.
    let spec = |mode, keys: Vec<usize>| ImportSpec { table: "DMP_A".into(), create: None, columns: vec![TargetColumn { name: "ID".into(), declared_type: "integer".into() }, TargetColumn { name: "NAME".into(), declared_type: "varchar".into() }], key_columns: keys, mode, extended_insert: false, max_statement_bytes: 1024, continue_on_error: true };
    let mut queue: Vec<ImportFeed> = vec![ImportFeed::Row(vec![Some("2".into()), Some("upd".into())]), ImportFeed::Row(vec![Some("7".into()), Some("new".into())]), ImportFeed::Row(vec![Some("bad".into()), Some("z".into())])];
    queue.reverse();
    let mut noop = |_: String| {};
    let mut noprog = |_: &ImportStats| {};
    let stats = s.import(&spec(ImportMode::AppendUpdate, vec![0]), &mut || queue.pop().unwrap_or(ImportFeed::End), ImportHooks { log: &mut noop, progress: &mut noprog, cancelled: &|| false }).unwrap();
    assert_eq!((stats.updated, stats.added, stats.errors), (1, 1, 1), "{stats:?}");
    let q = s.execute("SELECT NAME FROM DMP_A WHERE ID IN (2, 7) ORDER BY ID", 10).unwrap();
    assert_eq!(q.statements[0].result.as_ref().unwrap().rows.iter().map(|r| r[0].clone().unwrap()).collect::<Vec<_>>(), vec!["upd", "new"]);

    for sql in ["DROP VIEW DMP_V", "DROP PROCEDURE DMP_P", "DROP TABLE DMP_B", "DROP TABLE DMP_A", "DROP GENERATOR DMP_G"] {
        s.execute(sql, 10).unwrap();
    }
}

#[test]
fn console_knows_complete_commands() {
    assert!(console_is_complete("select 1 from rdb$database;"));
    assert!(!console_is_complete("select 1 from rdb$database"));
    assert!(!console_is_complete("create procedure p as begin x = 1;"));
    assert!(console_is_complete("create procedure p as begin x = 1; end;"));
    assert!(console_is_complete(".tables"));
}

#[test]
fn live_console_and_monitor() {
    let Some(p) = params() else { return };
    let _guard = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    let s = FirebirdSession::open(Target::from_params(&p)).unwrap();
    let out = s.console_run("select 1 as a, 'x' as b from rdb$database;", ConsoleOptions::default()).unwrap();
    assert!(out.text.contains("(1 row)"), "{}", out.text);
    let t = s.console_run(".tables", ConsoleOptions::default()).unwrap();
    assert!(t.text.contains("List of relations") || t.text.contains("Did not find"));
    let snap = s.monitor_snapshot().unwrap();
    assert!(snap.processes.iter().any(|p| p.id == s.attachment_id), "{:?}", snap.processes);
    assert!(snap.variables.iter().any(|(k, _)| k == "Firebird version"));
    // Encerrar uma conexão de outra sessão.
    let other = FirebirdSession::open(Target::from_params(&p)).unwrap();
    assert!(s.monitor_end_process(other.attachment_id).unwrap());
    assert!(other.execute("select 1 from rdb$database", 10).is_err());
}
