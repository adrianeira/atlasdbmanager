//! Execução de SQL do editor e leitura paginada de tabelas.

use crate::connect::PgSession;
use crate::error::map_error;
use atlas_core::{DatabaseError, OrderBy, QueryOutcome, ResultColumn, ResultSet, StatementOutcome, TableFilter};
use futures_util::{StreamExt, pin_mut};
use std::time::Instant;
use tokio_postgres::SimpleQueryMessage;

/// Quoting de identificador PostgreSQL: aspas duplas e aspas internas duplicadas (ADR-008).
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Executa o texto como o usuário escreveu, pelo protocolo simples do PostgreSQL: o próprio
/// servidor separa os comandos (sem split por `;` no cliente — ADR-016) e devolve os valores
/// já formatados em texto, sem perda de precisão.
///
/// Cada result set guarda no máximo `max_rows` linhas; o excedente é lido e descartado
/// (memória limitada) e o result set é marcado como `truncated`.
pub async fn execute(s: &PgSession, sql: &str, max_rows: usize) -> Result<QueryOutcome, DatabaseError> {
    let started = Instant::now();
    let result = execute_inner(s, sql, max_rows, started).await;
    s.client.log_command(sql, started.elapsed().as_secs_f64(), result.as_ref().err());
    result
}

async fn execute_inner(s: &PgSession, sql: &str, max_rows: usize, started: Instant) -> Result<QueryOutcome, DatabaseError> {

    // Os tipos das colunas vêm de descrever o comando (`prepare`), mas só DEPOIS de executá-lo: descrever antes
    // faz o servidor recusar textos com vários comandos ou com erro, e dentro de uma transação aberta pelo
    // usuário essa recusa a abortaria — o erro real seria trocado por "current transaction is aborted". Com o
    // comando já executado com sucesso, descrevê-lo não falha.
    let stream = s.client.simple_query_raw(sql).await.map_err(|e| map_error(&e))?;
    pin_mut!(stream);

    let mut statements = Vec::new();
    let mut current: Option<ResultSet> = None;
    // O servidor responde um comando atrás do outro: o tempo de cada um é medido entre as respostas.
    let mut texts = crate::console::split_statements(sql).into_iter();
    let mut mark = Instant::now();
    let mut first_row: Option<Instant> = None;
    while let Some(msg) = stream.next().await {
        match msg.map_err(|e| map_error(&e))? {
            SimpleQueryMessage::RowDescription(cols) => {
                first_row = Some(Instant::now());
                current = Some(ResultSet {
                    columns: cols.iter().map(|c| ResultColumn { name: c.name().to_string(), type_name: None }).collect(),
                    rows: Vec::new(),
                    truncated: false,
                });
            }
            SimpleQueryMessage::Row(row) => {
                if let Some(rs) = current.as_mut() {
                    if rs.rows.len() < max_rows {
                        rs.rows.push((0..row.len()).map(|i| row.get(i).map(str::to_string)).collect());
                    } else {
                        rs.truncated = true;
                    }
                }
            }
            SimpleQueryMessage::CommandComplete(n) => {
                let now = Instant::now();
                let began = first_row.take().unwrap_or(now);
                statements.push(StatementOutcome {
                    result: current.take(),
                    rows_affected: Some(n),
                    sql: texts.next().unwrap_or_default(),
                    query_time: began.duration_since(mark).as_secs_f64(),
                    fetch_time: now.duration_since(began).as_secs_f64(),
                });
                mark = now;
            }
            _ => {}
        }
    }
    // Tipos: só para um único comando com um único conjunto de linhas (com vários comandos o servidor recusa o
    // `prepare`). Qualquer falha aqui é ignorada: os dados já foram lidos.
    if is_single_statement(sql)
        && statements.iter().filter(|st| st.result.is_some()).count() == 1
        && let Ok(stmt) = s.client.prepare(sql).await
    {
        let types: Vec<String> = stmt.columns().iter().map(|c| c.type_().name().to_string()).collect();
        if let Some(rs) = statements.iter_mut().find_map(|st| st.result.as_mut())
            && rs.columns.len() == types.len()
        {
            for (col, ty) in rs.columns.iter_mut().zip(types) {
                col.type_name = Some(ty);
            }
        }
    }
    Ok(QueryOutcome { statements, elapsed_ms: started.elapsed().as_millis() as u64 })
}

/// `true` se o texto tem no máximo um comando, isto é, nenhum `;` seguido de mais código fora de aspas,
/// `$tag$...$tag$` e comentários. Não separa comandos (ADR-016): só decide se vale descrever o texto antes de
/// executá-lo. Na dúvida erra para o lado seguro (não descrever).
pub fn is_single_statement(sql: &str) -> bool {
    let b = sql.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
    let mut i = 0;
    let mut after_semicolon = false;
    while i < b.len() {
        match b[i] {
            c if c.is_ascii_whitespace() => i += 1,
            b'-' if b.get(i + 1) == Some(&b'-') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                // Comentários de bloco aninham no PostgreSQL.
                let mut depth = 0;
                while i < b.len() {
                    if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                        depth += 1;
                        i += 2;
                    } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b';' => {
                after_semicolon = true;
                i += 1;
            }
            q @ (b'\'' | b'"') => {
                if after_semicolon {
                    return false;
                }
                // E'...' aceita \ como escape.
                let escapes = q == b'\'' && i > 0 && matches!(b[i - 1], b'e' | b'E') && (i < 2 || !is_ident(b[i - 2]));
                i += 1;
                while i < b.len() {
                    if escapes && b[i] == b'\\' {
                        i += 2;
                    } else if b[i] == q {
                        if b.get(i + 1) == Some(&q) {
                            i += 2; // aspa dobrada
                        } else {
                            i += 1;
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            b'$' => {
                if after_semicolon {
                    return false;
                }
                // $$ ou $tag$ abre um bloco de texto; $1 é só um parâmetro.
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                let tag_ok = b.get(j) == Some(&b'$') && (j == i + 1 || !b[i + 1].is_ascii_digit());
                if tag_ok {
                    let tag = &b[i..=j];
                    i = j + 1;
                    while i < b.len() && !b[i..].starts_with(tag) {
                        i += 1;
                    }
                    i = (i + tag.len()).min(b.len());
                } else {
                    i += 1;
                }
            }
            _ => {
                if after_semicolon {
                    return false;
                }
                i += 1;
            }
        }
    }
    true
}

/// Consulta do editor pronta para a exportação em streaming: sem o `;` final e, quando só algumas colunas foram
/// pedidas, embrulhada num `SELECT colunas FROM (...)`. O texto da consulta vai numa linha própria para que um
/// comentário `--` no fim não engula o parêntese de fechamento.
pub fn wrap_query(sql: &str, columns: &[String]) -> String {
    let body = sql.trim().trim_end_matches(';').trim_end();
    if columns.is_empty() {
        return body.to_string();
    }
    let list = columns.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ");
    format!("SELECT {list} FROM (\n{body}\n) AS atlas_export")
}

/// `search_path` para o schema escolhido no editor: o schema primeiro e `public` depois (o padrão do
/// PostgreSQL), para que funções de extensões instaladas em `public` continuem valendo. `pg_catalog`
/// é sempre procurado antes, implicitamente.
pub fn search_path_for(schema: &str) -> String {
    if schema == "public" {
        "\"public\"".to_string()
    } else {
        format!("{}, \"public\"", quote_ident(schema))
    }
}

/// Como `execute`, mas resolvendo nomes sem schema no `schema` escolhido no editor (`None` = padrão da
/// conexão). O `SET` vai numa mensagem à parte antes do texto do usuário: o texto não muda (as posições de
/// erro continuam certas) e comandos que não aceitam bloco de transação (`VACUUM`, `CREATE DATABASE`) seguem
/// funcionando. Depois da execução o caminho volta ao padrão, e a trava impede que outra aba do editor use a
/// mesma conexão entre o `SET` e o `RESET`.
pub async fn execute_in_schema(s: &PgSession, sql: &str, max_rows: usize, schema: Option<&str>) -> Result<QueryOutcome, DatabaseError> {
    let Some(schema) = schema.filter(|x| !x.is_empty()) else {
        return execute(s, sql, max_rows).await;
    };
    let _turn = s.exec_lock.lock().await;
    // O `SET` nunca pode impedir o comando do usuário: numa transação abortada ele é recusado (25P02), e o
    // que o usuário precisa rodar é justamente o `ROLLBACK`/`ROLLBACK TO`. Se falhar, o comando roda do mesmo
    // jeito (com o caminho padrão) e devolve o seu próprio resultado ou erro.
    let _ = s.client.simple_query(&format!("SET search_path TO {}", search_path_for(schema))).await;
    let outcome = execute(s, sql, max_rows).await;
    // Se o script deixou a transação abortada o RESET falha; o próximo SET/consulta já lida com isso.
    let _ = s.client.simple_query("RESET search_path").await;
    outcome
}

/// `SELECT` de uma tabela com o filtro e a ordenação do grid, sem `LIMIT`. `columns` vazio = `*`.
/// Só identificadores (entre aspas) entram no texto; o filtro é a expressão do próprio usuário.
pub fn select_sql(schema: &str, table: &str, columns: &[String], order_by: &[OrderBy], filter: Option<&TableFilter>) -> String {
    let list = if columns.is_empty() { "*".to_string() } else { columns.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ") };
    let mut sql = format!("SELECT {list} FROM {}.{}", quote_ident(schema), quote_ident(table));
    if let Some(f) = filter {
        let expr = f.expr.trim();
        if !expr.is_empty() {
            sql.push_str(&format!(" WHERE {expr}"));
        }
    }
    if !order_by.is_empty() {
        let cols: Vec<String> = order_by
            .iter()
            .map(|o| format!("{} {}", quote_ident(&o.column), if o.descending { "DESC" } else { "ASC" }))
            .collect();
        sql.push_str(&format!(" ORDER BY {}", cols.join(", ")));
    }
    sql
}

/// Lê o resultado de um único `SELECT` linha a linha, sem guardar nada na memória: `on_begin` recebe
/// as colunas (com o tipo, quando o servidor descreve) e `on_row` cada linha; devolver `Ok(false)`
/// encerra a leitura (cancelamento). Devolve quantas linhas foram entregues.
pub async fn stream_rows<B, R>(s: &PgSession, sql: &str, on_begin: B, on_row: R) -> Result<u64, DatabaseError>
where
    B: FnOnce(&[ResultColumn]) -> Result<(), DatabaseError>,
    R: FnMut(&[Option<&str>]) -> Result<bool, DatabaseError>,
{
    let started = Instant::now();
    let result = stream_rows_inner(s, sql, on_begin, on_row).await;
    s.client.log_command(sql, started.elapsed().as_secs_f64(), result.as_ref().err());
    result
}

async fn stream_rows_inner<B, R>(s: &PgSession, sql: &str, on_begin: B, mut on_row: R) -> Result<u64, DatabaseError>
where
    B: FnOnce(&[ResultColumn]) -> Result<(), DatabaseError>,
    R: FnMut(&[Option<&str>]) -> Result<bool, DatabaseError>,
{
    let described: Option<Vec<String>> = if is_single_statement(sql) {
        match s.client.prepare(sql).await {
            Ok(stmt) => Some(stmt.columns().iter().map(|c| c.type_().name().to_string()).collect()),
            Err(_) => None,
        }
    } else {
        None
    };
    let stream = s.client.simple_query_raw(sql).await.map_err(|e| map_error(&e))?;
    pin_mut!(stream);

    let mut on_begin = Some(on_begin);
    let mut count = 0u64;
    while let Some(msg) = stream.next().await {
        match msg.map_err(|e| map_error(&e))? {
            SimpleQueryMessage::RowDescription(cols) => {
                if let Some(begin) = on_begin.take() {
                    let columns: Vec<ResultColumn> = cols
                        .iter()
                        .enumerate()
                        .map(|(i, c)| ResultColumn {
                            name: c.name().to_string(),
                            type_name: described.as_ref().filter(|d| d.len() == cols.len()).map(|d| d[i].clone()),
                        })
                        .collect();
                    begin(&columns)?;
                }
            }
            SimpleQueryMessage::Row(row) => {
                let values: Vec<Option<&str>> = (0..row.len()).map(|i| row.get(i)).collect();
                count += 1;
                if !on_row(&values)? {
                    break;
                }
            }
            _ => {}
        }
    }
    Ok(count)
}

/// Página de uma tabela. A ordenação pela chave primária torna a paginação determinística
/// (spec §31.2); sem chave, a ordem do servidor não é garantida.
///
/// `limit = None` lê a tabela inteira ("Limitar registros" desmarcado); ainda assim a
/// memória fica limitada a `max_rows` linhas e o excedente é sinalizado como `truncated`.
#[allow(clippy::too_many_arguments)]
pub async fn table_rows(
    s: &PgSession,
    schema: &str,
    table: &str,
    order_by: &[OrderBy],
    filter: Option<&TableFilter>,
    limit: Option<u32>,
    offset: u64,
    max_rows: usize,
) -> Result<(String, QueryOutcome), DatabaseError> {
    let mut sql = select_sql(schema, table, &[], order_by, filter);
    let keep = match limit {
        Some(limit) => {
            sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}"));
            (limit as usize).min(max_rows)
        }
        None => max_rows,
    };
    let out = execute(s, &sql, keep).await?;
    Ok((sql, out))
}

/// Total de linhas (com o mesmo WHERE) para o botão "Última página" / numeração.
pub async fn table_count(
    s: &PgSession,
    schema: &str,
    table: &str,
    filter: Option<&TableFilter>,
) -> Result<u64, DatabaseError> {
    let mut sql = format!("SELECT COUNT(*) FROM {}.{}", quote_ident(schema), quote_ident(table));
    if let Some(f) = filter {
        let expr = f.expr.trim();
        if !expr.is_empty() {
            sql.push_str(&format!(" WHERE {expr}"));
        }
    }
    let out = execute(s, &sql, 1).await?;
    let n = out
        .statements
        .first()
        .and_then(|st| st.result.as_ref())
        .and_then(|rs| rs.rows.first())
        .and_then(|r| r.first())
        .and_then(|v| v.as_ref())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::{is_single_statement, quote_ident};

    #[test]
    fn wrap_query_strips_the_trailing_semicolon_and_selects_columns() {
        use super::wrap_query;
        assert_eq!(wrap_query("  select 1 as a ;\n", &[]), "select 1 as a");
        assert_eq!(wrap_query("select 1;;", &[]), "select 1");
        assert_eq!(
            wrap_query("select 1 as a, 2 as \"B c\" -- fim", &["a".into(), "B c".into()]),
            "SELECT \"a\", \"B c\" FROM (\nselect 1 as a, 2 as \"B c\" -- fim\n) AS atlas_export"
        );
    }

    #[test]
    fn single_statement_detection() {
        for one in [
            "select 1", "select 1;", "  select 1 ;  ", "select 1; -- fim", "select 1; /* fim */", "select ';'", "select \"a;b\"",
            "select $$a;b$$", "select $tag$ a ; b $tag$;", "select 1 -- ; x", "/* ; */ select 1", "select 'it''s;'", "select E'\\';' ",
            "select $1, $2", "", "   ", "begin", "select 'ç;ã'",
        ] {
            assert!(is_single_statement(one), "deveria ser único: {one:?}");
        }
        for many in ["select 1; select 2", "begin; select 1", "select 1;select 2;", "update t set a = 1; commit", "select 1; 'x'", "select 1; \"y\"", "select 1; $$z$$"] {
            assert!(!is_single_statement(many), "deveria ser lote: {many:?}");
        }
    }

    #[test]
    fn quoting_escapes_embedded_quotes() {
        assert_eq!(quote_ident("audit_log"), "\"audit_log\"");
        assert_eq!(quote_ident("Weird.Schema"), "\"Weird.Schema\"");
        assert_eq!(quote_ident("weird\"; DROP TABLE x; --"), "\"weird\"\"; DROP TABLE x; --\"");
    }
}


/// Rótulos de chaves estrangeiras: para cada chave pedida, o texto da coluna de descrição da tabela referenciada.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FkLabels {
    /// Coluna usada como descrição; `None` se a tabela não tem uma que sirva (a tela mostra a chave mesmo).
    pub label_column: Option<String>,
    /// Pares `[chave, rótulo]` (só das chaves que existem na tabela).
    pub labels: Vec<[String; 2]>,
}

/// Máximo de chaves por chamada (a grade mostra uma página por vez).
const MAX_FK_KEYS: usize = 5_000;

/// Escolhe a coluna de descrição: nomes típicos primeiro (`nome`, `name`, `descricao`...), depois a primeira coluna de texto
/// que não seja a própria chave nem pareça outra chave (`..._id`, `id_...`, `codigo`).
fn pick_label_column(columns: &[(String, String)], key: &str) -> Option<String> {
    let textual = |t: &str| matches!(t, "text" | "varchar" | "bpchar" | "citext" | "name");
    let candidates: Vec<&(String, String)> = columns.iter().filter(|(n, t)| n != key && textual(t)).collect();
    const PREFERRED: [&str; 12] = ["nome", "name", "titulo", "title", "descricao", "description", "descr", "label", "razao_social", "fantasia", "apelido", "rotulo"];
    for want in PREFERRED {
        if let Some((n, _)) = candidates.iter().find(|(n, _)| n.to_lowercase() == want) {
            return Some(n.clone());
        }
    }
    for want in PREFERRED {
        if let Some((n, _)) = candidates.iter().find(|(n, _)| n.to_lowercase().contains(want)) {
            return Some(n.clone());
        }
    }
    candidates
        .iter()
        .find(|(n, _)| {
            let l = n.to_lowercase();
            !(l.ends_with("_id") || l.starts_with("id_") || l.starts_with("cod") || l == "id")
        })
        .map(|(n, _)| n.clone())
}

pub async fn fk_labels(s: &PgSession, schema: &str, table: &str, key_column: &str, keys: &[String]) -> Result<FkLabels, DatabaseError> {
    let none = FkLabels { label_column: None, labels: Vec::new() };
    if keys.is_empty() {
        return Ok(none);
    }
    let cols = s
        .client
        .query(
            "SELECT a.attname::text, t.typname::text FROM pg_catalog.pg_attribute a \
               JOIN pg_catalog.pg_class c ON c.oid = a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
               JOIN pg_catalog.pg_type t ON t.oid = a.atttypid \
              WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum",
            &[&schema, &table],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let columns: Vec<(String, String)> = cols.iter().map(|r| (r.get(0), r.get(1))).collect();
    let Some(label) = pick_label_column(&columns, key_column) else { return Ok(none) };
    let keys: Vec<&str> = keys.iter().take(MAX_FK_KEYS).map(String::as_str).collect();
    let target = format!("{}.{}", quote_ident(schema), quote_ident(table));
    let sql = format!(
        "SELECT {k}::text, {l}::text FROM {target} WHERE {k}::text = ANY($1) AND {l} IS NOT NULL",
        k = quote_ident(key_column),
        l = quote_ident(&label),
    );
    let started = Instant::now();
    let rows = s.client.query(&sql, &[&keys]).await.map_err(|e| map_error(&e))?;
    s.client.log_command(&sql, started.elapsed().as_secs_f64(), None);
    Ok(FkLabels { label_column: Some(label), labels: rows.iter().map(|r| [r.get(0), r.get(1)]).collect() })
}

#[cfg(test)]
mod label_tests {
    use super::pick_label_column;

    fn cols(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect()
    }

    #[test]
    fn picks_a_description_column() {
        assert_eq!(pick_label_column(&cols(&[("id", "int4"), ("name", "varchar"), ("code", "text")]), "id").as_deref(), Some("name"));
        assert_eq!(pick_label_column(&cols(&[("id", "int4"), ("cod", "text"), ("descricao_longa", "text")]), "id").as_deref(), Some("descricao_longa"));
        // Sem nome típico: a primeira de texto que não parece chave.
        assert_eq!(pick_label_column(&cols(&[("id", "int4"), ("cliente_id", "text"), ("observacao", "text")]), "id").as_deref(), Some("observacao"));
        // Nada que sirva.
        assert_eq!(pick_label_column(&cols(&[("id", "int4"), ("qtd", "int4")]), "id"), None);
        assert_eq!(pick_label_column(&cols(&[("id", "text")]), "id"), None);
    }
}
