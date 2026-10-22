//! Testes contra um PostgreSQL real. Rodam só quando as variáveis abaixo existem
//! (credenciais nunca ficam no repositório):
//!
//!   ATLAS_TEST_PG_HOST, ATLAS_TEST_PG_PORT, ATLAS_TEST_PG_USER,
//!   ATLAS_TEST_PG_PASSWORD, ATLAS_TEST_PG_DB (opcional, padrão "postgres")
//!
//! Exemplo (PowerShell):
//!   $env:ATLAS_TEST_PG_HOST="localhost"; $env:ATLAS_TEST_PG_PORT="5433"; ...
//!   cargo test -p atlas-pg --test live_postgres -- --nocapture
//!
//! Os testes só leem catálogo e usam tabela TEMPORÁRIA; nada persiste no servidor.

use atlas_core::{ConnectionParams, ErrorKind, Secret, SslMode};

fn params() -> Option<ConnectionParams> {
    let host = std::env::var("ATLAS_TEST_PG_HOST").ok()?;
    Some(ConnectionParams {
        host,
        port: std::env::var("ATLAS_TEST_PG_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(5432),
        database: std::env::var("ATLAS_TEST_PG_DB").unwrap_or_else(|_| "postgres".into()),
        user: std::env::var("ATLAS_TEST_PG_USER").unwrap_or_else(|_| "postgres".into()),
        password: Secret::new(std::env::var("ATLAS_TEST_PG_PASSWORD").unwrap_or_default()),
        ssl_mode: SslMode::Disable,
        connect_timeout_secs: 10,
    })
}

macro_rules! require_server {
    () => {
        match params() {
            Some(p) => p,
            None => {
                eprintln!("ATLAS_TEST_PG_HOST não definido: teste pulado");
                return;
            }
        }
    };
}

#[tokio::test]
async fn connects_and_reads_server_info() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    eprintln!("servidor: {:?}", s.server);
    assert!(s.server.version_num >= 90000);
    assert_eq!(s.server.current_database, p.database);
}

#[tokio::test]
async fn wrong_password_is_typed_error() {
    let mut p = require_server!();
    p.password = Secret::new("definitely-wrong-password-atlas");
    let err = atlas_pg::connect(&p, None).await.err().expect("must fail");
    eprintln!("erro: {err:?}");
    // Servidores em "trust" aceitam qualquer senha; aí o teste não se aplica.
    if err.kind != ErrorKind::AuthenticationFailed {
        eprintln!("aviso: servidor não rejeitou a senha (kind={:?})", err.kind);
    }
    assert!(!err.message.contains("definitely-wrong-password-atlas"), "senha vazou na mensagem");
}

#[tokio::test]
async fn lists_catalog() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let dbs = atlas_pg::list_databases(&s).await.expect("databases");
    assert!(dbs.databases.iter().any(|d| d == &p.database));
    let schemas = atlas_pg::list_schemas(&s).await.expect("schemas");
    assert!(!schemas.iter().any(|n| n.starts_with("pg_")));
    let objs = atlas_pg::list_objects(&s, "public").await.expect("objects");
    eprintln!("public: {} tabelas, {} views, {} funções", objs.tables.len(), objs.views.len(), objs.functions.len());
    let roles = atlas_pg::list_roles(&s).await.expect("roles");
    assert!(!roles.is_empty());
}

#[tokio::test]
async fn executes_multi_statement_with_types_nulls_and_limit() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");

    // Comando único: tipos descritos, NULL distinto de vazio, numeric sem perda.
    let out = atlas_pg::execute(&s, "SELECT 1::int4 AS a, NULL::text AS b, ''::text AS c, 12345678901234567890.1234::numeric AS d", 100)
        .await
        .expect("select");
    let rs = out.statements[0].result.as_ref().expect("result set");
    assert_eq!(rs.columns[0].type_name.as_deref(), Some("int4"));
    assert_eq!(rs.rows[0], vec![Some("1".into()), None, Some(String::new()), Some("12345678901234567890.1234".into())]);

    // Lote: o servidor separa os comandos; ';' dentro de literal não quebra nada.
    let out = atlas_pg::execute(
        &s,
        "CREATE TEMP TABLE atlas_t(id int primary key, v text); \
         INSERT INTO atlas_t SELECT g, 'x;y' FROM generate_series(1, 50) g; \
         SELECT * FROM atlas_t ORDER BY id",
        10,
    )
    .await
    .expect("batch");
    assert_eq!(out.statements.len(), 3);
    assert_eq!(out.statements[1].rows_affected, Some(50));
    let rs = out.statements[2].result.as_ref().expect("rows");
    assert_eq!(rs.rows.len(), 10);
    assert!(rs.truncated);
    assert_eq!(rs.rows[0][1].as_deref(), Some("x;y"));

    // Detalhe da tabela temporária: PK identificada.
    let schema = atlas_pg::execute(&s, "SELECT nspname FROM pg_namespace WHERE oid = pg_my_temp_schema()", 1).await.unwrap();
    let temp_schema = schema.statements[0].result.as_ref().unwrap().rows[0][0].clone().unwrap();
    let detail = atlas_pg::table_detail(&s, &temp_schema, "atlas_t").await.expect("detail");
    assert_eq!(detail.columns[0].key, Some(1));
    assert_eq!(detail.columns[0].data_type, "int4");
}

#[tokio::test]
async fn apply_commits_or_rolls_back_on_conflict() {
    use atlas_pg::PlannedStatement;
    let p = require_server!();

    // Commit: UPDATE parametrizado com cast de tipo, afetando exatamente 1 linha.
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    atlas_pg::execute(&s, "CREATE TEMP TABLE atlas_a(id int primary key, v text); INSERT INTO atlas_a VALUES (1,'a'),(2,'b')", 1)
        .await
        .unwrap();
    let ok = atlas_pg::apply_in_transaction(
        s,
        &[PlannedStatement {
            sql: "UPDATE atlas_a SET v = $1::text WHERE id = $2::text::int4 AND v = $3::text".into(),
            params: vec![Some("O'Brien; DROP".into()), Some("1".into()), Some("a".into())],
            expect_one_row: true,
        }],
    )
    .await
    .expect("apply");
    assert_eq!(ok, vec![1]);

    // Conflito: o valor original não confere (outra sessão mudou) → 0 linhas → rollback de tudo.
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    atlas_pg::execute(&s, "CREATE TEMP TABLE atlas_b(id int primary key, v text); INSERT INTO atlas_b VALUES (1,'a'),(2,'b')", 1)
        .await
        .unwrap();
    let err = atlas_pg::apply_in_transaction(
        s,
        &[
            PlannedStatement { sql: "UPDATE atlas_b SET v = 'changed' WHERE id = 2".into(), params: vec![], expect_one_row: true },
            PlannedStatement {
                sql: "UPDATE atlas_b SET v = $1::text WHERE id = $2::text::int4 AND v = $3::text".into(),
                params: vec![Some("x".into()), Some("1".into()), Some("stale".into())],
                expect_one_row: true,
            },
        ],
    )
    .await
    .err()
    .expect("must conflict");
    assert_eq!(err.kind, ErrorKind::ConcurrencyConflict);
}

#[tokio::test]
async fn syntax_error_reports_position() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let err = atlas_pg::execute(&s, "SELEC 1", 10).await.err().expect("must fail");
    assert_eq!(err.kind, ErrorKind::SqlSyntaxError);
    assert_eq!(err.code.as_deref(), Some("42601"));
    assert_eq!(err.position, Some(1));
}

#[tokio::test]
async fn view_detail_reads_definition_and_options() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let barrier = if s.server.version_num >= 90200 { " WITH (security_barrier = true)" } else { "" };
    let check = if s.server.version_num >= 90400 { " WITH LOCAL CHECK OPTION" } else { "" };
    atlas_pg::execute(
        &s,
        &format!(
            "CREATE TEMP TABLE atlas_vt (id int, status text); \
             CREATE TEMP VIEW atlas_v{barrier} AS SELECT id FROM atlas_vt WHERE status = 'open'{check}; \
             COMMENT ON VIEW atlas_v IS 'só abertos'"
        ),
        1,
    )
    .await
    .expect("temp view");
    // Objetos temporários ficam no schema pg_temp_N da sessão.
    let out = atlas_pg::execute(&s, "SELECT nspname::text FROM pg_namespace WHERE oid = pg_my_temp_schema()", 1).await.expect("ns");
    let ns = out.statements[0].result.as_ref().unwrap().rows[0][0].clone().unwrap();

    let v = atlas_pg::view_detail(&s, &ns, "atlas_v").await.expect("view detail");
    assert!(v.definition.starts_with("SELECT"), "{}", v.definition);
    assert!(v.definition.contains("'open'::text"), "{}", v.definition);
    assert!(!v.definition.ends_with(';'));
    assert_eq!(v.comment.as_deref(), Some("só abertos"));
    assert_eq!(v.security_barrier, s.server.version_num >= 90200);
    assert_eq!(v.check_option.as_deref(), (s.server.version_num >= 90400).then_some("local"));
    assert!(v.rules.is_empty());

    let err = atlas_pg::view_detail(&s, &ns, "atlas_vt").await.expect_err("table is not a view");
    assert_eq!(err.code.as_deref(), Some("42P01"));
}

#[tokio::test]
async fn table_rows_pages_and_reads_all() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    atlas_pg::execute(&s, "CREATE TEMP TABLE atlas_pages AS SELECT g AS id FROM generate_series(1, 25) g", 1)
        .await
        .expect("temp table");
    let order = vec![atlas_core::OrderBy { column: "id".into(), descending: false }];
    let rows = |o: &atlas_core::QueryOutcome| o.statements[0].result.as_ref().unwrap().rows.clone();

    // Página 2 com 10 por página.
    let (sql, out) = atlas_pg::table_rows(&s, "pg_temp", "atlas_pages", &order, None, Some(10), 10, 1_000).await.expect("page");
    assert!(sql.ends_with("LIMIT 10 OFFSET 10"), "{sql}");
    let r = rows(&out);
    assert_eq!(r.len(), 10);
    assert_eq!(r[0][0].as_deref(), Some("11"));

    // "Limitar registros" desmarcado: sem LIMIT, tabela inteira.
    let (sql, out) = atlas_pg::table_rows(&s, "pg_temp", "atlas_pages", &order, None, None, 0, 1_000).await.expect("all");
    assert!(!sql.contains("LIMIT"), "{sql}");
    assert_eq!(rows(&out).len(), 25);
    assert!(!out.statements[0].result.as_ref().unwrap().truncated);

    // Teto de segurança: guarda só `max_rows` e sinaliza o corte.
    let (_, out) = atlas_pg::table_rows(&s, "pg_temp", "atlas_pages", &order, None, None, 0, 20).await.expect("cap");
    assert_eq!(rows(&out).len(), 20);
    assert!(out.statements[0].result.as_ref().unwrap().truncated);
}

#[tokio::test]
async fn stream_rows_reads_in_order_reports_types_and_can_stop_early() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    atlas_pg::execute(
        &s,
        "CREATE TEMP TABLE atlas_stream AS SELECT g AS id, (g % 2 = 0) AS par, 'v' || g AS txt, NULL::text AS nulo FROM generate_series(1, 25) g",
        1,
    )
    .await
    .expect("temp table");
    let sql = atlas_pg::select_sql("pg_temp", "atlas_stream", &["id".into(), "par".into(), "nulo".into()], &[atlas_core::OrderBy { column: "id".into(), descending: true }], None);
    assert_eq!(sql, "SELECT \"id\", \"par\", \"nulo\" FROM \"pg_temp\".\"atlas_stream\" ORDER BY \"id\" DESC");

    // Lê tudo: colunas com tipo, NULL como None, ordem pedida.
    let mut cols = Vec::new();
    let mut first = None;
    let n = atlas_pg::stream_rows(
        &s,
        &sql,
        |c| {
            cols = c.iter().map(|x| (x.name.clone(), x.type_name.clone())).collect();
            Ok(())
        },
        |row| {
            if first.is_none() {
                first = Some(row.iter().map(|v| v.map(str::to_string)).collect::<Vec<_>>());
            }
            Ok(true)
        },
    )
    .await
    .expect("stream");
    assert_eq!(n, 25);
    assert_eq!(cols, vec![("id".into(), Some("int4".into())), ("par".into(), Some("bool".into())), ("nulo".into(), Some("text".into()))]);
    assert_eq!(first.unwrap(), vec![Some("25".into()), Some("f".into()), None]);

    // Parada antecipada (cancelamento): entrega só o que foi lido e a sessão continua utilizável.
    let mut seen = 0;
    let n = atlas_pg::stream_rows(&s, &sql, |_| Ok(()), |_| { seen += 1; Ok(seen < 3) }).await.expect("stop early");
    assert_eq!(n, 3);
    let again = atlas_pg::execute(&s, "SELECT count(*) FROM atlas_stream", 1).await.expect("session still works");
    assert_eq!(again.statements[0].result.as_ref().unwrap().rows[0][0].as_deref(), Some("25"));

    // Resultado vazio não chama on_row, mas ainda descreve as colunas.
    let mut described = false;
    let n = atlas_pg::stream_rows(&s, "SELECT id FROM atlas_stream WHERE false", |c| { described = !c.is_empty(); Ok(()) }, |_| Ok(true)).await.expect("empty");
    assert!(n == 0 && described);
}
