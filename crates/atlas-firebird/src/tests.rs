//! Testes contra um Firebird de verdade. Rodam só com as variáveis `ATLAS_TEST_FIREBIRD_*` (senão passam sem fazer nada).

use super::*;
use atlas_core::{ConnectionParams, OrderBy, Secret, SslMode, TableFilter};

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
    assert!(d.indexes.iter().any(|i| i.primary));
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
