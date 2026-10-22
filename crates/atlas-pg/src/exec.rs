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

    // Os tipos das colunas vêm de descrever o comando (`prepare`), mas só DEPOIS de executá-lo: descrever antes
    // faz o servidor recusar textos com vários comandos ou com erro, e dentro de uma transação aberta pelo
    // usuário essa recusa a abortaria — o erro real seria trocado por "current transaction is aborted". Com o
    // comando já executado com sucesso, descrevê-lo não falha.
    let stream = s.client.simple_query_raw(sql).await.map_err(|e| map_error(&e))?;
    pin_mut!(stream);

    let mut statements = Vec::new();
    let mut current: Option<ResultSet> = None;
    while let Some(msg) = stream.next().await {
        match msg.map_err(|e| map_error(&e))? {
            SimpleQueryMessage::RowDescription(cols) => {
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
                statements.push(StatementOutcome { result: current.take(), rows_affected: Some(n) });
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
pub(crate) fn is_single_statement(sql: &str) -> bool {
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
pub async fn stream_rows<B, R>(s: &PgSession, sql: &str, on_begin: B, mut on_row: R) -> Result<u64, DatabaseError>
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
