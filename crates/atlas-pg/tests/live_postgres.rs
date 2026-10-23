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
