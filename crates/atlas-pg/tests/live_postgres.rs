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

/// Servidor de destino das transferências e sincronizações: outra porta (e, se preciso, outro usuário e senha) cruza 9.6 e 18.
fn target_params(p: &ConnectionParams) -> ConnectionParams {
    let mut t = p.clone();
    if let Some(port) = std::env::var("ATLAS_TEST_PG_TARGET_PORT").ok().and_then(|x| x.parse().ok()) {
        t.port = port;
    }
    if let Ok(user) = std::env::var("ATLAS_TEST_PG_TARGET_USER") {
        t.user = user;
    }
    if let Ok(password) = std::env::var("ATLAS_TEST_PG_TARGET_PASSWORD") {
        t.password = Secret::new(password);
    }
    t
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
    let Err(err) = atlas_pg::connect(&p, None).await else { panic!("must fail") };
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
    let dbs = atlas_pg::list_databases(&s, false).await.expect("databases");
    assert!(dbs.databases.iter().any(|d| d == &p.database));
    let schemas = atlas_pg::list_schemas(&s, false).await.expect("schemas");
    assert!(!schemas.iter().any(|n| n.name.starts_with("pg_")));

    // "Show Hidden Items": com ele, entram os bancos ocultos (template0/template1) e os schemas de sistema.
    assert!(dbs.hidden >= 1, "o servidor tem ao menos um template oculto");
    let todos = atlas_pg::list_databases(&s, true).await.expect("databases com ocultos");
    assert_eq!(todos.hidden, 0);
    assert!(todos.databases.len() as u32 == dbs.databases.len() as u32 + dbs.hidden, "os ocultos passam a vir na lista");
    assert!(todos.databases.iter().any(|d| d == "template1"));
    let sistema = atlas_pg::list_schemas(&s, true).await.expect("schemas com ocultos");
    assert!(sistema.iter().any(|n| n.name == "pg_catalog") && sistema.iter().any(|n| n.name == "information_schema"));
    assert!(!sistema.iter().any(|n| n.name.starts_with("pg_temp") || n.name.starts_with("pg_toast")), "temporários e TOAST ficam de fora");
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
    .expect_err("must conflict");
    assert_eq!(err.kind, ErrorKind::ConcurrencyConflict);
}

#[tokio::test]
async fn manual_transaction_keeps_batches_until_commit() {
    let p = require_server!();
    let name = format!("atlas_mt_{}", std::process::id());
    let setup = atlas_pg::connect(&p, None).await.expect("connect");
    atlas_pg::execute(&setup, &format!("DROP TABLE IF EXISTS {name}; CREATE TABLE {name}(id int primary key, v text); INSERT INTO {name} VALUES (1,'a'),(2,'b')"), 1)
        .await
        .unwrap();
    let st = |sql: String, params: Vec<Option<String>>| atlas_pg::PlannedStatement { sql, params, expect_one_row: true };

    let tx = atlas_pg::connect(&p, None).await.expect("connect");
    atlas_pg::begin_manual_transaction(&tx).await.unwrap();
    // Lote 1 ok; lote 2 falha (chave duplicada) e só ele é desfeito; a transação segue válida.
    atlas_pg::apply_in_open_transaction(&tx, &[st(format!("UPDATE {name} SET v = 'x' WHERE id = 1"), vec![])]).await.unwrap();
    let Err(err) = atlas_pg::apply_in_open_transaction(
        &tx,
        &[st(format!("UPDATE {name} SET v = 'y' WHERE id = 2"), vec![]), st(format!("UPDATE {name} SET id = 1 WHERE id = 2"), vec![])],
    )
    .await
    else {
        panic!("duplicate key must fail")
    };
    assert_eq!(err.code.as_deref(), Some("23505"));
    // Conflito de linhas afetadas também desfaz só o lote.
    let Err(err) = atlas_pg::apply_in_open_transaction(&tx, &[st(format!("UPDATE {name} SET v = 'z' WHERE id = 99"), vec![])]).await else {
        panic!("0 rows must conflict")
    };
    assert_eq!(err.kind, ErrorKind::ConcurrencyConflict);

    // Dentro da transação o lote 1 é visível e o 2 não; fora dela nada foi gravado ainda.
    let inside = atlas_pg::execute(&tx, &format!("SELECT string_agg(v, ',' ORDER BY id) FROM {name}"), 10).await.unwrap();
    assert_eq!(inside.statements[0].result.as_ref().unwrap().rows[0][0].as_deref(), Some("x,b"));
    let outside = atlas_pg::execute(&setup, &format!("SELECT string_agg(v, ',' ORDER BY id) FROM {name}"), 10).await.unwrap();
    assert_eq!(outside.statements[0].result.as_ref().unwrap().rows[0][0].as_deref(), Some("a,b"));

    atlas_pg::end_manual_transaction(&tx, true).await.unwrap();
    let after = atlas_pg::execute(&setup, &format!("SELECT string_agg(v, ',' ORDER BY id) FROM {name}"), 10).await.unwrap();
    assert_eq!(after.statements[0].result.as_ref().unwrap().rows[0][0].as_deref(), Some("x,b"));

    // Rollback desfaz tudo o que foi feito depois.
    let tx = atlas_pg::connect(&p, None).await.expect("connect");
    atlas_pg::begin_manual_transaction(&tx).await.unwrap();
    atlas_pg::apply_in_open_transaction(&tx, &[st(format!("UPDATE {name} SET v = 'gone' WHERE id = 2"), vec![])]).await.unwrap();
    atlas_pg::end_manual_transaction(&tx, false).await.unwrap();
    let after = atlas_pg::execute(&setup, &format!("SELECT string_agg(v, ',' ORDER BY id) FROM {name}"), 10).await.unwrap();
    assert_eq!(after.statements[0].result.as_ref().unwrap().rows[0][0].as_deref(), Some("x,b"));
    atlas_pg::execute(&setup, &format!("DROP TABLE {name}"), 1).await.unwrap();
}

#[tokio::test]
async fn matview_detail_reads_definition_and_populated_state() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    if s.server.version_num < 90300 {
        return; // views materializadas: PostgreSQL 9.3+
    }
    let ns = format!("atlas_mv_{}", std::process::id());
    atlas_pg::execute(
        &s,
        &format!(
            "DROP SCHEMA IF EXISTS {ns} CASCADE; CREATE SCHEMA {ns}; CREATE TABLE {ns}.t(id int); INSERT INTO {ns}.t VALUES (1),(2); \
             CREATE MATERIALIZED VIEW {ns}.mv_full AS SELECT id FROM {ns}.t; \
             CREATE MATERIALIZED VIEW {ns}.mv_empty AS SELECT id FROM {ns}.t WITH NO DATA; \
             COMMENT ON MATERIALIZED VIEW {ns}.mv_full IS 'resumo'"
        ),
        1,
    )
    .await
    .unwrap();

    let full = atlas_pg::matview_detail(&s, &ns, "mv_full").await.expect("matview detail");
    assert!(full.definition.to_lowercase().contains("select"));
    assert_eq!(full.comment.as_deref(), Some("resumo"));
    assert!(full.populated);
    assert!(!atlas_pg::matview_detail(&s, &ns, "mv_empty").await.unwrap().populated);

    // Refresh (inclusive o concorrente, que exige índice único) e leitura dos dados.
    atlas_pg::execute(&s, &format!("REFRESH MATERIALIZED VIEW {ns}.mv_empty"), 1).await.unwrap();
    assert!(atlas_pg::matview_detail(&s, &ns, "mv_empty").await.unwrap().populated);
    atlas_pg::execute(&s, &format!("CREATE UNIQUE INDEX ON {ns}.mv_full(id); REFRESH MATERIALIZED VIEW CONCURRENTLY {ns}.mv_full"), 1)
        .await
        .unwrap();

    // Uma tabela ou uma view comum não é view materializada, e vice-versa.
    atlas_pg::execute(&s, &format!("CREATE VIEW {ns}.v AS SELECT 1 AS x"), 1).await.unwrap();
    assert!(atlas_pg::matview_detail(&s, &ns, "v").await.is_err());
    assert!(atlas_pg::view_detail(&s, &ns, "mv_full").await.is_err());
    // A leitura de dados e a estrutura (colunas) funcionam como numa tabela.
    let detail = atlas_pg::table_detail(&s, &ns, "mv_full").await.expect("columns of a matview");
    assert_eq!(detail.columns.len(), 1);
    atlas_pg::execute(&s, &format!("DROP SCHEMA {ns} CASCADE"), 1).await.unwrap();
}

#[tokio::test]
async fn function_detail_reads_signature_options_and_overloads() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let ns = format!("atlas_fn_{}", std::process::id());
    atlas_pg::execute(
        &s,
        &format!(
            "DROP SCHEMA IF EXISTS {ns} CASCADE; CREATE SCHEMA {ns}; \
             CREATE FUNCTION {ns}.soma(a integer, b integer DEFAULT 2) RETURNS integer LANGUAGE sql IMMUTABLE STRICT AS $$ SELECT a + b $$; \
             CREATE FUNCTION {ns}.soma(a text) RETURNS text LANGUAGE plpgsql SECURITY DEFINER AS $$ BEGIN RETURN a || '!'; END $$; \
             CREATE FUNCTION {ns}.par(IN x integer, OUT dobro integer, OUT triplo integer) LANGUAGE sql AS $$ SELECT x * 2, x * 3 $$; \
             CREATE FUNCTION {ns}.sem_nome(integer) RETURNS integer LANGUAGE sql STABLE AS $$ SELECT 1 $$; \
             COMMENT ON FUNCTION {ns}.sem_nome(integer) IS 'um'"
        ),
        1,
    )
    .await
    .unwrap();

    let a = atlas_pg::function_detail(&s, &ns, "soma", "a integer, b integer").await.expect("detail");
    assert_eq!(a.language, "sql");
    assert_eq!(a.returns.as_deref(), Some("integer"));
    assert_eq!(a.volatility, "immutable");
    assert!(a.strict && !a.security_definer && !a.procedure);
    assert!(a.arguments.contains("DEFAULT 2"), "{}", a.arguments);
    assert_eq!(a.arg_names, vec!["a", "b"]);
    assert_eq!(a.identity, "a integer, b integer");
    assert!(a.body.contains("a + b"));

    // A sobrecarga com o mesmo nome é outra função.
    let b = atlas_pg::function_detail(&s, &ns, "soma", "a text").await.expect("overload");
    assert_eq!(b.language, "plpgsql");
    assert!(b.security_definer);
    assert_eq!(b.volatility, "volatile");

    let par = atlas_pg::function_detail(&s, &ns, "par", "x integer, OUT dobro integer, OUT triplo integer").await.expect("out params");
    assert!(par.arguments.contains("OUT dobro integer"), "{}", par.arguments);
    assert_eq!(par.arg_names, vec!["x", "dobro", "triplo"]);

    let unnamed = atlas_pg::function_detail(&s, &ns, "sem_nome", "integer").await.expect("unnamed");
    assert_eq!(unnamed.volatility, "stable");
    assert_eq!(unnamed.comment.as_deref(), Some("um"));
    assert!(unnamed.arg_names.iter().all(|n| n.is_empty()), "{:?}", unnamed.arg_names);

    assert!(atlas_pg::function_detail(&s, &ns, "soma", "a bigint").await.is_err());
    if s.server.version_num >= 110000 {
        atlas_pg::execute(&s, &format!("CREATE PROCEDURE {ns}.proc(x integer) LANGUAGE sql AS $$ SELECT 1 $$"), 1).await.unwrap();
        // A forma da identidade varia entre versões (14+ imprime "IN x integer"): usa a que a listagem devolve.
        let listed = atlas_pg::list_objects(&s, &ns).await.unwrap();
        let identity = listed.functions.iter().find(|f| f.name == "proc").and_then(|f| f.args.clone()).expect("listed procedure");
        let pr = atlas_pg::function_detail(&s, &ns, "proc", &identity).await.expect("procedure");
        assert!(pr.procedure && pr.returns.is_none());
    }
    atlas_pg::execute(&s, &format!("DROP SCHEMA {ns} CASCADE"), 1).await.unwrap();
}

#[tokio::test]
async fn sequence_detail_reads_parameters_position_and_owner() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let ns = format!("atlas_sq_{}", std::process::id());
    atlas_pg::execute(
        &s,
        &format!(
            "DROP SCHEMA IF EXISTS {ns} CASCADE; CREATE SCHEMA {ns}; CREATE TABLE {ns}.t(id integer); \
             CREATE SEQUENCE {ns}.s1 INCREMENT BY 5 MINVALUE 10 MAXVALUE 1000 START WITH 20 CACHE 1 CYCLE OWNED BY {ns}.t.id; \
             CREATE SEQUENCE {ns}.s2; CREATE SEQUENCE {ns}.s3 CACHE 3; \
             COMMENT ON SEQUENCE {ns}.s2 IS 'contador'"
        ),
        1,
    )
    .await
    .unwrap();

    let a = atlas_pg::sequence_detail(&s, &ns, "s1").await.expect("detail");
    assert_eq!((a.start.as_str(), a.increment.as_str(), a.min.as_str(), a.max.as_str(), a.cache.as_str()), ("20", "5", "10", "1000", "1"));
    assert!(a.cycle);
    assert_eq!(a.owned_by.as_deref(), Some(format!("{ns}.t.id").as_str()));
    // Nunca usada: o próximo valor é o inicial.
    assert!(!a.is_called);
    assert_eq!(a.last_value, "20");

    // Depois de usar, `last_value` é o último entregue.
    atlas_pg::execute(&s, &format!("SELECT nextval('{ns}.s1'); SELECT nextval('{ns}.s1')"), 1).await.unwrap();
    let a = atlas_pg::sequence_detail(&s, &ns, "s1").await.unwrap();
    assert!(a.is_called);
    assert_eq!(a.last_value, "25");

    let b = atlas_pg::sequence_detail(&s, &ns, "s2").await.unwrap();
    assert_eq!(b.data_type, "bigint");
    assert_eq!((b.start.as_str(), b.increment.as_str(), b.min.as_str(), b.cache.as_str()), ("1", "1", "1", "1"));
    assert_eq!(b.max, "9223372036854775807");
    assert!(!b.cycle && b.owned_by.is_none());
    assert_eq!(b.comment.as_deref(), Some("contador"));

    assert_eq!(atlas_pg::sequence_detail(&s, &ns, "s3").await.unwrap().cache, "3");

    // Uma tabela não é sequência.
    assert!(atlas_pg::sequence_detail(&s, &ns, "t").await.is_err());
    atlas_pg::execute(&s, &format!("DROP SCHEMA {ns} CASCADE"), 1).await.unwrap();
}

#[tokio::test]
async fn user_types_are_listed_and_described() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let ns = format!("atlas_ty_{}", std::process::id());
    atlas_pg::execute(
        &s,
        &format!(
            "DROP SCHEMA IF EXISTS {ns} CASCADE; CREATE SCHEMA {ns}; \
             CREATE TYPE {ns}.cor AS ENUM ('vermelho', 'verde', 'azul'); \
             CREATE TYPE {ns}.ponto AS (x integer, y numeric(10,2), rotulo text); \
             CREATE DOMAIN {ns}.positivo AS integer DEFAULT 1 NOT NULL CONSTRAINT maior_zero CHECK (VALUE > 0); \
             CREATE TYPE {ns}.faixa AS RANGE (subtype = integer); \
             CREATE TABLE {ns}.tabela(id integer); \
             COMMENT ON TYPE {ns}.cor IS 'cores'"
        ),
        1,
    )
    .await
    .unwrap();

    // A listagem traz os 4 tipos, sem o tipo de linha da tabela nem tipos de array.
    let objs = atlas_pg::list_objects(&s, &ns).await.unwrap();
    let listed: Vec<(String, String)> = objs.types.iter().map(|t| (t.name.clone(), t.data_type.clone().unwrap())).collect();
    assert_eq!(
        listed,
        vec![
            ("cor".to_string(), "enum".to_string()),
            ("faixa".to_string(), "range".to_string()),
            ("ponto".to_string(), "composite".to_string()),
            ("positivo".to_string(), "domain".to_string()),
        ]
    );

    let cor = atlas_pg::type_detail(&s, &ns, "cor").await.expect("enum");
    assert_eq!(cor.kind, "enum");
    assert_eq!(cor.labels, ["vermelho", "verde", "azul"]);
    assert_eq!(cor.comment.as_deref(), Some("cores"));

    let ponto = atlas_pg::type_detail(&s, &ns, "ponto").await.expect("composite");
    assert_eq!(ponto.kind, "composite");
    let attrs: Vec<(String, String)> = ponto.attributes.iter().map(|a| (a.name.clone(), a.data_type.clone())).collect();
    assert_eq!(attrs, [("x".into(), "integer".into()), ("y".into(), "numeric(10,2)".into()), ("rotulo".into(), "text".into())]);

    let pos = atlas_pg::type_detail(&s, &ns, "positivo").await.expect("domain");
    assert_eq!(pos.kind, "domain");
    assert_eq!(pos.base_type.as_deref(), Some("integer"));
    assert_eq!(pos.default.as_deref(), Some("1"));
    assert!(pos.not_null);
    assert_eq!(pos.checks.len(), 1);
    assert_eq!(pos.checks[0].name, "maior_zero");
    assert!(pos.checks[0].definition.starts_with("CHECK"), "{}", pos.checks[0].definition);

    let faixa = atlas_pg::type_detail(&s, &ns, "faixa").await.expect("range");
    assert_eq!(faixa.kind, "range");
    assert_eq!(faixa.subtype.as_deref(), Some("integer"));

    // Tabela não é tipo "avulso".
    assert!(atlas_pg::type_detail(&s, &ns, "tabela").await.is_err());
    atlas_pg::execute(&s, &format!("DROP SCHEMA {ns} CASCADE"), 1).await.unwrap();
}

#[tokio::test]
async fn role_detail_reads_attributes_and_memberships() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let suffix = std::process::id();
    let (grupo, usuario) = (format!("atlas_grp_{suffix}"), format!("atlas_usr_{suffix}"));
    atlas_pg::execute(
        &s,
        &format!(
            "DROP ROLE IF EXISTS {usuario}; DROP ROLE IF EXISTS {grupo}; \
             CREATE ROLE {grupo} NOLOGIN; \
             CREATE ROLE {usuario} LOGIN CREATEDB NOINHERIT CONNECTION LIMIT 7 VALID UNTIL '2099-01-01 00:00:00+00' IN ROLE {grupo}; \
             COMMENT ON ROLE {usuario} IS 'conta de teste'"
        ),
        1,
    )
    .await
    .unwrap();

    let u = atlas_pg::role_detail(&s, &usuario).await.expect("user");
    assert!(u.can_login && u.create_db && !u.inherit && !u.superuser && !u.create_role && !u.replication);
    assert_eq!(u.connection_limit, 7);
    assert!(u.valid_until.as_deref().is_some_and(|v| v.starts_with("2098-12-31") || v.starts_with("2099-01-01")), "{:?}", u.valid_until);
    assert_eq!(u.comment.as_deref(), Some("conta de teste"));
    assert_eq!(u.member_of, std::slice::from_ref(&grupo));
    assert!(u.members.is_empty());
    assert_eq!(u.bypass_rls.is_some(), s.server.version_num >= 90500);

    let g = atlas_pg::role_detail(&s, &grupo).await.expect("group");
    assert!(!g.can_login && g.inherit && g.connection_limit == -1 && g.valid_until.is_none());
    assert_eq!(g.members, std::slice::from_ref(&usuario));

    assert!(atlas_pg::role_detail(&s, "atlas_nao_existe_xyz").await.is_err());
    atlas_pg::execute(&s, &format!("DROP ROLE {usuario}; DROP ROLE {grupo}"), 1).await.unwrap();
}

/// Diagrama ER: tabelas com colunas (PK e FK marcadas) e chaves estrangeiras, inclusive composta e para a própria
/// tabela, num schema descartável; views não entram.
#[tokio::test]
async fn schema_diagram_reads_tables_columns_and_foreign_keys() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let ns = format!("atlas_er_{}", std::process::id());
    atlas_pg::execute(
        &s,
        &format!(
            "DROP SCHEMA IF EXISTS {ns} CASCADE; CREATE SCHEMA {ns}; \
             CREATE TABLE {ns}.comp(a integer, b integer, PRIMARY KEY (a, b)); \
             CREATE TABLE {ns}.pai(id integer PRIMARY KEY, nome varchar(30) NOT NULL, avo_id integer REFERENCES {ns}.pai(id)); \
             CREATE TABLE {ns}.filha(id integer PRIMARY KEY, pai_id integer NOT NULL, ca integer, cb integer, criado timestamp(3), \
                CONSTRAINT fk_filha_pai FOREIGN KEY (pai_id) REFERENCES {ns}.pai(id), \
                CONSTRAINT fk_filha_comp FOREIGN KEY (ca, cb) REFERENCES {ns}.comp(a, b)); \
             CREATE VIEW {ns}.v AS SELECT 1 AS x"
        ),
        1,
    )
    .await
    .unwrap();

    let d = atlas_pg::schema_diagram(&s, &ns).await.expect("diagram");
    let names: Vec<&str> = d.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["comp", "filha", "pai"], "só tabelas, em ordem de nome (a view não entra)");

    let filha = d.tables.iter().find(|t| t.name == "filha").unwrap();
    let col = |n: &str| filha.columns.iter().find(|c| c.name == n).unwrap();
    assert!(col("id").is_pk && !col("id").is_fk);
    assert!(col("pai_id").is_fk && !col("pai_id").is_pk);
    assert!(col("ca").is_fk && col("cb").is_fk);
    assert_eq!((col("criado").data_type.as_str(), col("criado").length), ("timestamp", 3));
    let pai = d.tables.iter().find(|t| t.name == "pai").unwrap();
    let nome = pai.columns.iter().find(|c| c.name == "nome").unwrap();
    assert_eq!((nome.data_type.as_str(), nome.length), ("varchar", 30));
    assert_eq!(d.tables.iter().find(|t| t.name == "comp").unwrap().columns.iter().filter(|c| c.is_pk).count(), 2);

    let rel = |n: &str| d.relations.iter().find(|r| r.name == n).unwrap_or_else(|| panic!("relação {n}"));
    let r = rel("fk_filha_pai");
    assert_eq!((r.table.as_str(), r.ref_schema.as_str(), r.ref_table.as_str()), ("filha", ns.as_str(), "pai"));
    assert_eq!((r.columns.as_slice(), r.ref_columns.as_slice()), (&["pai_id".to_string()][..], &["id".to_string()][..]));
    let r = rel("fk_filha_comp");
    assert_eq!(r.columns, ["ca", "cb"]);
    assert_eq!(r.ref_columns, ["a", "b"]);
    assert!(d.relations.iter().any(|r| r.table == "pai" && r.ref_table == "pai"), "a chave para a própria tabela vem na lista");
    // Obrigatória só quando todas as colunas da chave são NOT NULL.
    assert!(rel("fk_filha_pai").required, "pai_id é NOT NULL");
    assert!(!rel("fk_filha_comp").required, "ca e cb aceitam nulo");
    assert!(!d.relations.iter().find(|r| r.table == "pai" && r.ref_table == "pai").unwrap().required, "avo_id aceita nulo");

    atlas_pg::execute(&s, &format!("DROP SCHEMA {ns} CASCADE"), 1).await.unwrap();
}

/// Extensões: lista do banco (com o schema de cada uma), disponíveis para instalar e detalhe de uma instalada.
#[tokio::test]
async fn extensions_are_listed_available_and_described() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");

    // O plpgsql vem em todo banco e vive em pg_catalog; a lista é a mesma em qualquer schema.
    let objs = atlas_pg::list_objects(&s, "public").await.expect("objects");
    let pl = objs.extensions.iter().find(|e| e.name == "plpgsql").expect("plpgsql instalado");
    assert_eq!(pl.schema, "pg_catalog");
    assert!(!pl.version.is_empty() && !pl.owner.is_empty());
    let d = atlas_pg::extension_detail(&s, "plpgsql").await.expect("detail");
    assert_eq!((d.schema.as_str(), d.relocatable), ("pg_catalog", false));
    assert!(atlas_pg::extension_detail(&s, "atlas_nao_existe_xyz").await.is_err());

    // As disponíveis não incluem as já instaladas.
    let avail = atlas_pg::available_extensions(&s).await.expect("available");
    assert!(!avail.iter().any(|a| a.name == "plpgsql"));

    // Instala e remove uma extensão leve, se o servidor a oferece e ela ainda não está no banco
    // (sem permissão para criar extensões, esta parte é pulada).
    if let Some(a) = avail.iter().find(|a| a.name == "pg_trgm")
        && atlas_pg::execute(&s, "CREATE EXTENSION pg_trgm", 1).await.is_ok()
    {
        atlas_pg::execute(&s, "COMMENT ON EXTENSION pg_trgm IS 'busca por similaridade'", 1).await.unwrap();
        let d = atlas_pg::extension_detail(&s, "pg_trgm").await.expect("pg_trgm");
        assert_eq!(d.version, a.default_version);
        assert_eq!(d.comment.as_deref(), Some("busca por similaridade"));
        assert!(d.relocatable);
        let nova = atlas_pg::list_objects(&s, "public").await.expect("objects");
        assert!(nova.extensions.iter().any(|e| e.name == "pg_trgm"));
        atlas_pg::execute(&s, "DROP EXTENSION pg_trgm", 1).await.unwrap();
        assert!(atlas_pg::extension_detail(&s, "pg_trgm").await.is_err());
    }
}

/// "Editar banco": lê os atributos e roda os mesmos comandos que a interface monta (`src/sql/dbedit.ts`),
/// com o `RENAME TO` por último, num banco descartável.
#[tokio::test]
async fn database_detail_reads_attributes_and_edit_commands_apply() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let suffix = std::process::id();
    let (antigo, novo) = (format!("atlas_edb_{suffix}"), format!("atlas_edb_novo_{suffix}"));
    atlas_pg::execute(&s, &format!("DROP DATABASE IF EXISTS \"{antigo}\""), 1).await.unwrap();
    atlas_pg::execute(&s, &format!("DROP DATABASE IF EXISTS \"{novo}\""), 1).await.unwrap();
    atlas_pg::execute(&s, &format!("CREATE DATABASE \"{antigo}\""), 1).await.unwrap();

    let d = atlas_pg::database_detail(&s, &antigo).await.expect("detail");
    assert_eq!(d.name, antigo);
    assert!(!d.owner.is_empty() && !d.encoding.is_empty() && !d.collate.is_empty() && !d.ctype.is_empty() && !d.tablespace.is_empty());
    assert_eq!(d.connection_limit, -1);
    assert!(d.allow_connections && !d.is_template && d.comment.is_none());
    // O banco da própria sessão também é legível.
    assert_eq!(atlas_pg::database_detail(&s, &p.database).await.expect("current").name, p.database);

    // O comentário aparece na lista de bancos da árvore (dica ao passar o mouse); sem comentário, o banco não consta.
    let sem = atlas_pg::list_databases(&s, false).await.expect("databases");
    assert!(sem.databases.contains(&antigo) && !sem.comments.contains_key(&antigo));
    atlas_pg::execute(&s, &format!("COMMENT ON DATABASE \"{antigo}\" IS 'na árvore'"), 1).await.unwrap();
    let com = atlas_pg::list_databases(&s, false).await.expect("databases");
    assert_eq!(com.comments.get(&antigo).map(String::as_str), Some("na árvore"));

    if s.server.version_num >= 90500 {
        atlas_pg::execute(
            &s,
            &format!(
                "ALTER DATABASE \"{antigo}\" WITH CONNECTION LIMIT 5 ALLOW_CONNECTIONS false IS_TEMPLATE true; \
                 COMMENT ON DATABASE \"{antigo}\" IS 'banco de ''teste'''; \
                 ALTER DATABASE \"{antigo}\" RENAME TO \"{novo}\""
            ),
            1,
        )
        .await
        .unwrap();
        let d = atlas_pg::database_detail(&s, &novo).await.expect("renamed");
        assert_eq!(d.connection_limit, 5);
        assert!(!d.allow_connections && d.is_template);
        assert_eq!(d.comment.as_deref(), Some("banco de 'teste'"));
        assert!(atlas_pg::database_detail(&s, &antigo).await.is_err(), "o nome antigo não pode mais existir");
        // Um banco marcado como template não pode ser excluído.
        atlas_pg::execute(&s, &format!("ALTER DATABASE \"{novo}\" WITH IS_TEMPLATE false"), 1).await.unwrap();
        atlas_pg::execute(&s, &format!("DROP DATABASE \"{novo}\""), 1).await.unwrap();
    } else {
        atlas_pg::execute(&s, &format!("DROP DATABASE \"{antigo}\""), 1).await.unwrap();
    }
    assert!(atlas_pg::database_detail(&s, "atlas_nao_existe_xyz").await.is_err());
}

/// "Editar schema": dono, comentário e renomear, num schema descartável do banco do teste.
#[tokio::test]
async fn schema_detail_reads_owner_comment_and_rename_applies() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let suffix = std::process::id();
    let (antigo, novo) = (format!("atlas_esc_{suffix}"), format!("atlas_esc_novo_{suffix}"));
    atlas_pg::execute(&s, &format!("DROP SCHEMA IF EXISTS {antigo} CASCADE; DROP SCHEMA IF EXISTS {novo} CASCADE"), 1).await.unwrap();
    atlas_pg::execute(&s, &format!("CREATE SCHEMA {antigo}; COMMENT ON SCHEMA {antigo} IS 'schema de ''teste'''"), 1).await.unwrap();

    let d = atlas_pg::schema_detail(&s, &antigo).await.expect("detail");
    assert_eq!(d.name, antigo);
    assert!(!d.owner.is_empty());
    assert_eq!(d.comment.as_deref(), Some("schema de 'teste'"));
    // O comentário também vem na lista de schemas da árvore.
    let lista = atlas_pg::list_schemas(&s, false).await.expect("schemas");
    assert_eq!(lista.iter().find(|x| x.name == antigo).and_then(|x| x.comment.as_deref()), Some("schema de 'teste'"));

    atlas_pg::execute(&s, &format!("COMMENT ON SCHEMA {antigo} IS NULL; ALTER SCHEMA {antigo} RENAME TO {novo}"), 1).await.unwrap();
    assert!(atlas_pg::schema_detail(&s, &antigo).await.is_err(), "o nome antigo não pode mais existir");
    let d = atlas_pg::schema_detail(&s, &novo).await.expect("renamed");
    assert!(d.comment.is_none());

    atlas_pg::execute(&s, &format!("DROP SCHEMA {novo}"), 1).await.unwrap();
    assert!(atlas_pg::schema_detail(&s, "atlas_nao_existe_xyz").await.is_err());
}

#[tokio::test]
async fn triggers_are_described_in_full() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let ns = format!("atlas_tg_{}", std::process::id());
    atlas_pg::execute(
        &s,
        &format!(
            "DROP SCHEMA IF EXISTS {ns} CASCADE; CREATE SCHEMA {ns};              CREATE TABLE {ns}.t(id integer, nome text, total numeric);              CREATE FUNCTION {ns}.\"Fn Gatilho\"() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;              CREATE TRIGGER a_antes BEFORE INSERT OR UPDATE OF nome, total ON {ns}.t FOR EACH ROW WHEN (NEW.id > 0)                EXECUTE PROCEDURE {ns}.\"Fn Gatilho\"('x', 'dois, tres');              CREATE TRIGGER b_depois AFTER DELETE OR TRUNCATE ON {ns}.t FOR EACH STATEMENT EXECUTE PROCEDURE {ns}.\"Fn Gatilho\"();              ALTER TABLE {ns}.t DISABLE TRIGGER b_depois;              COMMENT ON TRIGGER a_antes ON {ns}.t IS 'auditoria'"
        ),
        1,
    )
    .await
    .unwrap();

    let d = atlas_pg::table_detail(&s, &ns, "t").await.unwrap();
    assert_eq!(d.triggers.len(), 2);
    let a = &d.triggers[0];
    assert_eq!(a.name, "a_antes");
    assert_eq!(a.timing, "BEFORE");
    assert_eq!(a.events, ["INSERT", "UPDATE"]);
    assert!(a.for_each_row && a.enabled && !a.read_only);
    assert_eq!(a.when.as_deref(), Some("(new.id > 0)"), "{:?}", a.when);
    assert_eq!(a.function, format!("{ns}.\"Fn Gatilho\""));
    assert_eq!(a.args, ["x", "dois, tres"]);
    assert_eq!(a.update_columns, ["nome", "total"]);
    assert_eq!(a.comment.as_deref(), Some("auditoria"));

    let b = &d.triggers[1];
    assert_eq!((b.timing.as_str(), b.for_each_row, b.enabled), ("AFTER", false, false));
    assert_eq!(b.events, ["DELETE", "TRUNCATE"]);
    assert!(b.when.is_none() && b.args.is_empty() && b.update_columns.is_empty());
    atlas_pg::execute(&s, &format!("DROP SCHEMA {ns} CASCADE"), 1).await.unwrap();
}

#[tokio::test]
async fn find_searches_names_columns_definitions_and_comments() {
    use atlas_pg::FindOptions;
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let ns = format!("atlas_fd_{}", std::process::id());
    let other = format!("{ns}_b");
    atlas_pg::execute(
        &s,
        &format!(
            "DROP SCHEMA IF EXISTS {ns} CASCADE; DROP SCHEMA IF EXISTS {other} CASCADE; CREATE SCHEMA {ns}; CREATE SCHEMA {other}; \
             CREATE TABLE {ns}.clientes(id integer, cod_Zeta text, nome text); \
             CREATE VIEW {ns}.v_vendas AS SELECT id FROM {ns}.clientes WHERE nome LIKE 'zeta%'; \
             CREATE FUNCTION {ns}.calcula() RETURNS integer LANGUAGE sql AS $$ SELECT count(*)::integer FROM {ns}.clientes /* ZETA */ $$; \
             CREATE TYPE {ns}.tipo_zeta AS ENUM ('a'); \
             CREATE SEQUENCE {ns}.seq_zeta; \
             COMMENT ON TABLE {ns}.clientes IS 'cadastro de zeta'; \
             COMMENT ON COLUMN {ns}.clientes.nome IS 'nome do ZETA'; \
             CREATE TABLE {other}.zeta_outro(id integer)"
        ),
        1,
    )
    .await
    .unwrap();

    let opts = |text: &str| FindOptions {
        text: text.into(),
        schema: Some(ns.clone()),
        match_case: false,
        regex: false,
        names: true,
        columns: true,
        definitions: true,
        comments: true,
    };
    let run = |o: FindOptions| {
        let s = &s;
        async move { atlas_pg::find(s, &o).await.unwrap() }
    };
    // Tudo que contém "zeta" no schema: nomes (tipo, sequência), coluna, definição de view e função, 2 comentários.
    let all = run(opts("zeta")).await;
    let summary: Vec<(String, String, String)> =
        all.hits.iter().map(|h| (h.kind.clone(), h.name.clone(), format!("{}{}", h.matched_in, h.column.as_deref().map(|c| format!(":{c}")).unwrap_or_default()))).collect();
    for want in [
        ("type", "tipo_zeta", "name"),
        ("sequence", "seq_zeta", "name"),
        ("table", "clientes", "column:cod_zeta"),
        ("view", "v_vendas", "definition"),
        ("function", "calcula", "definition"),
        ("table", "clientes", "comment"),
        ("table", "clientes", "comment:nome"),
    ] {
        assert!(summary.iter().any(|(k, n, m)| (k.as_str(), n.as_str(), m.as_str()) == want), "{want:?} em {summary:?}");
    }
    // O outro schema só entra sem o filtro.
    assert!(all.hits.iter().all(|h| h.schema == ns));
    let mut everywhere = opts("zeta_outro");
    everywhere.schema = None;
    assert!(run(everywhere).await.hits.iter().any(|h| h.schema == other && h.name == "zeta_outro"));

    // O trecho da definição mostra o contexto.
    let f = all.hits.iter().find(|h| h.kind == "function" && h.matched_in == "definition").unwrap();
    assert!(f.snippet.as_deref().unwrap().contains("ZETA"), "{:?}", f.snippet);
    assert_eq!(f.identity.as_deref(), Some(""));

    // Só nomes / diferenciar maiúsculas / regex.
    let mut only_names = opts("zeta");
    only_names.columns = false;
    only_names.definitions = false;
    only_names.comments = false;
    assert!(run(only_names).await.hits.iter().all(|h| h.matched_in == "name"));
    let mut case = opts("ZETA");
    case.match_case = true;
    case.names = false;
    case.columns = false;
    let hits = run(case).await.hits;
    assert!(hits.iter().any(|h| h.matched_in == "definition") && hits.iter().all(|h| h.name != "tipo_zeta"), "{hits:?}");
    let mut regex = opts("^(seq|tipo)_zeta$");
    regex.regex = true;
    regex.columns = false;
    regex.definitions = false;
    regex.comments = false;
    let hits = run(regex).await.hits;
    assert_eq!(hits.len(), 2, "{hits:?}");

    // Curingas do LIKE valem como texto; regex inválida volta como erro do servidor.
    assert!(run(opts("z%a")).await.hits.is_empty());
    let mut bad = opts("(");
    bad.regex = true;
    assert!(atlas_pg::find(&s, &bad).await.is_err());
    // Cada categoria sozinha também funciona (antes os literais sem tipo quebravam a ordenação num servidor 9.6).
    for only in ["names", "columns", "definitions", "comments"] {
        let mut o = opts("zeta");
        (o.names, o.columns, o.definitions, o.comments) = (only == "names", only == "columns", only == "definitions", only == "comments");
        assert!(!run(o).await.hits.is_empty(), "só {only}");
    }
    // Sem texto ou sem nenhuma categoria marcada: nada.
    assert!(run(opts("")).await.hits.is_empty());
    atlas_pg::execute(&s, &format!("DROP SCHEMA {ns} CASCADE; DROP SCHEMA {other} CASCADE"), 1).await.unwrap();
}

/// Despeja o schema, restaura num banco novo e despeja de novo: os dois dumps (estrutura e dados) precisam ser iguais.
#[tokio::test]
async fn sql_dump_round_trips_through_a_fresh_database() {
    use atlas_pg::{DumpOptions, DumpScope};
    use futures_util::SinkExt;
    let p = require_server!();
    let src = atlas_pg::connect(&p, None).await.expect("connect");
    let v = src.server.version_num;
    let ns = format!("atlas_dp_{}", std::process::id());
    let target_db = format!("atlas_dprt_{}", std::process::id());
    let mut ddl = format!(
        "DROP SCHEMA IF EXISTS {ns} CASCADE; CREATE SCHEMA {ns}; \
         CREATE TYPE {ns}.cor AS ENUM ('vermelho', 'azul'); \
         CREATE TYPE {ns}.ponto AS (x integer, y numeric(10,2)); \
         CREATE DOMAIN {ns}.positivo AS integer NOT NULL DEFAULT 1 CHECK (VALUE > 0); \
         CREATE FUNCTION {ns}.dobro(a integer) RETURNS integer LANGUAGE sql IMMUTABLE AS $$ SELECT a * 2 $$; \
         CREATE FUNCTION {ns}.audita() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN NEW.nota := coalesce(NEW.nota, '') || '!'; RETURN NEW; END $$; \
         CREATE TABLE {ns}.clientes(id serial PRIMARY KEY, nome text NOT NULL, cor {ns}.cor DEFAULT 'azul', total numeric(12,2), qtd {ns}.positivo, \
                                    nota text, criado timestamptz DEFAULT '2026-01-02 03:04:05+00', CONSTRAINT nome_ok CHECK (length(nome) > 1), UNIQUE (nome)); \
         CREATE UNLOGGED TABLE {ns}.pedidos(id integer PRIMARY KEY, cliente_id integer NOT NULL REFERENCES {ns}.clientes(id) ON DELETE CASCADE, valor numeric); \
         CREATE INDEX pedidos_valor_idx ON {ns}.pedidos (valor) WHERE valor > 0; \
         CREATE TRIGGER clientes_audita BEFORE INSERT ON {ns}.clientes FOR EACH ROW EXECUTE PROCEDURE {ns}.audita(); \
         CREATE VIEW {ns}.v_base AS SELECT id, nome FROM {ns}.clientes; \
         CREATE VIEW {ns}.v_derivada AS SELECT id FROM {ns}.v_base WHERE id > 0; \
         CREATE MATERIALIZED VIEW {ns}.m_totais AS SELECT cliente_id, sum(valor) AS soma FROM {ns}.pedidos GROUP BY 1; \
         CREATE INDEX m_totais_idx ON {ns}.m_totais (cliente_id); \
         COMMENT ON TABLE {ns}.clientes IS 'cadastro \\ de ''clientes'''; \
         COMMENT ON COLUMN {ns}.clientes.nome IS 'nome completo'; \
         COMMENT ON FUNCTION {ns}.dobro(integer) IS 'dobra'; \
         COMMENT ON VIEW {ns}.v_base IS 'base'; \
         CREATE SEQUENCE {ns}.avulsa INCREMENT BY 3 START WITH 10; SELECT nextval('{ns}.avulsa'); \
         CREATE TABLE {ns}.pai(id integer, grupo text); CREATE TABLE {ns}.filha(extra integer) INHERITS ({ns}.pai)"
    );
    if v >= 100000 {
        ddl.push_str(&format!(
            "; CREATE TABLE {ns}.medidas(dia date NOT NULL, valor integer) PARTITION BY RANGE (dia); \
             CREATE TABLE {ns}.medidas_2026 PARTITION OF {ns}.medidas FOR VALUES FROM ('2026-01-01') TO ('2027-01-01'); \
             CREATE TABLE {ns}.itens(id integer GENERATED ALWAYS AS IDENTITY, nome text)"
        ));
    }
    if v >= 120000 {
        ddl.push_str(&format!("; CREATE TABLE {ns}.calc(a integer, b integer GENERATED ALWAYS AS (a * 2) STORED)"));
    }
    atlas_pg::execute(&src, &ddl, 1).await.expect("create source");
    atlas_pg::execute(
        &src,
        &format!(
            "INSERT INTO {ns}.clientes(nome, total, qtd, nota) VALUES ('Ana', 10.5, 3, E'linha1\\nlinha2\\ttab'), ('João \\ barra', NULL, 7, NULL), ('Zé', 0, 1, 'x'); \
             INSERT INTO {ns}.pedidos VALUES (1, 1, 5), (2, 1, 7.5), (3, 2, NULL); \
             REFRESH MATERIALIZED VIEW {ns}.m_totais; \
             INSERT INTO {ns}.pai VALUES (1, 'a'); INSERT INTO {ns}.filha VALUES (2, 'b', 9)"
        ),
        1,
    )
    .await
    .unwrap();
    if v >= 100000 {
        atlas_pg::execute(&src, &format!("INSERT INTO {ns}.medidas VALUES ('2026-05-05', 3); INSERT INTO {ns}.itens(nome) VALUES ('um'), ('dois')"), 1).await.unwrap();
    }
    if v >= 120000 {
        atlas_pg::execute(&src, &format!("INSERT INTO {ns}.calc(a) VALUES (4)"), 1).await.unwrap();
    }

    let opts = DumpOptions { scope: DumpScope::Schema { schema: ns.clone() }, structure: true, data: true, clean: false, include_owner: false };
    // Despeja uma conexão: estrutura, dados (COPY) e depois dos dados, na ordem em que o arquivo é gravado.
    async fn dump_text(s: &atlas_pg::PgSession, opts: &DumpOptions) -> (String, atlas_pg::DumpPlan, Vec<(String, Vec<u8>)>) {
        atlas_pg::begin_snapshot(s).await.unwrap();
        let plan = atlas_pg::dump_plan(s, opts).await.expect("plan");
        let mut data = Vec::new();
        for t in &plan.data_tables {
            let mut bytes = Vec::new();
            atlas_pg::copy_table(s, t, |chunk, _| {
                bytes.extend_from_slice(chunk);
                Ok(true)
            })
            .await
            .unwrap();
            data.push((t.quoted.clone(), bytes));
        }
        atlas_pg::end_snapshot(s).await;
        let mut text = plan.pre_data.join("\n\n");
        for (q, d) in &data {
            text.push_str(&format!("\n\n-- {q}\n{}", String::from_utf8_lossy(d)));
        }
        text.push_str(&plan.post_data.join("\n\n"));
        (text, plan, data)
    }
    let (text1, plan1, data1) = dump_text(&src, &opts).await;
    assert!(plan1.objects >= 12, "objetos: {}", plan1.objects);
    assert!(plan1.pre_data.iter().any(|x| x.starts_with("CREATE TABLE")), "{text1}");
    assert!(text1.contains("REFRESH MATERIALIZED VIEW"), "{text1}");
    // v_base vem antes de v_derivada (dependência).
    let base_pos = text1.find(&format!("CREATE VIEW {ns}.v_base")).unwrap();
    let derived_pos = text1.find(&format!("CREATE VIEW {ns}.v_derivada")).unwrap();
    assert!(base_pos < derived_pos, "views fora de ordem");
    assert!(text1.contains("cadastro \\ de ''clientes''"), "comentário com aspas e barra: {text1}");

    // Restaura num banco novo: SET, estrutura, dados (COPY), depois dos dados.
    atlas_pg::execute(&src, &format!("DROP DATABASE IF EXISTS {target_db}"), 1).await.unwrap();
    atlas_pg::execute(&src, &format!("CREATE DATABASE {target_db}"), 1).await.expect("create target db");
    let dst = atlas_pg::connect(&p, Some(&target_db)).await.expect("connect target");
    atlas_pg::execute(&dst, "SET check_function_bodies = false; SET standard_conforming_strings = on; SELECT pg_catalog.set_config('search_path', '', false)", 1).await.unwrap();
    for stmt in &plan1.pre_data {
        atlas_pg::execute(&dst, stmt, 1).await.unwrap_or_else(|e| panic!("pre-data falhou: {e:?}\n{stmt}"));
    }
    for (t, (_, bytes)) in plan1.data_tables.iter().zip(&data1) {
        let sql = atlas_pg::copy_header(t);
        let sink = dst.client.copy_in(sql.trim_end().trim_end_matches(';')).await.expect("copy_in");
        futures_util::pin_mut!(sink);
        sink.send(bytes::Bytes::from(bytes.clone())).await.unwrap();
        sink.close().await.unwrap_or_else(|e| panic!("COPY de {} falhou: {e:?}", t.quoted));
    }
    for stmt in &plan1.post_data {
        if stmt.starts_with("--") {
            continue;
        }
        atlas_pg::execute(&dst, stmt, 1).await.unwrap_or_else(|e| panic!("post-data falhou: {e:?}\n{stmt}"));
    }
    let (text2, _, data2) = dump_text(&dst, &opts).await;
    drop(dst);
    atlas_pg::execute(&src, &format!("DROP DATABASE {target_db}"), 1).await.unwrap();
    atlas_pg::execute(&src, &format!("DROP SCHEMA {ns} CASCADE"), 1).await.unwrap();
    // O segundo dump é idêntico ao primeiro: nada se perdeu na ida e na volta (dados incluídos).
    let a: Vec<&str> = text1.lines().collect();
    let b: Vec<&str> = text2.lines().collect();
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        assert_eq!(x, y, "diferença na linha {i}");
    }
    assert_eq!(a.len(), b.len());
    assert_eq!(data1, data2);
}

/// Roda uma importação com linhas fixas; devolve as contagens e o registro.
async fn run_import_rows(
    s: &atlas_pg::PgSession,
    spec: &atlas_pg::ImportSpec,
    rows: Vec<atlas_pg::Feed>,
    cancel_after: Option<usize>,
) -> (Result<atlas_pg::ImportStats, atlas_core::DatabaseError>, Vec<String>) {
    let mut log = Vec::new();
    let mut iter = rows.into_iter();
    let mut feed = || iter.next().unwrap_or(atlas_pg::Feed::End);
    let result = {
        let mut on_log = |l: String| log.push(l);
        let mut on_progress = |_: &atlas_pg::ImportStats| {};
        let count = std::sync::atomic::AtomicUsize::new(0);
        let cancelled = || cancel_after.is_some_and(|n| count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1 > n);
        atlas_pg::run_import(s, spec, &mut feed, atlas_pg::ImportHooks { log: &mut on_log, progress: &mut on_progress, cancelled: &cancelled }).await
    };
    (result, log)
}

/// Conteúdo da tabela `t` do teste, numa linha só (ordenado por id).
async fn dump_t(s: &atlas_pg::PgSession, ns: &str) -> String {
    let sql = format!("SELECT string_agg(id || ':' || coalesce(nome, '~') || ':' || coalesce(valor::text, '~') || ':' || coalesce(ativo::text, '~'), ' | ' ORDER BY id) FROM {ns}.t");
    atlas_pg::execute(s, &sql, 10).await.unwrap().statements[0].result.as_ref().unwrap().rows[0][0].clone().unwrap_or_default()
}

fn import_row(values: &[Option<&str>]) -> atlas_pg::Feed {
    atlas_pg::Feed::Row(values.iter().map(|v| v.map(str::to_string)).collect())
}

#[tokio::test]
async fn import_engine_modes_errors_and_rollback() {
    use atlas_pg::{Feed, ImportMode, ImportSpec, TargetColumn};
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let ns = format!("atlas_im_{}", std::process::id());
    atlas_pg::execute(
        &s,
        &format!(
            "DROP SCHEMA IF EXISTS {ns} CASCADE; CREATE SCHEMA {ns}; \
             CREATE TABLE {ns}.t(id integer PRIMARY KEY, nome text, valor numeric(10,2), ativo boolean, quando timestamptz)"
        ),
        1,
    )
    .await
    .unwrap();
    if s.server.version_num >= 100000 {
        atlas_pg::execute(&s, &format!("CREATE TABLE {ns}.ident(id integer GENERATED ALWAYS AS IDENTITY, nome text)"), 1).await.unwrap();
    }
    let col = |n: &str, t: &str| TargetColumn { name: n.into(), cast_type: t.into(), identity_always: false };
    let cols = || vec![col("id", "integer"), col("nome", "text"), col("valor", "numeric(10,2)"), col("ativo", "boolean"), col("quando", "timestamp with time zone")];
    let spec = |mode: ImportMode| ImportSpec {
        schema: ns.clone(),
        table: "t".into(),
        create: None,
        columns: cols(),
        key_columns: vec![0],
        mode,
        extended_insert: false,
        max_statement_bytes: 1024 * 1024,
        continue_on_error: false,
        manage_transaction: true,
    };
    // Acrescentar (um INSERT por linha), com conversão de tipos pelo servidor.
    let rows = vec![
        import_row(&[Some("1"), Some("Ana"), Some("10.5"), Some("true"), Some("2026-01-15 07:30:00-03")]),
        import_row(&[Some("2"), Some("Zé"), None, Some("false"), None]),
        import_row(&[Some("3"), None, Some("3"), None, None]),
    ];
    let (r, _) = run_import_rows(&s, &spec(ImportMode::Append), rows, None).await;
    let st = r.unwrap();
    assert_eq!((st.processed, st.added, st.errors, st.rolled_back), (3, 3, 0, false));
    assert_eq!(dump_t(&s, &ns).await, "1:Ana:10.50:true | 2:Zé:~:false | 3:~:3.00:~");

    // Erro sem "continuar": a primeira linha duplicada desfaz tudo.
    let rows = vec![
        import_row(&[Some("10"), Some("novo"), None, None, None]),
        import_row(&[Some("1"), Some("duplicada"), None, None, None]),
    ];
    let (r, log) = run_import_rows(&s, &spec(ImportMode::Append), rows, None).await;
    let st = r.unwrap();
    assert!(st.rolled_back && st.added == 0 && st.errors == 1, "{st:?}");
    assert!(log.iter().any(|l| l.contains("row 2")), "{log:?}");
    assert_eq!(dump_t(&s, &ns).await, "1:Ana:10.50:true | 2:Zé:~:false | 3:~:3.00:~", "nada mudou");

    // Continuar em erro, linha a linha e em INSERT estendido: a duplicada e a de valor inválido são puladas.
    for extended in [false, true] {
        atlas_pg::execute(&s, &format!("DELETE FROM {ns}.t WHERE id >= 10"), 1).await.unwrap();
        let mut sp = spec(ImportMode::Append);
        sp.continue_on_error = true;
        sp.extended_insert = extended;
        let rows = vec![
            import_row(&[Some("10"), Some("a"), Some("1"), None, None]),
            import_row(&[Some("1"), Some("duplicada"), None, None, None]),
            import_row(&[Some("11"), Some("b"), Some("abc"), None, None]),
            import_row(&[Some("12"), Some("c"), Some("2"), None, None]),
            Feed::Bad("data inválida: \"ontem\"".into()),
        ];
        let (r, log) = run_import_rows(&s, &sp, rows, None).await;
        let st = r.unwrap();
        assert_eq!((st.processed, st.added, st.errors, st.rolled_back), (5, 2, 3, false), "extended={extended}: {st:?} {log:?}");
        assert!(log.iter().filter(|l| l.starts_with("[ERR]")).count() == 3, "{log:?}");
        assert_eq!(dump_t(&s, &ns).await, "1:Ana:10.50:true | 2:Zé:~:false | 3:~:3.00:~ | 10:a:1.00:~ | 12:c:2.00:~", "extended={extended}");
    }
    atlas_pg::execute(&s, &format!("DELETE FROM {ns}.t WHERE id >= 10"), 1).await.unwrap();

    // Muitas linhas em INSERT estendido: o lote respeita o limite de tamanho.
    let mut sp = spec(ImportMode::Append);
    sp.extended_insert = true;
    sp.max_statement_bytes = 2000;
    let rows: Vec<Feed> = (100..1100).map(|i| import_row(&[Some(&i.to_string()), Some("x"), Some("1"), None, None])).collect();
    let st = run_import_rows(&s, &sp, rows, None).await.0.unwrap();
    assert_eq!((st.processed, st.added, st.errors), (1000, 1000, 0));
    atlas_pg::execute(&s, &format!("DELETE FROM {ns}.t WHERE id >= 10"), 1).await.unwrap();

    // Atualizar: casa pela chave; a que não existe não conta.
    let rows = vec![
        import_row(&[Some("1"), Some("Ana B"), Some("11"), Some("false"), None]),
        import_row(&[Some("3"), Some("Três"), None, None, None]),
        import_row(&[Some("99"), Some("sumida"), None, None, None]),
    ];
    let st = run_import_rows(&s, &spec(ImportMode::Update), rows, None).await.0.unwrap();
    assert_eq!((st.processed, st.updated, st.added), (3, 2, 0));
    assert_eq!(dump_t(&s, &ns).await, "1:Ana B:11.00:false | 2:Zé:~:false | 3:Três:~:~");

    // Acrescentar/atualizar e acrescentar sem atualizar.
    let rows = || vec![import_row(&[Some("2"), Some("Zé 2"), Some("7"), None, None]), import_row(&[Some("4"), Some("quatro"), None, None, None])];
    let st = run_import_rows(&s, &spec(ImportMode::AppendUpdate), rows(), None).await.0.unwrap();
    assert_eq!((st.updated, st.added), (1, 1));
    assert_eq!(dump_t(&s, &ns).await, "1:Ana B:11.00:false | 2:Zé 2:7.00:~ | 3:Três:~:~ | 4:quatro:~:~");
    let rows = vec![import_row(&[Some("4"), Some("ignorada"), None, None, None]), import_row(&[Some("5"), Some("cinco"), None, None, None])];
    let st = run_import_rows(&s, &spec(ImportMode::AppendNoUpdate), rows, None).await.0.unwrap();
    assert_eq!((st.updated, st.added), (0, 1));
    assert!(dump_t(&s, &ns).await.ends_with("4:quatro:~:~ | 5:cinco:~:~"));

    // Excluir pelas chaves.
    let rows = vec![import_row(&[Some("4"), None, None, None, None]), import_row(&[Some("5"), None, None, None, None]), import_row(&[Some("77"), None, None, None, None])];
    let st = run_import_rows(&s, &spec(ImportMode::Delete), rows, None).await.0.unwrap();
    assert_eq!((st.processed, st.deleted), (3, 2));

    // Copiar: apaga tudo e repovoa.
    let rows = vec![import_row(&[Some("50"), Some("só esta"), None, None, None])];
    let (r, log) = run_import_rows(&s, &spec(ImportMode::Copy), rows, None).await;
    let st = r.unwrap();
    assert_eq!((st.deleted, st.added), (3, 1), "{log:?}");
    assert_eq!(dump_t(&s, &ns).await, "50:só esta:~:~");

    // Modos que precisam de chave, sem chave: erro claro e nada gravado.
    let mut nokey = spec(ImportMode::Update);
    nokey.key_columns.clear();
    let (r, log) = run_import_rows(&s, &nokey, vec![import_row(&[Some("1"), None, None, None, None])], None).await;
    assert!(r.unwrap().rolled_back && log.iter().any(|l| l.contains("key field")), "{log:?}");

    // Cancelar no meio desfaz e devolve Canceled.
    let rows: Vec<Feed> = (200..260).map(|i| import_row(&[Some(&i.to_string()), None, None, None, None])).collect();
    let (r, _) = run_import_rows(&s, &spec(ImportMode::Append), rows, Some(10)).await;
    assert_eq!(r.unwrap_err().kind, atlas_core::ErrorKind::Canceled);
    assert_eq!(dump_t(&s, &ns).await, "50:só esta:~:~");

    // Tabela nova criada na mesma transação.
    let created = ImportSpec {
        schema: ns.clone(),
        table: "nova".into(),
        create: Some(vec![("codigo".into(), "integer".into()), ("texto".into(), "text".into())]),
        columns: vec![col("codigo", "integer"), col("texto", "text")],
        key_columns: vec![],
        mode: ImportMode::Append,
        extended_insert: true,
        max_statement_bytes: 1024,
        continue_on_error: false,
        manage_transaction: true,
    };
    let rows = vec![import_row(&[Some("1"), Some("a")]), import_row(&[Some("2"), Some("b")])];
    let st = run_import_rows(&s, &created, rows, None).await.0.unwrap();
    assert_eq!(st.added, 2);
    let n = atlas_pg::execute(&s, &format!("SELECT count(*) FROM {ns}.nova"), 1).await.unwrap().statements[0].result.as_ref().unwrap().rows[0][0].clone();
    assert_eq!(n.as_deref(), Some("2"));
    // Se a importação falha, a tabela nova também não fica.
    let mut failing = created.clone();
    failing.table = "nova2".into();
    let (r, _) = run_import_rows(&s, &failing, vec![import_row(&[Some("x"), Some("a")])], None).await;
    assert!(r.unwrap().rolled_back);
    let exists = atlas_pg::execute(&s, &format!("SELECT to_regclass('{ns}.nova2') IS NOT NULL"), 1).await.unwrap().statements[0].result.as_ref().unwrap().rows[0][0].clone();
    assert_eq!(exists.as_deref(), Some("f"));

    // Identidade GENERATED ALWAYS: aceita o valor do arquivo.
    if s.server.version_num >= 100000 {
        let ident = ImportSpec {
            schema: ns.clone(),
            table: "ident".into(),
            create: None,
            columns: vec![TargetColumn { name: "id".into(), cast_type: "integer".into(), identity_always: true }, col("nome", "text")],
            key_columns: vec![],
            mode: ImportMode::Append,
            extended_insert: false,
            max_statement_bytes: 1024,
            continue_on_error: false,
            manage_transaction: true,
        };
        let st = run_import_rows(&s, &ident, vec![import_row(&[Some("7"), Some("sete")])], None).await.0.unwrap();
        assert_eq!(st.added, 1);
    }
    atlas_pg::execute(&s, &format!("DROP SCHEMA {ns} CASCADE"), 1).await.unwrap();
}

#[tokio::test]
async fn dump_selection_only_includes_chosen_objects() {
    use atlas_pg::{DumpOptions, DumpScope};
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let ns = format!("atlas_sel_{}", std::process::id());
    atlas_pg::execute(
        &s,
        &format!(
            "DROP SCHEMA IF EXISTS {ns} CASCADE; CREATE SCHEMA {ns}; \
             CREATE TYPE {ns}.cor AS ENUM ('a', 'b'); CREATE TYPE {ns}.ponto AS (x integer); \
             CREATE FUNCTION {ns}.dobro(a integer) RETURNS integer LANGUAGE sql AS $$ SELECT a * 2 $$; \
             CREATE FUNCTION {ns}.triplo(a integer) RETURNS integer LANGUAGE sql AS $$ SELECT a * 3 $$; \
             CREATE TABLE {ns}.clientes(id serial PRIMARY KEY, nome text); CREATE TABLE {ns}.outra(id integer); \
             INSERT INTO {ns}.clientes(nome) VALUES ('Ana'); INSERT INTO {ns}.outra VALUES (1); \
             CREATE VIEW {ns}.v_ok AS SELECT id FROM {ns}.clientes; CREATE VIEW {ns}.v_fora AS SELECT id FROM {ns}.outra; \
             CREATE SEQUENCE {ns}.avulsa; CREATE SEQUENCE {ns}.fora_seq"
        ),
        1,
    )
    .await
    .expect("create source");
    let opts = DumpOptions {
        scope: DumpScope::Selection {
            schema: ns.clone(),
            tables: vec!["clientes".into()],
            views: vec!["v_ok".into()],
            functions: vec!["dobro(a integer)".into()],
            sequences: vec!["avulsa".into()],
            types: vec!["cor".into()],
        },
        structure: true,
        data: true,
        clean: false,
        include_owner: false,
    };
    atlas_pg::begin_snapshot(&s).await.unwrap();
    let plan = atlas_pg::dump_plan(&s, &opts).await.expect("plan");
    atlas_pg::end_snapshot(&s).await;
    let text = format!("{}\n{}", plan.pre_data.join("\n"), plan.post_data.join("\n"));
    assert!(text.contains(&format!("CREATE TYPE {ns}.cor")), "{text}");
    assert!(!text.contains("ponto"), "tipo não escolhido entrou: {text}");
    assert!(text.contains("dobro") && !text.contains("triplo"), "{text}");
    assert!(text.contains(&format!("CREATE TABLE {ns}.clientes")) && !text.contains("outra"), "{text}");
    assert!(text.contains("v_ok") && !text.contains("v_fora"), "{text}");
    assert!(text.contains(&format!("CREATE SEQUENCE {ns}.avulsa")) && !text.contains("fora_seq"), "{text}");
    assert!(text.contains("clientes_id_seq"), "a sequência da tabela escolhida deve entrar: {text}");
    assert!(!text.contains("CREATE SCHEMA"), "a seleção não cria o schema: {text}");
    assert_eq!(plan.data_tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["clientes"]);
    atlas_pg::execute(&s, &format!("DROP SCHEMA {ns} CASCADE"), 1).await.unwrap();
}

#[tokio::test]
async fn transfer_copies_objects_between_schemas_and_servers() {
    use atlas_pg::{TableOverride, TransferHooks, TransferSpec};
    let p = require_server!();
    let src = atlas_pg::connect(&p, None).await.expect("connect source");
    // Destino: outra porta se `ATLAS_TEST_PG_TARGET_PORT` existir (cruza 9.6 e 18); senão o mesmo servidor.
    let tp = target_params(&p);
    let dst = atlas_pg::connect(&tp, None).await.expect("connect target");
    let pid = std::process::id();
    let (a, b) = (format!("atlas_tr_a_{pid}"), format!("atlas_tr_b_{pid}"));
    atlas_pg::execute(
        &src,
        &format!(
            "DROP SCHEMA IF EXISTS {a} CASCADE; CREATE SCHEMA {a}; \
             CREATE TYPE {a}.cor AS ENUM ('azul', 'verde'); \
             CREATE FUNCTION {a}.dobro(x integer) RETURNS integer LANGUAGE sql AS $$ SELECT x * 2 $$; \
             CREATE TABLE {a}.clientes(id serial PRIMARY KEY, nome text NOT NULL, cor {a}.cor DEFAULT 'azul', nota text, total numeric(12,2)); \
             CREATE FUNCTION {a}.quantos() RETURNS bigint LANGUAGE sql AS $$ SELECT count(*) FROM {a}.clientes $$; \
             CREATE TABLE {a}.pedidos(id integer PRIMARY KEY, cliente_id integer NOT NULL REFERENCES {a}.clientes(id), valor numeric); \
             CREATE INDEX pedidos_valor_idx ON {a}.pedidos (valor); \
             CREATE VIEW {a}.v_base AS SELECT id, nome FROM {a}.clientes; \
             CREATE SEQUENCE {a}.avulsa START WITH 10 INCREMENT BY 3; SELECT nextval('{a}.avulsa'); \
             INSERT INTO {a}.clientes(nome, cor, nota, total) VALUES ('Ana', 'verde', E'linha1\\nlinha2\\ttab \\\\ barra', 10.5), ('João ''Zé''', 'azul', NULL, NULL), ('日本', 'azul', '', 0); \
             INSERT INTO {a}.pedidos VALUES (1, 1, 5), (2, 1, 7.5), (3, 2, NULL)"
        ),
        1,
    )
    .await
    .expect("create source");
    atlas_pg::execute(&dst, &format!("DROP SCHEMA IF EXISTS {b} CASCADE"), 1).await.unwrap();

    let mut spec = TransferSpec {
        source_schema: a.clone(),
        target_schema: b.clone(),
        tables: vec!["clientes".into(), "pedidos".into()],
        views: vec!["v_base".into()],
        functions: vec!["dobro".into(), "quantos".into()],
        sequences: vec!["avulsa".into()],
        types: vec!["cor".into()],
        overrides: vec![],
        drop_before_create: false,
        continue_on_error: false,
        structure: true,
        data: true,
    };
    async fn go(src: &atlas_pg::PgSession, dst: &atlas_pg::PgSession, spec: &TransferSpec) -> (atlas_pg::TransferStats, Vec<String>) {
        let mut log = Vec::new();
        let mut progress = 0usize;
        let stats = {
            let mut on_log = |l: String| log.push(l);
            let mut on_progress = |_: &atlas_pg::TransferStats| progress += 1;
            atlas_pg::run_transfer(src, dst, spec, TransferHooks { log: &mut on_log, progress: &mut on_progress, cancelled: &|| false }).await.expect("transfer")
        };
        assert!(progress > 0, "o progresso deve ser avisado");
        (stats, log)
    }
    async fn text(s: &atlas_pg::PgSession, sql: String) -> String {
        atlas_pg::execute(s, &sql, 10).await.unwrap().statements[0].result.as_ref().unwrap().rows[0][0].clone().unwrap_or_default()
    }
    let (stats, log) = go(&src, &dst, &spec).await;
    assert_eq!(stats.errors, 0, "{log:#?}");
    assert_eq!(stats.transferred, 6, "{log:#?}");
    let rows_sql = |ns: &str| format!("SELECT string_agg(id || ':' || nome || ':' || cor || ':' || coalesce(nota, '~') || ':' || coalesce(total::text, '~'), ' | ' ORDER BY id) FROM {ns}.clientes");
    assert_eq!(text(&dst, rows_sql(&b)).await, text(&src, rows_sql(&a)).await);
    assert_eq!(text(&dst, format!("SELECT count(*) FROM {b}.pedidos")).await, "3");
    assert_eq!(text(&dst, format!("SELECT count(*) FROM {b}.v_base")).await, "3");
    assert_eq!(text(&dst, format!("SELECT {b}.dobro(4)")).await, "8");
    // A função que lê uma tabela foi criada antes dela e funciona (o corpo aponta para o schema de destino).
    assert_eq!(text(&dst, format!("SELECT {b}.quantos()")).await, "3");
    assert_eq!(text(&dst, format!("SELECT count(*) FROM pg_constraint c JOIN pg_namespace n ON n.oid = c.connamespace WHERE n.nspname = '{b}' AND c.contype = 'f'")).await, "1");
    assert_eq!(text(&dst, format!("SELECT count(*) FROM pg_indexes WHERE schemaname = '{b}' AND indexname = 'pedidos_valor_idx'")).await, "1");
    // A sequência avulsa continua de onde a origem parou.
    assert_eq!(text(&dst, format!("SELECT nextval('{b}.avulsa')")).await, text(&src, format!("SELECT nextval('{a}.avulsa')")).await);
    // A sequência da coluna serial também (próximo id = 4).
    assert_eq!(text(&dst, format!("SELECT nextval('{b}.clientes_id_seq')")).await, "4");

    // Sem "apagar antes", repetir bate nas tabelas que já existem: erro e nada pela metade.
    let (stats, log) = go(&src, &dst, &spec).await;
    assert!(stats.errors > 0 && log.iter().any(|l| l.starts_with("[ERR]")), "{log:#?}");
    assert_eq!(text(&dst, format!("SELECT count(*) FROM {b}.clientes")).await, "3");

    // Apagando antes de criar, repete sem duplicar; filtro e lote por tabela.
    spec.drop_before_create = true;
    spec.overrides = vec![TableOverride { table: "pedidos".into(), rows_per_batch: Some(1), recordsets: Some(vec!["valor > 5".into()]), ..Default::default() }];
    let (stats, log) = go(&src, &dst, &spec).await;
    assert_eq!(stats.errors, 0, "{log:#?}");
    assert_eq!(text(&dst, format!("SELECT count(*) FROM {b}.clientes")).await, "3");
    assert_eq!(text(&dst, format!("SELECT count(*) FROM {b}.pedidos")).await, "1");

    // Só estrutura.
    spec.data = false;
    spec.overrides = vec![];
    let (stats, _) = go(&src, &dst, &spec).await;
    assert_eq!(stats.transferred, 0);
    assert_eq!(text(&dst, format!("SELECT count(*) FROM {b}.clientes")).await, "0");

    atlas_pg::execute(&src, &format!("DROP SCHEMA {a} CASCADE"), 1).await.unwrap();
    atlas_pg::execute(&dst, &format!("DROP SCHEMA {b} CASCADE"), 1).await.unwrap();
}

#[tokio::test]
async fn data_sync_compares_scripts_and_deploys() {
    use atlas_pg::{CompareHooks, DeployHooks, DiffKind, SyncSpec, SyncTable, TableSelection};
    let p = require_server!();
    let src = atlas_pg::connect(&p, None).await.expect("connect source");
    let tp = target_params(&p);
    let dst = atlas_pg::connect(&tp, None).await.expect("connect target");
    let pid = std::process::id();
    let (a, b) = (format!("atlas_ds_a_{pid}"), format!("atlas_ds_b_{pid}"));
    let ddl = |ns: &str| {
        format!(
            "DROP SCHEMA IF EXISTS {ns} CASCADE; CREATE SCHEMA {ns}; \
             CREATE TABLE {ns}.t(id integer PRIMARY KEY, nome text, valor numeric(10,2), f double precision, ativo boolean, nota text, ts timestamptz); \
             CREATE TABLE {ns}.sempk(x integer); \
             CREATE TABLE {ns}.comp(a integer, b text, v text, PRIMARY KEY (a, b));"
        )
    };
    atlas_pg::execute(&src, &ddl(&a), 1).await.expect("source ddl");
    atlas_pg::execute(&dst, &ddl(&b), 1).await.expect("target ddl");
    atlas_pg::execute(
        &src,
        &format!(
            "INSERT INTO {a}.t VALUES (1, 'Ana', 10.50, 0.1, true, 'x', '2026-01-02 03:04:05+00'), (2, 'Bruno', 20, 0.2, false, NULL, '2026-02-02 00:00:00+00'), \
             (3, 'Carla', NULL, NULL, NULL, E'linha1\\nlinha2 \\\\ barra ''aspas''', NULL), (4, 'Novo', 4.40, 4.4, true, 'inserir', '2026-04-04 04:04:04+00'); \
             INSERT INTO {a}.sempk VALUES (1); \
             INSERT INTO {a}.comp VALUES (1, 'x', 'p'), (1, 'y', 'q')"
        ),
        1,
    )
    .await
    .unwrap();
    atlas_pg::execute(
        &dst,
        &format!(
            "INSERT INTO {b}.t VALUES (1, 'Ana', 10.50, 0.1, true, 'x', '2026-01-02 03:04:05+00'), (2, 'Bruno X', 21, 0.2, false, NULL, '2026-02-02 00:00:00+00'), \
             (3, 'Carla', NULL, NULL, NULL, E'linha1\\nlinha2 \\\\ barra ''aspas''', NULL), (5, 'Sobra', 5, 5, false, NULL, NULL); \
             INSERT INTO {b}.comp VALUES (1, 'x', 'p'), (2, 'z', 'r')"
        ),
        1,
    )
    .await
    .unwrap();
    let table = |s: &str| SyncTable { source: s.into(), target: s.into(), key: None, fields: None };
    let spec = SyncSpec { source_schema: a.clone(), target_schema: b.clone(), tables: vec![table("t"), table("sempk"), table("comp")] };

    async fn run_compare(src: &atlas_pg::PgSession, dst: &atlas_pg::PgSession, spec: &SyncSpec) -> atlas_pg::Comparison {
        let mut steps = 0usize;
        let c = {
            let mut on_progress = |_: &str, f: f64| {
                assert!((0.0..=1.0).contains(&f));
                steps += 1;
            };
            atlas_pg::compare_data(src, dst, spec, CompareHooks { progress: &mut on_progress, cancelled: &|| false }).await.expect("compare")
        };
        assert!(steps > 0, "o progresso deve ser avisado");
        c
    }
    let c = run_compare(&src, &dst, &spec).await;
    let (t, sempk, comp) = (&c.tables[0], &c.tables[1], &c.tables[2]);
    assert_eq!((t.insert, t.update, t.delete, t.same), (1, 1, 1, 2), "{:?}", t.message);
    assert_eq!(sempk.message.as_deref(), Some("Source table do not have primary key"));
    assert_eq!((comp.insert, comp.update, comp.delete, comp.same), (1, 0, 1, 1), "{:?}", comp.message);
    // A atualização mostra origem e destino e marca só os campos que mudam (nome e valor).
    let upd = t.rows.iter().find(|r| r.kind == DiffKind::Update).expect("update row");
    assert_eq!(upd.source.as_ref().unwrap()[1].as_deref(), Some("Bruno"));
    assert_eq!(upd.target.as_ref().unwrap()[1].as_deref(), Some("Bruno X"));
    let changed: Vec<&str> = t.source_fields.iter().zip(&upd.changed).filter(|(_, c)| **c).map(|(f, _)| f.as_str()).collect();
    assert_eq!(changed, vec!["nome", "valor"]);

    let all = |table: &str, insert, update, delete| TableSelection { table: table.into(), insert, update, delete, excluded: vec![] };
    // Só atualizações: um UPDATE.
    let only_update = atlas_pg::data_script(&c, &[all("t", false, true, false)]);
    assert_eq!(only_update.len(), 1, "{only_update:?}");
    assert!(only_update[0].starts_with("UPDATE") && only_update[0].contains("WHERE \"id\" = 2"), "{only_update:?}");
    // Desmarcar a linha da inclusão a tira do script.
    let ins_idx = t.rows.iter().position(|r| r.kind == DiffKind::Insert).unwrap() as u32;
    let no_insert = atlas_pg::data_script(&c, &[TableSelection { excluded: vec![ins_idx], ..all("t", true, false, false) }]);
    assert!(no_insert.is_empty(), "{no_insert:?}");

    // Tudo: INSERT, UPDATE e DELETE (os DELETE no fim).
    let statements = atlas_pg::data_script(&c, &[all("t", true, true, true), all("comp", true, true, true)]);
    assert_eq!(statements.len(), 5, "{statements:#?}");
    assert!(statements.iter().any(|s| s.starts_with("INSERT INTO") && s.contains("'linha")) || statements.iter().any(|s| s.contains("'inserir'")));
    assert!(statements.last().unwrap().starts_with("DELETE"));
    let mut log = Vec::new();
    let stats = {
        let mut on_log = |l: String| log.push(l);
        let mut on_progress = |_: u64, _: u64| {};
        atlas_pg::deploy_data(&dst, &statements, true, false, DeployHooks { log: &mut on_log, progress: &mut on_progress, cancelled: &|| false, echo: false }).await.expect("deploy")
    };
    assert_eq!((stats.done, stats.errors), (5, 0), "{log:#?}");
    assert!(log.iter().any(|l| l == "--Transaction - Commit--") && log.iter().any(|l| l == "Executed Query: 5"), "{log:#?}");
    // Depois de implantar, comparar de novo não acha diferença.
    let again = run_compare(&src, &dst, &spec).await;
    for tb in [&again.tables[0], &again.tables[2]] {
        assert_eq!((tb.insert, tb.update, tb.delete), (0, 0, 0), "{} {:?}", tb.source, tb.message);
    }
    assert_eq!(again.tables[0].same, 4);
    let q = |ns: String| format!("SELECT string_agg(id || ':' || coalesce(nome,'~') || ':' || coalesce(valor::text,'~') || ':' || coalesce(nota,'~'), ' | ' ORDER BY id) FROM {ns}.t");
    async fn text(s: &atlas_pg::PgSession, sql: String) -> String {
        atlas_pg::execute(s, &sql, 10).await.unwrap().statements[0].result.as_ref().unwrap().rows[0][0].clone().unwrap_or_default()
    }
    assert_eq!(text(&dst, q(b.clone())).await, text(&src, q(a.clone())).await);

    // Um erro dentro da transação desfaz tudo.
    let bad = vec!["INSERT INTO ".to_string() + &b + ".t (id) VALUES (900);", "INSERT INTO ".to_string() + &b + ".t (id) VALUES (1);"];
    let stats = {
        let mut on_log = |_: String| {};
        let mut on_progress = |_: u64, _: u64| {};
        atlas_pg::deploy_data(&dst, &bad, true, false, DeployHooks { log: &mut on_log, progress: &mut on_progress, cancelled: &|| false, echo: false }).await.expect("deploy")
    };
    assert!(stats.rolled_back && stats.errors == 1);
    assert_eq!(text(&dst, format!("SELECT count(*) FROM {b}.t WHERE id = 900")).await, "0");

    atlas_pg::execute(&src, &format!("DROP SCHEMA {a} CASCADE"), 1).await.unwrap();
    atlas_pg::execute(&dst, &format!("DROP SCHEMA {b} CASCADE"), 1).await.unwrap();
}

#[tokio::test]
async fn syntax_error_reports_position() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let err = atlas_pg::execute(&s, "SELEC 1", 10).await.expect_err("must fail");
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

#[tokio::test]
async fn editor_schema_resolves_unqualified_names_and_restores_the_path() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let first = |o: &atlas_core::QueryOutcome, i: usize| o.statements[i].result.as_ref().unwrap().rows[0][0].clone();
    let before = first(&atlas_pg::execute(&s, "SHOW search_path", 1).await.unwrap(), 0);

    // Tudo numa transação que termina em ROLLBACK: nada persiste no servidor.
    atlas_pg::execute(&s, "BEGIN", 1).await.unwrap();
    atlas_pg::execute(
        &s,
        "CREATE SCHEMA atlas_sp_a; CREATE SCHEMA \"Atlas.Sp B\"; \
         CREATE TABLE atlas_sp_a.somente_aqui (id int); INSERT INTO atlas_sp_a.somente_aqui VALUES (7); \
         CREATE TABLE \"Atlas.Sp B\".t (v text); INSERT INTO \"Atlas.Sp B\".t VALUES ('b'); \
         CREATE TABLE public.atlas_sp_fallback (z int); INSERT INTO public.atlas_sp_fallback VALUES (5)",
        1,
    )
    .await
    .expect("setup");

    // O nome sem schema passa a valer no schema escolhido (o cenário do editor).
    let out = atlas_pg::execute_in_schema(&s, "SELECT id FROM somente_aqui", 10, Some("atlas_sp_a")).await.expect("unqualified in chosen schema");
    assert_eq!(first(&out, 0).as_deref(), Some("7"));
    // Schema com ponto e maiúsculas (precisa de aspas).
    let out = atlas_pg::execute_in_schema(&s, "SELECT v FROM t", 10, Some("Atlas.Sp B")).await.expect("quoted schema name");
    assert_eq!(first(&out, 0).as_deref(), Some("b"));
    // `public` continua valendo depois do schema escolhido (funções de extensões, por exemplo).
    let out = atlas_pg::execute_in_schema(&s, "SELECT z FROM atlas_sp_fallback", 10, Some("atlas_sp_a")).await.expect("public fallback");
    assert_eq!(first(&out, 0).as_deref(), Some("5"));
    // current_schema() reflete a escolha e vários comandos mantêm a ordem dos resultados.
    let out = atlas_pg::execute_in_schema(&s, "SELECT current_schema(); SELECT id FROM somente_aqui", 10, Some("atlas_sp_a")).await.unwrap();
    assert_eq!(out.statements.len(), 2);
    assert_eq!(first(&out, 0).as_deref(), Some("atlas_sp_a"));
    // Depois de cada execução o caminho volta ao padrão da conexão.
    assert_eq!(first(&atlas_pg::execute(&s, "SHOW search_path", 1).await.unwrap(), 0), before);

    // Sem schema escolhido, o mesmo nome não existe: o comportamento antigo continua.
    atlas_pg::execute(&s, "SAVEPOINT sp1", 1).await.unwrap();
    let err = atlas_pg::execute_in_schema(&s, "SELECT id FROM somente_aqui", 10, None).await.expect_err("must not resolve");
    assert_eq!(err.code.as_deref(), Some("42P01"));
    atlas_pg::execute(&s, "ROLLBACK TO sp1", 1).await.unwrap();

    // A posição do erro não desloca: o SET vai numa mensagem à parte.
    atlas_pg::execute(&s, "SAVEPOINT sp2", 1).await.unwrap();
    let err = atlas_pg::execute_in_schema(&s, "SELEC 1", 10, Some("atlas_sp_a")).await.expect_err("syntax error");
    assert_eq!(err.position, Some(1));
    // Transação abortada: o ROLLBACK TO do usuário precisa rodar mesmo com um schema escolhido (o SET é recusado
    // nesse estado e não pode bloquear o comando).
    atlas_pg::execute_in_schema(&s, "ROLLBACK TO sp2", 1, Some("atlas_sp_a")).await.expect("rollback com schema escolhido");
    // E a conexão volta a funcionar normalmente, com o caminho padrão restaurado.
    atlas_pg::execute_in_schema(&s, "SELECT id FROM somente_aqui", 10, Some("atlas_sp_a")).await.expect("depois do rollback");
    assert_eq!(first(&atlas_pg::execute(&s, "SHOW search_path", 1).await.unwrap(), 0), before);

    atlas_pg::execute(&s, "ROLLBACK", 1).await.unwrap();
}

#[test]
fn search_path_puts_the_chosen_schema_first_and_keeps_public() {
    assert_eq!(atlas_pg::search_path_for("public"), "\"public\"");
    assert_eq!(atlas_pg::search_path_for("admin"), "\"admin\", \"public\"");
    assert_eq!(atlas_pg::search_path_for("Weird.Schema"), "\"Weird.Schema\", \"public\"");
    assert_eq!(atlas_pg::search_path_for("a\"b"), "\"a\"\"b\", \"public\"");
}

#[tokio::test]
async fn execute_reports_types_and_never_breaks_an_open_transaction() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");

    // Um único comando traz o tipo de cada coluna (o grid usa isso para os ícones e a formatação).
    let out = atlas_pg::execute(&s, "SELECT 1::int4 AS a, 'x'::text AS b, true AS c", 10).await.unwrap();
    let types: Vec<_> = out.statements[0].result.as_ref().unwrap().columns.iter().map(|c| c.type_name.clone()).collect();
    assert_eq!(types, vec![Some("int4".to_string()), Some("text".to_string()), Some("bool".to_string())]);

    // Vários comandos: sem tipos (o servidor não descreve lotes), mas os dados vêm.
    let out = atlas_pg::execute(&s, "SELECT 1 AS a; SELECT 2 AS b", 10).await.unwrap();
    assert_eq!(out.statements.len(), 2);
    assert!(out.statements[0].result.as_ref().unwrap().columns[0].type_name.is_none());

    // Dentro de uma transação aberta pelo usuário, um lote não pode abortá-la...
    atlas_pg::execute(&s, "BEGIN", 1).await.unwrap();
    atlas_pg::execute(&s, "SELECT 1; SELECT 2", 10).await.expect("lote dentro da transacao");
    atlas_pg::execute(&s, "SELECT 3", 10).await.expect("a transacao continua utilizavel");
    // ...e um erro real continua sendo o erro real (não "current transaction is aborted").
    let err = atlas_pg::execute(&s, "SELECT * FROM atlas_tabela_que_nao_existe", 10).await.expect_err("must fail");
    assert_eq!(err.code.as_deref(), Some("42P01"));
    atlas_pg::execute(&s, "ROLLBACK", 1).await.unwrap();
}

#[tokio::test]
async fn stream_rows_exports_a_query_with_column_subset() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    // O texto é uma consulta qualquer do editor, com `;` final: só as colunas pedidas, em streaming.
    let sql = atlas_pg::wrap_query("select g as id, g * 2 as dbl, 'x' as txt from generate_series(1, 5) g order by 1;", &["dbl".into(), "id".into()]);
    let mut cols = Vec::new();
    let mut last = None;
    let n = atlas_pg::stream_rows(
        &s,
        &sql,
        |c| {
            cols = c.iter().map(|x| x.name.clone()).collect();
            Ok(())
        },
        |row| {
            last = Some(row.iter().map(|v| v.map(str::to_string)).collect::<Vec<_>>());
            Ok(true)
        },
    )
    .await
    .expect("stream query");
    assert_eq!(n, 5);
    assert_eq!(cols, vec!["dbl".to_string(), "id".to_string()]);
    assert_eq!(last.unwrap(), vec![Some("10".into()), Some("5".into())]);
    // Um comando que não devolve linhas não chama on_begin (o app recusa a exportação).
    let mut began = false;
    let n = atlas_pg::stream_rows(&s, "SET search_path TO public", |_| { began = true; Ok(()) }, |_| Ok(true)).await.expect("no rows");
    assert!(n == 0 && !began);
}

#[tokio::test]
async fn structure_sync_compares_scripts_and_deploys() {
    use atlas_pg::{CompareHooks, DeployHooks, StructComparison, StructOp, StructSelection, StructSpec};
    let p = require_server!();
    let src = atlas_pg::connect(&p, None).await.expect("connect source");
    let tp = target_params(&p);
    let dst = atlas_pg::connect(&tp, None).await.expect("connect target");
    let pid = std::process::id();
    let (a, b) = (format!("atlas_ss_a_{pid}"), format!("atlas_ss_b_{pid}"));
    atlas_pg::execute(
        &src,
        &format!(
            "DROP SCHEMA IF EXISTS {a} CASCADE; CREATE SCHEMA {a}; \
             CREATE TYPE {a}.mood AS ENUM ('sad', 'ok', 'happy'); \
             CREATE TYPE {a}.pair AS (x integer, y text, z date); \
             CREATE DOMAIN {a}.posint AS integer DEFAULT 1 NOT NULL CONSTRAINT posint_ck CHECK (VALUE > 0); \
             CREATE SEQUENCE {a}.seq1 INCREMENT BY 2; \
             CREATE SEQUENCE {a}.only_seq; \
             CREATE TABLE {a}.clients(id integer PRIMARY KEY, name varchar(50) NOT NULL DEFAULT 'x', status boolean, humor {a}.mood) WITH (fillfactor=70); \
             CREATE INDEX idx_name ON {a}.clients(name); \
             COMMENT ON TABLE {a}.clients IS 'clientes'; \
             CREATE TABLE {a}.orders(id serial PRIMARY KEY, client_id integer REFERENCES {a}.clients(id), amount numeric CONSTRAINT amount_ok CHECK (amount >= 0)); \
             CREATE TABLE {a}.only_src(id integer PRIMARY KEY); \
             CREATE VIEW {a}.v1 AS SELECT id, name FROM {a}.clients; \
             CREATE FUNCTION {a}.f1(x integer) RETURNS integer LANGUAGE sql AS 'SELECT x + 1'; \
             GRANT SELECT ON {a}.clients TO PUBLIC; GRANT USAGE ON SEQUENCE {a}.seq1 TO PUBLIC; REVOKE EXECUTE ON FUNCTION {a}.f1(integer) FROM PUBLIC; \n             CREATE FUNCTION {a}.trg() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END'; \n             CREATE TRIGGER clients_trg BEFORE INSERT ON {a}.clients FOR EACH ROW EXECUTE PROCEDURE {a}.trg();"
        ),
        1,
    )
    .await
    .expect("source ddl");
    atlas_pg::execute(
        &dst,
        &format!(
            "DROP SCHEMA IF EXISTS {b} CASCADE; CREATE SCHEMA {b}; \
             CREATE TYPE {b}.mood AS ENUM ('sad', 'ok'); \
             CREATE TYPE {b}.pair AS (x bigint, y text, old text); \
             CREATE DOMAIN {b}.posint AS integer CONSTRAINT posint_ck CHECK (VALUE >= 0); \
             CREATE SEQUENCE {b}.seq1; \
             CREATE TABLE {b}.clients(id integer PRIMARY KEY, name varchar(20), old integer); \
             CREATE TABLE {b}.orders(id integer PRIMARY KEY, client_id integer, amount numeric CONSTRAINT amount_ok CHECK (amount > 0)); \
             CREATE TABLE {b}.only_dst(x integer); \
             CREATE VIEW {b}.v_gone AS SELECT x FROM {b}.only_dst; \
             CREATE FUNCTION {b}.f_gone() RETURNS integer LANGUAGE sql AS 'SELECT 1';"
        ),
        1,
    )
    .await
    .expect("target ddl");
    let spec = StructSpec { source_schema: a.clone(), target_schema: b.clone(), compare_owners: true, compare_privileges: true };

    async fn run(src: &atlas_pg::PgSession, dst: &atlas_pg::PgSession, spec: &StructSpec) -> StructComparison {
        let mut steps = 0usize;
        let c = {
            let mut on_progress = |_: &str, f: f64| {
                assert!((0.0..=1.0).contains(&f));
                steps += 1;
            };
            atlas_pg::compare_structure(src, dst, spec, CompareHooks { progress: &mut on_progress, cancelled: &|| false }).await.expect("compare")
        };
        assert!(steps > 0);
        c
    }
    let c = run(&src, &dst, &spec).await;
    let op = |kind: &str, name: &str| c.objects.iter().find(|o| o.kind == kind && o.name == name).unwrap_or_else(|| panic!("{kind} {name} missing")).op;
    assert_eq!(op("type", "mood"), StructOp::Modify);
    assert_eq!(op("sequence", "seq1"), StructOp::Modify);
    assert_eq!(op("type", "pair"), StructOp::Modify);
    assert_eq!(op("type", "posint"), StructOp::Modify);
    assert_eq!(op("sequence", "only_seq"), StructOp::Add);
    assert_eq!(op("table", "clients"), StructOp::Modify);
    assert_eq!(op("table", "orders"), StructOp::Modify);
    assert_eq!(op("table", "only_src"), StructOp::Add);
    assert_eq!(op("table", "only_dst"), StructOp::Delete);
    assert_eq!(op("view", "v1"), StructOp::Add);
    assert_eq!(op("view", "v_gone"), StructOp::Delete);
    assert_eq!(op("function", "f1(x integer)"), StructOp::Add);
    assert_eq!(op("function", "f_gone()"), StructOp::Delete);
    let clients = c.objects.iter().find(|o| o.kind == "table" && o.name == "clients").unwrap();
    assert!(clients.source_ddl.contains("CREATE TABLE") && clients.source_ddl.contains(&format!("\"{b}\".\"clients\"")), "{}", clients.source_ddl);
    let part = |n: &str| clients.parts.iter().find(|p| p.name == n).map(|p| p.op);
    assert_eq!(part("status"), Some(StructOp::Add));
    assert_eq!(part("old"), Some(StructOp::Delete));
    assert_eq!(part("name"), Some(StructOp::Modify));
    assert_eq!(part("id"), Some(StructOp::Same));
    assert_eq!(part("idx_name"), Some(StructOp::Add));

    let pick = |kind: &str, name: &str, parts: Option<Vec<usize>>| StructSelection { kind: kind.into(), name: name.into(), parts };
    // Só uma coluna marcada: um ALTER.
    let status_idx = clients.parts.iter().position(|p| p.name == "status").unwrap();
    let one = atlas_pg::structure_script(&c, &[pick("table", "clients", Some(vec![status_idx]))]);
    assert_eq!(one.len(), 1, "{one:?}");
    assert!(one[0].contains("ADD COLUMN \"status\""));

    // Tudo marcado.
    let selection: Vec<StructSelection> = c.objects.iter().filter(|o| o.op != StructOp::Same).map(|o| pick(o.kind, &o.name, None)).collect();
    let statements = atlas_pg::structure_script(&c, &selection);
    let at = |needle: &str| statements.iter().position(|s| s.contains(needle)).unwrap_or_else(|| panic!("{needle} not in {statements:#?}"));
    assert!(at("DROP VIEW") < at("DROP TABLE"), "a view sai antes da tabela");
    assert!(at("CREATE SEQUENCE") < at("ADD COLUMN"), "sequência antes das colunas que a usam");
    assert!(at("ADD CONSTRAINT \"orders_client_id_fkey\"") > at("ADD COLUMN"), "chave estrangeira depois das colunas");
    assert!(at("CREATE VIEW") > at("ADD COLUMN"), "view depois das tabelas");
    assert!(statements.iter().all(|s| !s.contains(&a)), "nada aponta para o schema da origem: {statements:#?}");

    let mut log = Vec::new();
    let stats = {
        let mut on_log = |l: String| log.push(l);
        let mut on_progress = |_: u64, _: u64| {};
        atlas_pg::deploy_data(&dst, &statements, false, false, DeployHooks { log: &mut on_log, progress: &mut on_progress, cancelled: &|| false, echo: true }).await.expect("deploy")
    };
    assert_eq!(stats.errors, 0, "{log:#?}");
    assert!(log.iter().any(|l| l.starts_with("CREATE SEQUENCE")), "o comando entra no registro: {log:#?}");

    // De novo: não sobra diferença.
    let again = run(&src, &dst, &spec).await;
    let left: Vec<String> = again
        .objects
        .iter()
        .filter(|o| o.op != StructOp::Same)
        .map(|o| format!("{} {} {:?}
--- source
{}
--- target
{}
--- parts {:?}", o.kind, o.name, o.op, o.source_ddl, o.target_ddl, o.parts.iter().filter(|p| p.op != StructOp::Same).map(|p| (&p.name, p.op)).collect::<Vec<_>>()))
        .collect();
    assert!(left.is_empty(), "{left:?}");

    atlas_pg::execute(&src, &format!("DROP SCHEMA {a} CASCADE"), 1).await.unwrap();
    atlas_pg::execute(&dst, &format!("DROP SCHEMA {b} CASCADE"), 1).await.unwrap();
}

#[tokio::test]
async fn structure_sync_alters_generated_columns() {
    use atlas_pg::{CompareHooks, DeployHooks, StructOp, StructSelection, StructSpec};
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    if s.server.version_num < 170000 {
        eprintln!("coluna gerada com SET EXPRESSION exige PostgreSQL 17: teste pulado");
        return;
    }
    let pid = std::process::id();
    let (a, b) = (format!("atlas_sg_a_{pid}"), format!("atlas_sg_b_{pid}"));
    atlas_pg::execute(
        &s,
        &format!(
            "DROP SCHEMA IF EXISTS {a} CASCADE; DROP SCHEMA IF EXISTS {b} CASCADE; CREATE SCHEMA {a}; CREATE SCHEMA {b}; \
             CREATE TABLE {a}.t(x integer, g integer GENERATED ALWAYS AS (x + 1) STORED, p integer GENERATED ALWAYS AS (x * 2) STORED, h integer); \
             CREATE TABLE {b}.t(x integer, g integer GENERATED ALWAYS AS (x + 2) STORED, p integer, h integer GENERATED ALWAYS AS (x * 3) STORED); \
             INSERT INTO {b}.t(x, p) VALUES (1, 7)"
        ),
        1,
    )
    .await
    .expect("ddl");
    let spec = StructSpec { source_schema: a.clone(), target_schema: b.clone(), compare_owners: true, compare_privileges: true };
    let c = atlas_pg::compare_structure(&s, &s, &spec, CompareHooks { progress: &mut |_: &str, _: f64| {}, cancelled: &|| false }).await.expect("compare");
    let table = c.objects.iter().find(|o| o.kind == "table" && o.name == "t").expect("table");
    assert_eq!(table.op, StructOp::Modify);
    let statements = atlas_pg::structure_script(&c, &[StructSelection { kind: "table".into(), name: "t".into(), parts: None }]);
    assert!(statements.iter().any(|x| x.contains("SET EXPRESSION AS")), "{statements:#?}");
    assert!(statements.iter().any(|x| x.contains("DROP EXPRESSION")), "{statements:#?}");
    let mut log = Vec::new();
    let stats = {
        let mut on_log = |l: String| log.push(l);
        atlas_pg::deploy_data(&s, &statements, false, false, DeployHooks { log: &mut on_log, progress: &mut |_: u64, _: u64| {}, cancelled: &|| false, echo: false }).await.expect("deploy")
    };
    assert_eq!(stats.errors, 0, "{log:#?} {statements:#?}");
    let again = atlas_pg::compare_structure(&s, &s, &spec, CompareHooks { progress: &mut |_: &str, _: f64| {}, cancelled: &|| false }).await.expect("compare");
    let left: Vec<String> = again.objects.iter().filter(|o| o.op != StructOp::Same).map(|o| o.name.clone()).collect();
    assert!(left.is_empty(), "{left:?}");
    atlas_pg::execute(&s, &format!("DROP SCHEMA {a} CASCADE; DROP SCHEMA {b} CASCADE"), 1).await.unwrap();
}

#[tokio::test]
async fn data_generation_describes_previews_and_runs() {
    use atlas_pg::{GenHooks, GenOptions, GenSpec, GenTableSpec, GenColumnSpec, Generator};
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let ns = format!("atlas_dg_{}", std::process::id());
    atlas_pg::execute(
        &s,
        &format!(
            "DROP SCHEMA IF EXISTS {ns} CASCADE; CREATE SCHEMA {ns}; \
             CREATE TYPE {ns}.mood AS ENUM ('sad', 'ok'); \
             CREATE TABLE {ns}.clients(id serial PRIMARY KEY, name varchar(30) NOT NULL, active boolean, born date, balance numeric(10,2), mood {ns}.mood, ts timestamptz); \
             CREATE TABLE {ns}.orders(id serial PRIMARY KEY, client_id integer NOT NULL REFERENCES {ns}.clients(id), amount numeric(8,2) NOT NULL, created timestamptz NOT NULL DEFAULT now()); \
             INSERT INTO {ns}.clients(id, name) VALUES (5, 'existing')"
        ),
        1,
    )
    .await
    .expect("ddl");

    let meta = atlas_pg::describe_generation(&s, &ns).await.expect("describe");
    let names: Vec<&str> = meta.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["clients", "orders"], "a tabela referenciada vem antes");
    let col = |t: &str, c: &str| meta.iter().find(|x| x.name == t).unwrap().columns.iter().find(|x| x.name == c).unwrap();
    assert!(matches!(col("clients", "id").suggested, Generator::Sequence { start: None, step: 1 }));
    assert!(matches!(col("clients", "name").suggested, Generator::Text { max_len: 30, .. }), "{:?}", col("clients", "name").suggested);
    assert!(matches!(col("clients", "mood").suggested, Generator::List { .. }));
    assert!(matches!(col("clients", "born").suggested, Generator::Date { .. }));
    assert!(matches!(&col("orders", "client_id").suggested, Generator::Reference { table, column } if table == "clients" && column == "id"));
    assert!(matches!(col("orders", "amount").suggested, Generator::Decimal { scale: 2, .. }));

    let table = |name: &str, rows: u64| GenTableSpec {
        name: name.into(),
        rows,
        columns: meta
            .iter()
            .find(|t| t.name == name)
            .unwrap()
            .columns
            .iter()
            .filter(|c| !c.generated && !(name == "orders" && c.name == "created"))
            .map(|c| GenColumnSpec { name: c.name.clone(), generator: c.suggested.clone(), null_percent: if c.not_null { 0 } else { 20 } })
            .collect(),
    };
    let preview = atlas_pg::preview_generation(&s, &ns, &table("clients", 3), 3).await.expect("preview");
    assert_eq!(preview.rows.len(), 3);
    assert_eq!(preview.rows[0][0].as_deref(), Some("6"), "a sequência continua do maior valor");
    assert_eq!(preview.rows[1][0].as_deref(), Some("7"));
    assert_eq!(atlas_pg::preview_generation(&s, &ns, &table("clients", 1), 1).await.unwrap().rows[0][0].as_deref(), Some("6"), "a prévia não grava nada");

    let count = |t: &str| {
        let sql = format!("SELECT count(*) FROM {ns}.{t}");
        let s = &s;
        async move { s.client.query_one(&sql, &[]).await.unwrap().get::<_, i64>(0) }
    };
    let options = |continue_on_error, transaction, multi_insert| GenOptions { continue_on_error, empty_first: false, transaction, multi_insert };
    let go = |tables: Vec<GenTableSpec>, options: GenOptions| {
        let s = &s;
        let ns = ns.clone();
        async move {
            let mut log = Vec::new();
            let stats = {
                let mut on_log = |l: String| log.push(l);
                atlas_pg::run_generation(s, &GenSpec { schema: ns, tables, options }, GenHooks { log: &mut on_log, progress: &mut |_| {}, cancelled: &|| false }).await.expect("run")
            };
            (stats, log)
        }
    };

    // Duas tabelas, com transação e inserção em lote.
    let (stats, log) = go(vec![table("clients", 700), table("orders", 120)], options(false, true, true)).await;
    assert_eq!((stats.objects, stats.generated, stats.inserted, stats.errors), (2, 820, 820, 0), "{log:#?}");
    assert_eq!((count("clients").await, count("orders").await), (701, 120));
    assert!(log.iter().any(|l| l.contains("1> clients: --Transaction - Commit--")) && log.last().unwrap().contains("Finished successfully"), "{log:#?}");
    // A sequência da coluna serial ficou depois do que foi gerado.
    let next: i32 = s.client.query_one(&format!("SELECT nextval(pg_get_serial_sequence('{ns}.clients', 'id'))::int"), &[]).await.unwrap().get(0);
    assert_eq!(next, 706, "5 existente + 700 gerados a partir do 6 = 705");

    // Valores repetidos: sem continuar, a primeira falha desfaz a tabela e para; continuando, só as linhas ruins ficam de fora.
    let mut dup = table("clients", 3);
    dup.columns[0].generator = Generator::Sequence { start: Some(5), step: 0 };
    let (stats, log) = go(vec![dup.clone()], options(false, true, false)).await;
    assert_eq!((stats.errors, stats.inserted), (1, 0), "{log:#?}");
    assert!(log.iter().any(|l| l.contains("--Transaction - Rollback--")) && log.last().unwrap().contains("Finished with error"), "{log:#?}");
    let (stats, _) = go(vec![dup], options(true, true, false)).await;
    assert_eq!((stats.errors, stats.inserted), (3, 0));
    assert_eq!(count("clients").await, 701);

    // Esvaziar antes: as tabelas selecionadas saem (as filhas primeiro).
    let mut empty = options(false, true, true);
    empty.empty_first = true;
    let (stats, _) = go(vec![table("clients", 10), table("orders", 10)], empty).await;
    assert_eq!((stats.inserted, stats.errors), (20, 0));
    assert_eq!((count("clients").await, count("orders").await), (10, 10));

    atlas_pg::execute(&s, &format!("DROP SCHEMA {ns} CASCADE"), 1).await.unwrap();
}

#[tokio::test]
async fn server_monitor_lists_processes_variables_status_and_ends_a_process() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let snap = atlas_pg::monitor_snapshot(&s).await.expect("snapshot");
    // Os processos incluem a própria conexão, com o comando e o bloqueio que ela segura.
    let me: i32 = s.client.query_one("SELECT pg_backend_pid()", &[]).await.unwrap().get(0);
    let mine: Vec<_> = snap.processes.iter().filter(|r| r.id == me).collect();
    assert!(!mine.is_empty() && mine.iter().all(|r| r.user.as_deref() == Some(p.user.as_str())));
    assert!(mine.iter().any(|r| r.info.as_deref().is_some_and(|i| i.starts_with("Locked "))), "{mine:?}");
    // As variáveis começam por DateStyle, IntervalStyle e TimeZone e seguem em ordem.
    let names: Vec<&str> = snap.variables.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(&names[..3], ["DateStyle", "IntervalStyle", "TimeZone"]);
    assert!(names[3..].windows(2).all(|w| w[0] <= w[1]));
    assert!(snap.variables.iter().any(|(n, v)| n == "max_connections" && v.parse::<i32>().is_ok()));
    let status: Vec<&str> = snap.status.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(status, ["db_blocks_fetched", "db_blocks_hit", "db_numbackends", "db_xact_commit", "db_xact_rollback"]);
    assert!(snap.status.iter().all(|(_, v)| v.parse::<i64>().is_ok()));

    // End Process: encerra uma outra conexão.
    let victim = atlas_pg::connect(&p, None).await.expect("victim");
    let pid: i32 = victim.client.query_one("SELECT pg_backend_pid()", &[]).await.unwrap().get(0);
    assert!(atlas_pg::monitor_end_process(&s, pid).await.expect("end"));
    for _ in 0..20 {
        if victim.client.is_closed() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(victim.client.is_closed(), "o processo encerrado perde a conexão");
}

#[tokio::test]
async fn history_log_records_statements_errors_and_masks_passwords() {
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let pid: i32 = s.client.query_one("SELECT pg_backend_pid()", &[]).await.unwrap().get(0);
    atlas_pg::history_set_paused(false);
    let (_, base) = atlas_pg::history_read(0);

    atlas_pg::execute(&s, "SELECT 1 AS um", 10).await.expect("ok");
    let bad = atlas_pg::execute(&s, "SELECT\n  nao_existe FROM pg_class", 10).await;
    assert!(bad.is_err());
    // Um comando com senha nunca entra com o texto da senha (o servidor recusa o papel que já existe ou aceita; tanto faz).
    let _ = atlas_pg::execute(&s, "CREATE ROLE atlas_hist_tmp LOGIN PASSWORD 'segredo123'", 1).await;
    let _ = atlas_pg::execute(&s, "DROP ROLE IF EXISTS atlas_hist_tmp", 1).await;

    let (entries, last) = atlas_pg::history_read(base);
    let mine: Vec<_> = entries.iter().filter(|e| e.pid == pid).collect();
    let ok = mine.iter().find(|e| e.text == "SELECT 1 AS um").expect("o comando aparece");
    assert!(ok.secs.is_some() && !ok.error && !ok.failed && ok.server.starts_with("PostgreSQL "), "{ok:?}");
    let failed = mine.iter().find(|e| e.text.contains("nao_existe FROM") && !e.error).expect("o comando que falhou aparece");
    assert!(failed.failed);
    let message = mine.iter().find(|e| e.error && e.text.contains("nao_existe")).expect("o erro aparece");
    assert!(message.text.starts_with("ERROR:  ") && message.text.contains("LINE 2:") && message.text.contains('^') && message.secs.is_none(), "{}", message.text);
    assert!(mine.iter().all(|e| !e.text.contains("segredo123")), "a senha sai mascarada");
    assert!(mine.iter().any(|e| e.text.contains("PASSWORD '********'")));

    // Pausado: nada novo é registrado.
    atlas_pg::history_set_paused(true);
    atlas_pg::execute(&s, "SELECT 2 AS dois", 10).await.expect("ok");
    atlas_pg::history_set_paused(false);
    assert!(atlas_pg::history_read(last).0.iter().all(|e| e.text != "SELECT 2 AS dois"));
}

#[tokio::test]
async fn console_runs_sql_and_backslash_commands_like_psql() {
    use atlas_pg::{ConsoleOptions, console_is_complete, console_run};
    let p = require_server!();
    let s = atlas_pg::connect(&p, None).await.expect("connect");
    let ns = format!("atlas_cn_{}", std::process::id());
    let opts = ConsoleOptions::default();
    let run = |input: &'static str| {
        let s = &s;
        async move { console_run(s, input, opts).await }
    };
    let setup = format!(
        "CREATE SCHEMA {ns}; SET search_path TO {ns}; \
         CREATE TABLE clients(id serial PRIMARY KEY, name varchar(30) NOT NULL, nota text DEFAULT 'x'); \
         CREATE UNIQUE INDEX clients_name_key ON clients(name); \
         CREATE TABLE orders(id int PRIMARY KEY, client_id int REFERENCES clients(id), amount numeric CHECK (amount > 0)); \
         INSERT INTO clients(name) VALUES ('Ana'), ('Bruno');"
    );
    assert!(console_is_complete(&setup));
    let out = console_run(&s, &setup, opts).await;
    assert!(out.text.contains("CREATE SCHEMA") && out.text.contains("SET\n") && out.text.contains("CREATE TABLE") && out.text.contains("CREATE INDEX") && out.text.contains("INSERT 0 2"), "{}", out.text);

    let out = run("SELECT id, name FROM clients ORDER BY id;").await;
    assert_eq!(out.text, " id | name\n----+-------\n  1 | Ana\n  2 | Bruno\n(2 rows)\n\n");
    let out = run("UPDATE clients SET nota = 'y' WHERE id = 1;").await;
    assert_eq!(out.text, "UPDATE 1\n\n");

    // Erro: posição e dica, como no psql; o console continua utilizável.
    let out = run("SELECT\n  nao_existe FROM clients;").await;
    assert!(out.text.starts_with("ERROR:  ") && out.text.contains("nao_existe") && out.text.contains("\nLINE 2:   nao_existe FROM clients;\n"), "{}", out.text);
    assert!(out.text.contains('^'));

    let out = run("\\dt").await;
    assert!(out.text.contains("List of relations") && out.text.contains(&ns) && out.text.contains("clients") && out.text.contains("orders") && out.text.contains("(2 rows)"), "{}", out.text);
    let out = run("\\d clients").await;
    assert!(out.text.contains(&format!("Table \"{ns}.clients\"")), "{}", out.text);
    assert!(out.text.contains("Indexes:") && out.text.contains("\"clients_pkey\" PRIMARY KEY, btree (id)") && out.text.contains("\"clients_name_key\" UNIQUE, btree (name)"), "{}", out.text);
    assert!(out.text.contains("Referenced by:") && out.text.contains("TABLE \"orders\" CONSTRAINT \"orders_client_id_fkey\" FOREIGN KEY (client_id) REFERENCES clients(id)"), "{}", out.text);
    assert!(out.text.contains("not null") && out.text.contains("nextval("), "{}", out.text);
    let out = run("\\d orders").await;
    assert!(out.text.contains("Check constraints:") && out.text.contains("CHECK (amount > 0::numeric)") && out.text.contains("Foreign-key constraints:"), "{}", out.text);
    assert_eq!(run("\\d nada_disso").await.text, "Did not find any relation named \"nada_disso\".\n");
    let out = run("\\l").await;
    assert!(out.text.starts_with("                                 List of databases") || out.text.contains("List of databases"), "{}", out.text);
    assert!(out.text.contains(&s.server.current_database));
    assert!(run("\\foo").await.text.starts_with("invalid command \\foo"));
    assert!(run("\\q").await.quit);
    assert_eq!(run("\\c outro_banco").await.switch_db.as_deref(), Some("outro_banco"));

    // \timing e \x são lembrados nas opções devolvidas.
    let out = run("\\timing").await;
    assert!(out.options.timing && out.text == "Timing is on.\n");
    let timed = console_run(&s, "SELECT 1;", out.options).await;
    assert!(timed.text.contains("Time: "), "{}", timed.text);
    let x = console_run(&s, "\\x", opts).await;
    assert!(x.options.expanded);
    let rec = console_run(&s, "SELECT 1 AS a, 'b' AS bb;", x.options).await;
    assert!(rec.text.starts_with("-[ RECORD 1 ]+"), "{}", rec.text);

    atlas_pg::execute(&s, &format!("DROP SCHEMA {ns} CASCADE"), 1).await.unwrap();
}
