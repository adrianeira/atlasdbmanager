//! Testes contra um MySQL de verdade. Rodam só com as variáveis `ATLAS_TEST_MYSQL_*` (senão passam sem fazer nada).

use super::*;
use atlas_core::{ConnectionParams, OrderBy, Secret, SslMode, TableFilter};

fn params() -> Option<ConnectionParams> {
    Some(ConnectionParams {
        host: std::env::var("ATLAS_TEST_MYSQL_HOST").ok()?,
        port: std::env::var("ATLAS_TEST_MYSQL_PORT").ok()?.parse().ok()?,
        database: String::new(),
        user: std::env::var("ATLAS_TEST_MYSQL_USER").ok()?,
        password: Secret::new(std::env::var("ATLAS_TEST_MYSQL_PASSWORD").ok()?),
        ssl_mode: SslMode::Disable,
        connect_timeout_secs: 15,
        ssh: None,
    })
}

#[test]
fn quoting_doubles_the_backtick() {
    assert_eq!(quote_ident("audit_log"), "`audit_log`");
    assert_eq!(quote_ident("we`ird"), "`we``ird`");
}

#[test]
fn select_pages_and_versions() {
    let o = [OrderBy { column: "id".into(), descending: true }];
    let f = TableFilter { expr: "qty > 1".into() };
    assert_eq!(select_sql("d", "t", &o, Some(&f), Some((10, 20))), "SELECT * FROM `d`.`t` WHERE qty > 1 ORDER BY `id` DESC LIMIT 10 OFFSET 20");
    assert_eq!(parse_version("8.4.3"), ("8.4".to_string(), 80403));
    assert_eq!(parse_version("10.11.5-MariaDB-1:10.11.5"), ("10.11".to_string(), 101105));
}

#[tokio::test]
async fn live_server_lists_reads_and_writes() {
    let Some(p) = params() else { return };
    let s = MysqlSession::open(p).await.unwrap();
    assert!(s.version_num >= 50700);
    let name = format!("atlas_t_{}", std::process::id());
    s.execute("", &format!("CREATE DATABASE {}", quote_ident(&name)), 10).await.unwrap();
    let r = async {
        s.execute(&name, "CREATE TABLE clients (id int AUTO_INCREMENT PRIMARY KEY, name varchar(20) NOT NULL, total decimal(10,2) NULL DEFAULT 0, active bit(1) NULL, born date NULL, photo blob NULL, CONSTRAINT ck_total CHECK (total >= 0))", 10).await?;
        s.execute(&name, "INSERT INTO clients (name, total, active, born, photo) VALUES ('Ana', 150.5, b'1', '2020-01-02', 0x0aff), ('Bia', NULL, b'0', NULL, NULL)", 10).await?;
        let dbs = s.list_databases(false).await?;
        assert!(dbs.databases.contains(&name) && !dbs.databases.contains(&"mysql".to_string()) && dbs.hidden >= 3);
        let d = s.table_detail(&name, "clients").await?;
        assert_eq!((d.columns[0].identity.as_deref(), d.columns[0].key), (Some("d"), Some(1)));
        assert_eq!(d.columns[1].formatted_type, "varchar(20)");
        assert_eq!((d.columns[2].formatted_type.as_str(), d.columns[2].default.as_deref()), ("decimal(10,2)", Some("0.00")));
        assert!(d.constraints.iter().any(|c| c.kind == "c" && c.name == "ck_total"));
        let (sql, out) = s.table_rows(&name, "clients", &[], None, Some(10), 0, 100).await?;
        assert!(sql.ends_with("LIMIT 10 OFFSET 0"));
        let rows = &out.statements[0].result.as_ref().unwrap().rows;
        assert_eq!(rows[0], vec![Some("1".into()), Some("Ana".into()), Some("150.50".into()), Some("t".into()), Some("2020-01-02".into()), Some("\\x0aff".into())]);
        assert_eq!(s.table_count(&name, "clients", None).await?, 2);
        assert_eq!(s.list_objects(&name).await?.tables[0].name, "clients");
        let up = PlannedStatement { sql: "UPDATE `atlas`.`x` SET name = ? WHERE id = ?".replace("`atlas`.`x`", &format!("{}.clients", quote_ident(&name))), params: vec![Some("Ana B".into()), Some("1".into())], expect_one_row: true, expect_rows: None };
        assert_eq!(s.apply(&name, &[up]).await?, vec![1]);
        // Um UPDATE que não muda nada ainda casa a linha (CLIENT_FOUND_ROWS).
        let same = PlannedStatement { sql: format!("UPDATE {}.clients SET name = ? WHERE id = ?", quote_ident(&name)), params: vec![Some("Ana B".into()), Some("1".into())], expect_one_row: true, expect_rows: None };
        assert_eq!(s.apply(&name, &[same]).await?, vec![1]);
        let bad = PlannedStatement { sql: format!("UPDATE {}.clients SET name = ? WHERE id = ?", quote_ident(&name)), params: vec![Some("x".into()), Some("99".into())], expect_one_row: true, expect_rows: None };
        assert_eq!(s.apply(&name, &[bad]).await.unwrap_err().kind, ErrorKind::ConcurrencyConflict);
        let tx = s.begin(&name).await?;
        tx.execute("DELETE FROM clients", 10).await?;
        assert_eq!(tx.table_count(&name, "clients", None).await?, 0);
        tx.finish(false).await?;
        assert_eq!(s.table_count(&name, "clients", None).await?, 2);
        // Vários resultados num texto só.
        let multi = s.execute(&name, "SELECT 1 AS a; SELECT 2 AS b, 3 AS c; UPDATE clients SET name = name WHERE id = 99", 10).await?;
        // O comando sem resultado no fim do texto não vira um resultado à parte (o driver não o expõe depois do último result set).
        assert_eq!(multi.statements.len(), 2);
        Ok::<_, DatabaseError>(())
    }
    .await;
    s.close_database(&name).await;
    let _ = s.execute("", &format!("DROP DATABASE {}", quote_ident(&name)), 10).await;
    r.unwrap();
}

#[tokio::test]
async fn live_cancel_stops_a_long_query() {
    let Some(p) = params() else { return };
    let s = std::sync::Arc::new(MysqlSession::open(p).await.unwrap());
    let runner = s.clone();
    let started = std::time::Instant::now();
    let long = tokio::spawn(async move { runner.execute("", "SELECT SLEEP(30)", 10).await });
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    s.cancel("").await.unwrap();
    let out = long.await.unwrap();
    // O MySQL devolve o `SLEEP` interrompido como resultado 1 (ou como erro 1317).
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    assert!(out.is_ok() || out.unwrap_err().kind == ErrorKind::Canceled);
}
