//! Testes contra um SQL Server de verdade. Rodam só com as variáveis `ATLAS_TEST_MSSQL_*` (senão passam sem fazer nada).

use super::*;
use atlas_core::{ConnectionParams, OrderBy, Secret, SslMode, TableFilter};

fn params() -> Option<ConnectionParams> {
    let host = std::env::var("ATLAS_TEST_MSSQL_HOST").ok()?;
    Some(ConnectionParams {
        host,
        port: std::env::var("ATLAS_TEST_MSSQL_PORT").ok()?.parse().ok()?,
        database: "master".into(),
        user: std::env::var("ATLAS_TEST_MSSQL_USER").ok()?,
        password: Secret::new(std::env::var("ATLAS_TEST_MSSQL_PASSWORD").ok()?),
        ssl_mode: SslMode::Require,
        connect_timeout_secs: 15,
        ssh: None,
    })
}

#[test]
fn quoting_doubles_the_closing_bracket() {
    assert_eq!(quote_ident("audit_log"), "[audit_log]");
    assert_eq!(quote_ident("we]ird"), "[we]]ird]");
    assert_eq!(quote_literal("it's"), "N'it''s'");
}

#[test]
fn select_pages_need_an_order() {
    let o = [OrderBy { column: "id".into(), descending: true }];
    let f = TableFilter { expr: "qty > 1".into() };
    assert_eq!(select_sql("dbo", "t", &[], None, None), "SELECT * FROM [dbo].[t]");
    assert_eq!(select_sql("dbo", "t", &o, Some(&f), Some((10, 20))), "SELECT * FROM [dbo].[t] WHERE qty > 1 ORDER BY [id] DESC OFFSET 20 ROWS FETCH NEXT 10 ROWS ONLY");
    assert_eq!(select_sql("dbo", "t", &[], None, Some((5, 0))), "SELECT * FROM [dbo].[t] ORDER BY (SELECT NULL) OFFSET 0 ROWS FETCH NEXT 5 ROWS ONLY");
}

#[tokio::test]
async fn live_server_lists_reads_and_writes() {
    let Some(p) = params() else { return };
    let s = MssqlSession::open(p).await.unwrap();
    assert!(s.version_num >= 90_000);
    let dbs = s.list_databases(true).await.unwrap();
    assert!(dbs.databases.iter().any(|d| d == "master"));
    let name = format!("atlas_t_{}", std::process::id());
    s.execute("master", &format!("CREATE DATABASE {}", quote_ident(&name)), 10).await.unwrap();
    let r = async {
        s.execute(&name, "CREATE TABLE dbo.clients (id int IDENTITY(1,1) PRIMARY KEY, name nvarchar(20) NOT NULL, total decimal(10,2) NULL DEFAULT 0, active bit NULL, born date NULL)", 10).await?;
        s.execute(&name, "INSERT INTO dbo.clients (name, total, active, born) VALUES (N'Ana', 150.5, 1, '2020-01-02'), (N'Bia', NULL, 0, NULL)", 10).await?;
        let detail = s.table_detail(&name, "dbo", "clients").await?;
        assert_eq!(detail.columns[0].identity.as_deref(), Some("a"));
        assert_eq!(detail.columns[0].key, Some(1));
        assert_eq!(detail.columns[1].formatted_type, "nvarchar(20)");
        assert_eq!(detail.columns[2].formatted_type, "decimal(10,2)");
        assert_eq!(detail.columns[2].default.as_deref(), Some("0"));
        let (sql, out) = s.table_rows(&name, "dbo", "clients", &[], None, Some(10), 0, 100).await?;
        assert!(sql.contains("OFFSET 0 ROWS"));
        let rows = &out.statements[0].result.as_ref().unwrap().rows;
        assert_eq!(rows[0], vec![Some("1".into()), Some("Ana".into()), Some("150.50".into()), Some("t".into()), Some("2020-01-02".into())]);
        assert_eq!(s.table_count(&name, "dbo", "clients", None).await?, 2);
        let objects = s.list_objects(&name, "dbo").await?;
        assert_eq!(objects.tables[0].name, "clients");
        // Gravação planejada: confere a contagem e desfaz tudo se errar.
        let up = PlannedStatement { sql: "UPDATE [dbo].[clients] SET [name] = @P1 WHERE [id] = @P2".into(), params: vec![Some("Ana B".into()), Some("1".into())], expect_one_row: true, expect_rows: None };
        assert_eq!(s.apply(&name, &[up]).await?, vec![1]);
        let bad = PlannedStatement { sql: "UPDATE [dbo].[clients] SET [name] = @P1 WHERE [id] = @P2".into(), params: vec![Some("x".into()), Some("99".into())], expect_one_row: true, expect_rows: None };
        assert_eq!(s.apply(&name, &[bad]).await.unwrap_err().kind, ErrorKind::ConcurrencyConflict);
        // Transação manual.
        let tx = s.begin(&name).await?;
        tx.execute("DELETE FROM dbo.clients", 10).await?;
        assert_eq!(tx.table_count("dbo", "clients", None).await?, 0);
        tx.finish(false).await?;
        assert_eq!(s.table_count(&name, "dbo", "clients", None).await?, 2);
        Ok::<_, DatabaseError>(())
    }
    .await;
    s.close_database(&name).await;
    let _ = s.execute("master", &format!("ALTER DATABASE {0} SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE {0}", quote_ident(&name)), 10).await;
    r.unwrap();
}

#[tokio::test]
async fn live_labels_diagram_and_find() {
    let Some(p) = params() else { return };
    let s = MssqlSession::open(p).await.unwrap();
    let name = format!("atlas_f_{}", std::process::id());
    s.execute("master", &format!("CREATE DATABASE {}", quote_ident(&name)), 10).await.unwrap();
    let r = async {
        s.execute(&name, "CREATE TABLE dbo.parent (id int PRIMARY KEY, name nvarchar(30) NOT NULL); CREATE TABLE dbo.child (id int PRIMARY KEY, parent_id int NOT NULL REFERENCES dbo.parent(id), note nvarchar(30)); CREATE INDEX ix_child_note ON dbo.child (note); INSERT INTO dbo.parent VALUES (1, N'Alpha'), (2, N'Beta'); INSERT INTO dbo.child VALUES (1, 1, N'parent note')", 10).await?;
        s.execute(&name, "CREATE VIEW dbo.v_parent AS SELECT id, name FROM dbo.parent", 10).await.ok();
        let labels = s.fk_labels(&name, "dbo", "parent", "id", &["1".into(), "2".into(), "9".into()]).await?;
        assert_eq!(labels.label_column.as_deref(), Some("name"));
        assert_eq!(labels.labels, vec![["1".to_string(), "Alpha".to_string()], ["2".to_string(), "Beta".to_string()]]);
        let d = s.schema_diagram(&name, "dbo").await?;
        assert_eq!(d.tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["child", "parent"]);
        assert!(d.tables[0].columns[0].is_pk && d.tables[0].columns[1].is_fk);
        assert_eq!((d.relations.len(), d.relations[0].required, d.relations[0].ref_table.as_str()), (1, true, "parent"));
        let opts = |text: &str, mode| FindOptions { text: text.into(), schema: None, mode, case_insensitive: true, tables: true, views: true, matviews: false, functions: true, indexes: true, triggers: true };
        let f = s.find(&name, &opts("parent", FindMode::Contains)).await?;
        let names: Vec<(&str, &str)> = f.objects.iter().map(|o| (o.kind.as_str(), o.name.as_str())).collect();
        assert_eq!(names, vec![("table", "child"), ("table", "parent"), ("view", "v_parent")]);
        let data = s.find_data(&name, &opts("alp", FindMode::Prefix), &|| false).await?;
        assert_eq!((data.tables.len(), data.tables[0].table.as_str(), data.tables[0].count), (1, "parent", 1));
        assert!(s.find_data(&name, &opts("x", FindMode::Regex), &|| false).await.is_err());
        Ok::<_, DatabaseError>(())
    }
    .await;
    s.close_database(&name).await;
    let _ = s.execute("master", &format!("ALTER DATABASE {0} SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE {0}", quote_ident(&name)), 10).await;
    r.unwrap();
}

#[tokio::test]
async fn live_cancel_stops_a_long_query_and_the_session_recovers() {
    let Some(p) = params() else { return };
    let s = std::sync::Arc::new(MssqlSession::open(p).await.unwrap());
    let runner = s.clone();
    let started = std::time::Instant::now();
    let long = tokio::spawn(async move { runner.execute("master", "WAITFOR DELAY '00:00:30'; SELECT 1", 10).await });
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    s.cancel("master").await.unwrap();
    let err = long.await.unwrap().unwrap_err();
    assert_eq!(err.kind, ErrorKind::Canceled);
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    // A próxima consulta abre uma conexão nova e funciona.
    let out = s.execute("master", "SELECT 7", 10).await.unwrap();
    assert_eq!(out.statements[0].result.as_ref().unwrap().rows[0][0].as_deref(), Some("7"));
}
