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
