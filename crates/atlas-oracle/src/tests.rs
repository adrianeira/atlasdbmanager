//! Testes do Oracle. Os de texto rodam sempre; os que usam um servidor de verdade rodam só com as variáveis `ATLAS_TEST_ORACLE_*`
//! (`HOST`, `PORT`, `SERVICE`, `USER`, `PASSWORD`; senão passam sem fazer nada).

use super::*;
use atlas_core::{ConnectionParams, OrderBy, Secret, SslMode, TableFilter};
use std::time::Duration;

fn params() -> Option<ConnectionParams> {
    Some(ConnectionParams {
        host: std::env::var("ATLAS_TEST_ORACLE_HOST").ok()?,
        port: std::env::var("ATLAS_TEST_ORACLE_PORT").ok()?.parse().ok()?,
        database: std::env::var("ATLAS_TEST_ORACLE_SERVICE").ok()?,
        user: std::env::var("ATLAS_TEST_ORACLE_USER").ok()?,
        password: Secret::new(std::env::var("ATLAS_TEST_ORACLE_PASSWORD").ok()?),
        ssl_mode: SslMode::Disable,
        connect_timeout_secs: 15,
        ssh: None,
    })
}

fn sqls(text: &str) -> Vec<String> {
    split_statements(text).into_iter().map(|p| p.sql).collect()
}

#[test]
fn quoting_doubles_the_quote() {
    assert_eq!(quote_ident("CLIENTS"), "\"CLIENTS\"");
    assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
}

#[test]
fn select_pages_with_offset_fetch() {
    let o = [OrderBy { column: "ID".into(), descending: true }];
    let f = TableFilter { expr: "QTY > 1".into() };
    assert_eq!(select_sql("APP", "T", &o, Some(&f), Some((10, 20))), "SELECT * FROM \"APP\".\"T\" WHERE QTY > 1 ORDER BY \"ID\" DESC OFFSET 20 ROWS FETCH NEXT 10 ROWS ONLY");
    assert_eq!(select_columns_sql("APP", "T", &["A".into(), "B".into()], &[], None), "SELECT \"A\", \"B\" FROM \"APP\".\"T\"");
    assert_eq!(parse_version("21.0.0.0.0"), ("21.0".to_string(), 210000));
    assert_eq!(parse_version("23.26.3.0.0"), ("23.26".to_string(), 232603));
}

#[test]
fn splits_plain_statements_without_the_semicolon() {
    assert_eq!(sqls("SELECT 1 FROM dual; SELECT 2 FROM dual;\n-- fim\n"), vec!["SELECT 1 FROM dual", "SELECT 2 FROM dual"]);
    assert_eq!(sqls("INSERT INTO t VALUES ('a;b'); /* ; */ UPDATE t SET x = q'[it's;]'"), vec!["INSERT INTO t VALUES ('a;b')", "/* ; */ UPDATE t SET x = q'[it's;]'"]);
    assert_eq!(sqls("SELECT \"a;b\" FROM dual"), vec!["SELECT \"a;b\" FROM dual"]);
}

#[test]
fn pieces_know_where_they_start() {
    let starts: Vec<usize> = split_statements("SELECT 1 FROM dual;   
  SELECT 'á' FROM dual;SELECT 3 FROM dual").iter().map(|p| p.start).collect();
    assert_eq!(starts, vec![0, 25, 46]);
}

#[test]
fn unbound_placeholders_are_found_outside_text_and_comments() {
    let at = |sql: &str| split::unbound_placeholder(sql);
    let colon = |sql: &str, n: usize| sql.char_indices().filter(|(_, c)| *c == ':').nth(n).map(|(i, _)| i).unwrap();
    let a = "SELECT * FROM t WHERE id = :id";
    assert_eq!(at(a), Some(("id".to_string(), colon(a, 0))));
    let b = "SELECT * FROM t WHERE \"ID\" = '42'::NUMBER(10)";
    assert_eq!(at(b), Some(("NUMBER".to_string(), colon(b, 1))));
    assert_eq!(at("SELECT ':a', \"x:b\", q'[:c]' FROM t -- :d
 /* :e */"), None);
    let c = "SELECT 'it''s :x' FROM t WHERE a = :1";
    assert_eq!(at(c), Some(("1".to_string(), colon(c, 1))));
    assert_eq!(at("BEGIN :new.a := 1; END;"), None);
    let d = "BEGIN :x := 1; END;";
    assert_eq!(at(d), Some(("x".to_string(), 6)));
    assert_eq!(at("SELECT 1 FROM dual"), None);
}

#[test]
fn plsql_blocks_end_at_slash_or_at_the_end_of_the_text() {
    let text = "CREATE OR REPLACE PROCEDURE p AS\nBEGIN\n  NULL;\nEND;\n/\nSELECT 1 FROM dual;\nBEGIN NULL; END;";
    assert_eq!(sqls(text), vec!["CREATE OR REPLACE PROCEDURE p AS\nBEGIN\n  NULL;\nEND;", "SELECT 1 FROM dual", "BEGIN NULL; END;"]);
    assert!(split::is_plsql("create or replace editionable trigger t before insert on x begin null; end;"));
    assert!(!split::is_plsql("CREATE TABLE t (a NUMBER)"));
    assert!(split::returns_rows("  -- c\n WITH x AS (SELECT 1 FROM dual) SELECT * FROM x"));
    assert!(!split::returns_rows("UPDATE t SET a = 1"));
}

#[test]
fn sequence_defaults_are_recognised() {
    assert_eq!(meta::sequence_of("\"SYSTEM\".\"ZI_ID_SEQ\".\"NEXTVAL\"").as_deref(), Some("SYSTEM.ZI_ID_SEQ"));
    assert_eq!(meta::sequence_of("seq1.nextval").as_deref(), Some("seq1"));
    assert_eq!(meta::sequence_of("SYSDATE"), None);
    assert_eq!(meta::sequence_of("'a.nextval'"), None);
}

#[test]
fn error_codes_become_categories() {
    let e = map_err(OraError::OracleError { code: 942, message: "ORA-00942: table or view does not exist".into() });
    assert_eq!(e.message, "table or view does not exist");
    assert_eq!(e.code.as_deref(), Some("ORA-00942"));
    assert!(matches!(e.kind, ErrorKind::SqlSyntaxError));
    assert!(matches!(map_err(OraError::OracleError { code: 1, message: String::new() }).kind, ErrorKind::ConstraintViolation));
    assert!(matches!(map_err(OraError::OracleError { code: 1017, message: String::new() }).kind, ErrorKind::AuthenticationFailed));
}

#[tokio::test]
async fn live_server_lists_reads_writes_and_survives_errors() {
    let Some(p) = params() else { return };
    let s = OracleSession::open(p).await.unwrap();
    assert!(s.version_num >= 120000, "version {}", s.version);
    let user = s.current_user.clone();
    let t = format!("ATLAS_T_{}", std::process::id());
    let qt = quote_ident(&t);

    s.execute("", &format!("CREATE TABLE {qt} (id NUMBER(10) GENERATED BY DEFAULT AS IDENTITY PRIMARY KEY, n NUMBER(12,3), s VARCHAR2(50 CHAR) NOT NULL, d DATE, ts TIMESTAMP, c CLOB, CONSTRAINT {c} CHECK (n >= 0))", c = quote_ident(&format!("{t}_CK"))), 10).await.unwrap();
    // Um erro de SQL não derruba a sessão e traz o texto do servidor.
    // A posição do erro em caracteres do texto todo (o segundo comando começa depois do primeiro).
    let err = s.execute("", "SELECT 'á' FROM dual; SELECT áé, bad FROM dual", 10).await.unwrap_err();
    assert_eq!(err.code.as_deref(), Some("ORA-00904"));
    assert_eq!(err.position, Some("SELECT 'á' FROM dual; SELECT áé, ".chars().count() as u32 + 1));
    let err = s.execute("", "SELEC 1 FROM dual", 10).await.unwrap_err();
    assert_eq!(err.code.as_deref(), Some("ORA-00900"));
    assert!(err.message.contains("invalid SQL statement"), "{}", err.message);
    let err = s.execute("", &format!("INSERT INTO {qt} (s, n) VALUES ('x', -1)"), 10).await.unwrap_err();
    assert!(matches!(err.kind, ErrorKind::ConstraintViolation), "{err:?}");

    let applied = s
        .apply(
            &[
                PlannedStatement { sql: format!("INSERT INTO {qt} (s, n, d, ts, c) VALUES (:1, :2, :3, :4, :5)"), params: vec![Some("olá ção".into()), Some("1.5".into()), Some("2026-10-27 13:45:00".into()), Some("2026-10-27 13:45:00.25".into()), Some("texto".into())], expect_one_row: true, expect_rows: None },
                PlannedStatement { sql: format!("INSERT INTO {qt} (s) VALUES (:1)"), params: vec![Some("sem numero".into())], expect_one_row: true, expect_rows: None },
            ],
        )
        .await
        .unwrap();
    assert_eq!(applied, vec![1, 1]);

    let (_, out) = s.table_rows(&user, &t, &[OrderBy { column: "ID".into(), descending: false }], None, Some(10), 0, 100).await.unwrap();
    let rs = out.statements[0].result.as_ref().unwrap();
    assert_eq!(rs.rows.len(), 2);
    assert_eq!(rs.rows[0][1].as_deref(), Some("1.5"));
    assert_eq!(rs.rows[0][2].as_deref(), Some("olá ção"));
    assert_eq!(rs.rows[0][3].as_deref(), Some("2026-10-27 13:45:00"));
    assert_eq!(rs.rows[0][4].as_deref(), Some("2026-10-27 13:45:00.25"));
    assert_eq!(rs.rows[0][5].as_deref(), Some("texto"));
    assert_eq!(rs.rows[1][1], None);
    assert_eq!(s.table_count(&user, &t, None).await.unwrap(), 2);

    // A estrutura.
    let d = s.table_detail(&user, &t).await.unwrap();
    assert_eq!(d.columns[0].key, Some(1));
    assert_eq!(d.columns[0].identity.as_deref(), Some("d"));
    assert_eq!(d.columns[1].formatted_type, "NUMBER(12,3)");
    assert_eq!(d.columns[2].formatted_type, "VARCHAR2(50 CHAR)");
    assert!(d.columns[2].not_null);
    assert!(d.indexes.iter().any(|i| i.primary));
    assert!(d.constraints.iter().any(|c| c.kind == "c" && c.definition.contains("n >= 0")), "{:?}", d.constraints);
    let objs = s.list_objects(&user).await.unwrap();
    assert!(objs.tables.iter().any(|o| o.name == t));
    assert!(s.list_schemas(false).await.unwrap().iter().any(|x| x.name == user));

    // Transação manual: o que não foi gravado some no desfazer.
    let tx = s.begin().await.unwrap();
    tx.apply(&[PlannedStatement { sql: format!("DELETE FROM {qt}"), params: vec![], expect_one_row: false, expect_rows: Some(2) }]).await.unwrap();
    assert_eq!(tx.table_count(&user, &t, None).await.unwrap(), 0);
    assert_eq!(s.table_count(&user, &t, None).await.unwrap(), 2);
    tx.finish(false).await.unwrap();
    assert_eq!(s.table_count(&user, &t, None).await.unwrap(), 2);

    // Uma contagem inesperada desfaz o lote inteiro.
    let err = s.apply(&[PlannedStatement { sql: format!("DELETE FROM {qt} WHERE s = :1"), params: vec![Some("nada".into())], expect_one_row: true, expect_rows: None }]).await.unwrap_err();
    assert!(matches!(err.kind, ErrorKind::ConcurrencyConflict));

    // Numeração por sequência (coluna que já existia): aparece como automática, com o nome da sequência.
    s.execute("", &format!("CREATE SEQUENCE {sq}", sq = quote_ident(&format!("{t}_SQ"))), 10).await.unwrap();
    s.execute("", &format!("ALTER TABLE {qt} ADD (k NUMBER DEFAULT {sq}.NEXTVAL)", sq = quote_ident(&format!("{t}_SQ"))), 10).await.unwrap();
    let d = s.table_detail(&user, &t).await.unwrap();
    let k = d.columns.iter().find(|c| c.name == "K").unwrap();
    assert_eq!(k.identity.as_deref(), Some("d"));
    assert_eq!(k.auto_source.as_deref(), Some(format!("{user}.{t}_SQ").as_str()));
    assert_eq!(k.default, None);
    s.execute("", &format!("DROP SEQUENCE {sq}", sq = quote_ident(&format!("{t}_SQ"))), 10).await.unwrap();

    // Linhas largas (a resposta passa de um pacote de rede) e valores iguais ao da linha anterior, inclusive entre um lote e o seguinte.
    let wt = quote_ident(&format!("{t}_W"));
    s.execute("", &format!("CREATE TABLE {wt} (id NUMBER, s VARCHAR2(2000), c CLOB, g VARCHAR2(10))"), 10).await.unwrap();
    s.execute("", &format!("INSERT INTO {wt} SELECT level, RPAD('v', 1500, 'x'), RPAD('c', 3000, 'y'), CASE WHEN MOD(level, 7) = 0 THEN NULL ELSE 'igual' END FROM dual CONNECT BY level <= 300"), 10).await.unwrap();
    let (_, out) = s.table_rows(&user, &format!("{t}_W"), &[OrderBy { column: "ID".into(), descending: false }], None, None, 0, 1000).await.unwrap();
    let rows = &out.statements[0].result.as_ref().unwrap().rows;
    assert_eq!(rows.len(), 300);
    for (i, r) in rows.iter().enumerate() {
        let id = i + 1;
        assert_eq!(r[1].as_deref().map(str::len), Some(1500), "linha {id}");
        assert_eq!(r[2].as_deref().map(str::len), Some(3000), "linha {id}");
        assert_eq!(r[3].as_deref(), if id % 7 == 0 { None } else { Some("igual") }, "linha {id}");
    }

    // Repetir a mesma consulta na sessão (o driver reaproveita o cursor) devolve as mesmas linhas, antes e depois de gravar.
    for round in 0..3 {
        let (_, out) = s.table_rows(&user, &format!("{t}_W"), &[OrderBy { column: "ID".into(), descending: false }], None, Some(1000), 0, 1000).await.unwrap();
        let rows = &out.statements[0].result.as_ref().unwrap().rows;
        assert_eq!(rows.len(), 300, "rodada {round}");
        assert_eq!(rows[299][1].as_deref().map(str::len), Some(1500), "rodada {round}");
        assert_eq!(rows[150][3].as_deref(), Some("igual"), "rodada {round}");
        s.apply(&[PlannedStatement { sql: format!("UPDATE {wt} SET g = 'igual' WHERE id = :1"), params: vec![Some("1".into())], expect_one_row: true, expect_rows: None }]).await.unwrap();
    }
    s.execute("", &format!("DROP TABLE {wt} PURGE"), 10).await.unwrap();

    // Resultados longos vêm inteiros (o driver os entrega em levas) e `SELECT NULL` devolve a linha.
    let out = s.execute("", "SELECT level, 'x' || level FROM dual CONNECT BY level <= 5000", 100_000).await.unwrap();
    let rs = out.statements[0].result.as_ref().unwrap();
    assert_eq!((rs.rows.len(), rs.truncated), (5000, false));
    assert_eq!(rs.rows[4999][1].as_deref(), Some("x5000"));
    let out = s.execute("", "SELECT level FROM dual CONNECT BY level <= 1000", 300).await.unwrap();
    let rs = out.statements[0].result.as_ref().unwrap();
    assert_eq!((rs.rows.len(), rs.truncated), (300, true));
    let out = s.execute("", "SELECT NULL FROM dual", 10).await.unwrap();
    assert_eq!(out.statements[0].result.as_ref().unwrap().rows, vec![vec![None]]);
    let mut seen = 0u64;
    let n = s.stream_rows("SELECT level FROM dual CONNECT BY level <= 2500", |_| Ok(()), |_| { seen += 1; Ok(true) }).await.unwrap();
    assert_eq!((n, seen), (2500, 2500));

    // `LONG` e `LONG RAW` (uma coluna de cada tipo por tabela) e `XMLType` (a grade o pede como texto).
    let lt = quote_ident(&format!("{t}_L"));
    let xt = quote_ident(&format!("{t}_X"));
    s.execute("", &format!("CREATE TABLE {lt} (id NUMBER, l LONG)"), 10).await.unwrap();
    s.execute("", &format!("INSERT INTO {lt} VALUES (1, 'texto longo'); INSERT INTO {lt} VALUES (2, NULL); INSERT INTO {lt} VALUES (3, 'fim')"), 10).await.unwrap();
    let (_, out) = s.table_rows(&user, &format!("{t}_L"), &[OrderBy { column: "ID".into(), descending: false }], None, None, 0, 100).await.unwrap();
    let rows = &out.statements[0].result.as_ref().unwrap().rows;
    assert_eq!(rows, &vec![vec![Some("1".into()), Some("texto longo".into())], vec![Some("2".into()), None], vec![Some("3".into()), Some("fim".into())]]);
    // Em alguns servidores o tablespace SYSTEM não aceita o armazenamento do XMLType (ORA-43853): o teste passa sem essa parte.
    match s.execute("", &format!("CREATE TABLE {xt} (id NUMBER, x XMLTYPE)"), 10).await {
        Ok(_) => {
            s.execute("", &format!("INSERT INTO {xt} VALUES (1, XMLTYPE('<a><b>1</b></a>'))"), 10).await.unwrap();
            let (_, out) = s.table_rows(&user, &format!("{t}_X"), &[], None, None, 0, 100).await.unwrap();
            let rows = &out.statements[0].result.as_ref().unwrap().rows;
            assert_eq!(rows[0][1].as_deref().map(|x| x.split_whitespace().collect::<String>()).as_deref(), Some("<a><b>1</b></a>"));
            s.execute("", &format!("DROP TABLE {xt} PURGE"), 10).await.unwrap();
        }
        Err(e) => assert_eq!(e.code.as_deref(), Some("ORA-43853"), "{e:?}"),
    }
    s.execute("", &format!("DROP TABLE {lt} PURGE"), 10).await.unwrap();

    // Diagrama ER: tabelas com colunas e chaves, e as chaves estrangeiras.
    let fk_t = quote_ident(&format!("{t}_F"));
    s.execute("", &format!("CREATE TABLE {fk_t} (id NUMBER PRIMARY KEY, ref_id NUMBER NOT NULL, CONSTRAINT {c} FOREIGN KEY (ref_id) REFERENCES {qt} (id))", c = quote_ident(&format!("{t}_FK"))), 10).await.unwrap();
    let dg = s.schema_diagram(&user).await.unwrap();
    let child = dg.tables.iter().find(|x| x.name == format!("{t}_F")).unwrap();
    assert!(child.columns[0].is_pk && child.columns[1].is_fk);
    let rel = dg.relations.iter().find(|x| x.table == format!("{t}_F")).unwrap();
    assert_eq!((rel.ref_table.as_str(), rel.columns.as_slice(), rel.required), (t.as_str(), &["REF_ID".to_string()][..], true));
    s.execute("", &format!("DROP TABLE {fk_t} PURGE"), 10).await.unwrap();

    // Variável de ligação sem valor: recusada antes de enviar (senão o servidor espera dados e a conexão trava), e a sessão segue boa.
    let err = s.execute("", "SELECT 1 FROM dual WHERE 1 = :x", 10).await.unwrap_err();
    assert_eq!(err.code.as_deref(), Some("ORA-01008"));
    assert_eq!(err.position, Some(30));
    s.execute("", "SELECT 1 FROM dual", 10).await.unwrap();

    // Vários comandos e PL/SQL.
    let out = s.execute("", &format!("UPDATE {qt} SET n = 2 WHERE n = 1.5; SELECT COUNT(*) FROM {qt}; BEGIN NULL; END;"), 10).await.unwrap();
    assert_eq!(out.statements.len(), 3);
    assert_eq!(out.statements[0].rows_affected, Some(1));
    s.execute("", &format!("DROP TABLE {qt} PURGE"), 10).await.unwrap();
}

#[tokio::test]
async fn live_cancel_stops_a_running_query_and_keeps_the_session() {
    let Some(p) = params() else { return };
    let s = std::sync::Arc::new(OracleSession::open(p).await.unwrap());
    let runner = s.clone();
    let slow = tokio::spawn(async move {
        runner
            .execute("", "SELECT COUNT(*) FROM (SELECT level l FROM dual CONNECT BY level <= 20000) a, (SELECT level l FROM dual CONNECT BY level <= 20000) b, (SELECT level l FROM dual CONNECT BY level <= 20000) c", 10)
            .await
    });
    tokio::time::sleep(Duration::from_millis(1500)).await;
    s.cancel().await.unwrap();
    let err = tokio::time::timeout(Duration::from_secs(20), slow).await.expect("the query was not interrupted").unwrap().unwrap_err();
    assert_eq!(err.code.as_deref(), Some("ORA-01013"));
    assert!(matches!(err.kind, ErrorKind::Canceled), "{err:?}");
    // A sessão continua boa, e cancelar sem nada rodando não afeta o comando seguinte.
    s.cancel().await.unwrap();
    let out = s.execute("", "SELECT 7 FROM dual", 10).await.unwrap();
    assert_eq!(out.statements[0].result.as_ref().unwrap().rows[0][0].as_deref(), Some("7"));
}

#[tokio::test]
async fn live_grid_sequence_open_count_filter_repeats() {
    let Some(p) = params() else { return };
    let s = OracleSession::open(p).await.unwrap();
    let user = s.current_user.clone();
    let t = format!("ATLAS_G_{}", std::process::id());
    let qt = quote_ident(&t);
    s.execute("", &format!("CREATE TABLE {qt} (id NUMBER PRIMARY KEY, nome VARCHAR2(60), obs CLOB)"), 10).await.unwrap();
    s.execute("", &format!("INSERT INTO {qt} SELECT level, 'nome ' || level, RPAD('o', 200, 'x') FROM dual CONNECT BY level <= 150"), 10).await.unwrap();
    let order = [OrderBy { column: "ID".into(), descending: false }];
    for round in 0..4 {
        // A interface dispara pedidos juntos na mesma sessão (duas leituras de estrutura e a da grade ao abrir a tabela).
        let (d1, d2, rows) = tokio::join!(s.table_detail(&user, &t), s.table_detail(&user, &t), s.table_rows(&user, &t, &order, None, Some(1000), 0, 1000));
        assert_eq!(d1.unwrap().columns.len(), 3, "rodada {round}");
        assert_eq!(d2.unwrap().columns.len(), 3, "rodada {round}");
        assert_eq!(rows.unwrap().1.statements[0].result.as_ref().unwrap().rows.len(), 150, "rodada {round}");
        let (_, all) = s.table_rows(&user, &t, &order, None, Some(1000), 0, 1000).await.unwrap();
        assert_eq!(all.statements[0].result.as_ref().unwrap().rows.len(), 150, "rodada {round}");
        assert_eq!(s.table_count(&user, &t, None).await.unwrap(), 150, "rodada {round}");
        let f = TableFilter { expr: "\"ID\" = 42".into() };
        let (_, one) = s.table_rows(&user, &t, &order, Some(&f), Some(1000), 0, 1000).await.unwrap();
        assert_eq!(one.statements[0].result.as_ref().unwrap().rows.len(), 1, "rodada {round}");
        assert_eq!(s.table_count(&user, &t, Some(&f)).await.unwrap(), 1, "rodada {round}");
    }
    s.execute("", &format!("DROP TABLE {qt} PURGE"), 10).await.unwrap();
}
