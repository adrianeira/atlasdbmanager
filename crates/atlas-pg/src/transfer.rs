//! Data Transfer: copia objetos de um schema (de uma conexão) para um schema de outra conexão, ou do mesmo servidor.
//!
//! A estrutura vem do planejador do Dump SQL (`DumpScope::Selection`), lida num snapshot da origem. O destino recebe
//! primeiro a estrutura (tipos, sequências, funções, tabelas), depois os dados em lotes pelo motor de importação
//! (`run_import`: parâmetros com `cast`, nada vai no texto do SQL) e por último o que depende dos dados (restrições,
//! índices, chaves estrangeiras, views, gatilhos, valores das sequências).
//!
//! Sem "continuar em erro", a estrutura e o pós-dados são cada um uma transação (tudo ou nada) e o primeiro erro de
//! dados interrompe a transferência; com ele, cada comando segue sozinho e os erros são só registrados.

use crate::connect::PgSession;
use crate::dump::{DumpOptions, DumpScope, begin_snapshot, end_snapshot, plan};
use crate::error::map_error;
use crate::exec::quote_ident;
use crate::import::{Feed, Hooks as ImportHooks, ImportMode, ImportSpec, TargetColumn, run as run_import};
use crate::meta::table_detail;
use atlas_core::{DatabaseError, ErrorKind};
use serde::Deserialize;
use std::time::Instant;
use tokio_postgres::SimpleQueryMessage;

const DEFAULT_BATCH: usize = 1000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldMap {
    pub source: String,
    pub target: String,
}

/// Ajustes de uma tabela (modo "Advanced" do assistente).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableOverride {
    pub table: String,
    /// Nome da tabela no destino.
    #[serde(default)]
    pub target_name: Option<String>,
    /// Campos escolhidos (origem → destino); vazio/ausente = todos, com o mesmo nome.
    #[serde(default)]
    pub fields: Option<Vec<FieldMap>>,
    /// Linhas por lote, cada lote numa transação; ausente = a tabela inteira numa transação só.
    #[serde(default)]
    pub rows_per_batch: Option<usize>,
    /// Filtros (`WHERE`): cada um é um "recordset" transferido à parte; ausente = a tabela toda.
    #[serde(default)]
    pub recordsets: Option<Vec<String>>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferSpec {
    pub source_schema: String,
    pub target_schema: String,
    #[serde(default)]
    pub tables: Vec<String>,
    #[serde(default)]
    pub views: Vec<String>,
    #[serde(default)]
    pub functions: Vec<String>,
    #[serde(default)]
    pub sequences: Vec<String>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub overrides: Vec<TableOverride>,
    /// `DROP ... IF EXISTS ... CASCADE` dos objetos do destino antes de criá-los.
    #[serde(default)]
    pub drop_before_create: bool,
    #[serde(default)]
    pub continue_on_error: bool,
    #[serde(default = "yes")]
    pub structure: bool,
    #[serde(default = "yes")]
    pub data: bool,
}

#[derive(Debug, Clone, Default)]
pub struct TransferStats {
    /// Objetos planejados (tabelas, views, sequências, funções, tipos).
    pub objects: u64,
    pub processed: u64,
    pub transferred: u64,
    pub errors: u64,
}

pub struct Hooks<'a> {
    pub log: &'a mut (dyn FnMut(String) + Send),
    pub progress: &'a mut (dyn FnMut(&TransferStats) + Send),
    pub cancelled: &'a (dyn Fn() -> bool + Sync),
}

fn sql_err(e: tokio_postgres::Error) -> DatabaseError {
    map_error(&e)
}

/// Primeira linha útil de um comando, curta, para o registro.
fn head(stmt: &str) -> String {
    let line = stmt.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if line.chars().count() > 110 { format!("{}...", line.chars().take(110).collect::<String>()) } else { line.to_string() }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '$' | '"' | '.')
}

/// Troca o schema de origem pelo de destino nos nomes qualificados (`origem.tabela` e `"origem".tabela`), fora de textos
/// entre aspas simples. Os comandos do planejador sempre qualificam os objetos.
pub fn rewrite_schema(sql: &str, from: &str, to: &str) -> String {
    if from == to {
        return sql.to_string();
    }
    // `SELECT pg_catalog.setval('schema.seq', ...)`: o nome da sequência vai dentro de um texto, e é ele que muda.
    const SETVAL: &str = "SELECT pg_catalog.setval('";
    if let Some(body) = sql.strip_prefix(SETVAL) {
        let b = body.as_bytes();
        let mut i = 0;
        let mut end = None;
        while i < b.len() {
            if b[i] == b'\'' {
                if b.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                end = Some(i);
                break;
            }
            i += 1;
        }
        if let Some(e) = end {
            let (name, tail) = body.split_at(e);
            return format!("{SETVAL}{}{tail}", rewrite_core(name, from, to, false));
        }
    }
    rewrite_core(sql, from, to, true)
}

fn rewrite_core(sql: &str, from: &str, to: &str, skip_strings: bool) -> String {
    // Sem aspas só se o nome é um identificador simples (como o planejador escreve); senão só a forma com aspas existe.
    let simple = from.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == '_') && from.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '$'));
    let plain = if simple { format!("{from}.") } else { String::new() };
    let quoted = format!("{}.", quote_ident(from));
    let target = format!("{}.", quote_ident(to));
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    let starts = |at: usize, pat: &str| -> bool { pat.chars().enumerate().all(|(k, pc)| chars.get(at + k) == Some(&pc)) };
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' && skip_strings {
            // Texto entre aspas simples: fica como está, menos `'schema.seq'::regclass` (o `DEFAULT nextval(...)` das colunas serial).
            let mut j = i + 1;
            while j < chars.len() {
                if chars[j] == '\'' {
                    if chars.get(j + 1) == Some(&'\'') {
                        j += 2;
                        continue;
                    }
                    break;
                }
                j += 1;
            }
            let end = j.min(chars.len().saturating_sub(1));
            let literal: String = chars[i + 1..end].iter().collect();
            let regclass = starts(end + 1, "::regclass");
            out.push('\'');
            out.push_str(&if regclass { rewrite_core(&literal, from, to, false) } else { literal });
            out.push('\'');
            i = end + 1;
            continue;
        }
        let boundary = i == 0 || !is_ident_char(chars[i - 1]);
        if boundary && starts(i, &quoted) {
            out.push_str(&target);
            i += quoted.chars().count();
            continue;
        }
        if boundary && !plain.is_empty() && starts(i, &plain) {
            out.push_str(&target);
            i += plain.chars().count();
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Ajusta um comando gerado a partir de um servidor novo para um destino mais antigo (o que dá para adaptar sem perder
/// nada): `CREATE SEQUENCE ... AS tipo` só existe a partir do PostgreSQL 10.
fn adapt_for_target(stmt: &str, target_version: i32) -> String {
    if target_version < 100000 && stmt.starts_with("CREATE SEQUENCE ") {
        return stmt
            .lines()
            .filter(|l| {
                let t = l.trim();
                !(t.starts_with("AS ") && matches!(t, "AS smallint" | "AS integer" | "AS bigint"))
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    stmt.to_string()
}

/// Roda comandos no destino. Sem `keep_going` é tudo ou nada (uma transação). Devolve `false` se parou por erro.
async fn run_statements(
    dst: &PgSession,
    statements: &[String],
    keep_going: bool,
    stats: &mut TransferStats,
    hooks: &mut Hooks<'_>,
) -> Result<bool, DatabaseError> {
    if statements.is_empty() {
        return Ok(true);
    }
    if !keep_going {
        dst.client.batch_execute("BEGIN").await.map_err(sql_err)?;
    }
    for stmt in statements {
        if (hooks.cancelled)() {
            if !keep_going {
                let _ = dst.client.batch_execute("ROLLBACK").await;
            }
            return Err(DatabaseError::new(ErrorKind::Canceled, "transfer canceled"));
        }
        (hooks.log)(format!("[DTF] {}", head(stmt)));
        if let Err(e) = dst.client.batch_execute(stmt).await {
            let err = map_error(&e);
            (hooks.log)(format!("[ERR] {}", err.message));
            stats.errors += 1;
            (hooks.progress)(stats);
            if keep_going {
                continue;
            }
            let _ = dst.client.batch_execute("ROLLBACK").await;
            return Ok(false);
        }
    }
    if !keep_going {
        dst.client.batch_execute("COMMIT").await.map_err(sql_err)?;
    }
    Ok(true)
}

async fn fetch_batch(src: &PgSession, cursor: &str, n: usize, columns: usize) -> Result<Vec<Vec<Option<String>>>, DatabaseError> {
    let msgs = src.client.simple_query(&format!("FETCH {n} FROM {cursor}")).await.map_err(sql_err)?;
    let mut rows = Vec::new();
    for m in msgs {
        if let SimpleQueryMessage::Row(r) = m {
            rows.push((0..columns).map(|i| r.get(i).map(str::to_string)).collect());
        }
    }
    Ok(rows)
}

/// Copia uma tabela (um filtro por vez). Devolve `false` se a transferência deve parar.
async fn copy_table_data(
    src: &PgSession,
    dst: &PgSession,
    spec: &TransferSpec,
    table: &crate::dump::DumpTable,
    ov: Option<&TableOverride>,
    stats: &mut TransferStats,
    hooks: &mut Hooks<'_>,
) -> Result<bool, DatabaseError> {
    let started = Instant::now();
    let target_name = ov.and_then(|o| o.target_name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| table.name.clone());
    let pairs: Vec<(String, String)> = match ov.and_then(|o| o.fields.as_ref()).filter(|f| !f.is_empty()) {
        Some(f) => f.iter().map(|m| (m.source.clone(), if m.target.is_empty() { m.source.clone() } else { m.target.clone() })).collect(),
        None => table.columns.iter().map(|c| (c.clone(), c.clone())).collect(),
    };
    let detail = match table_detail(dst, &spec.target_schema, &target_name).await {
        Ok(d) if !d.columns.is_empty() => d,
        _ => {
            (hooks.log)(format!("[ERR] {}: table {}.{} was not found in the target", table.name, spec.target_schema, target_name));
            stats.errors += 1;
            (hooks.progress)(stats);
            return Ok(spec.continue_on_error);
        }
    };
    let mut columns = Vec::new();
    for (_, target) in &pairs {
        let Some(c) = detail.columns.iter().find(|c| c.name == *target) else {
            (hooks.log)(format!("[ERR] {}: target field \"{target}\" does not exist in {target_name}", table.name));
            stats.errors += 1;
            (hooks.progress)(stats);
            return Ok(spec.continue_on_error);
        };
        columns.push(TargetColumn { name: c.name.clone(), cast_type: c.formatted_type.clone(), identity_always: c.identity.as_deref() == Some("a") });
    }
    let select_list = pairs.iter().map(|(s, _)| quote_ident(s)).collect::<Vec<_>>().join(", ");
    let batch = ov.and_then(|o| o.rows_per_batch).filter(|n| *n > 0);
    let recordsets: Vec<Option<String>> = match ov.and_then(|o| o.recordsets.as_ref()).filter(|r| r.iter().any(|x| !x.trim().is_empty())) {
        Some(r) => r.iter().filter(|x| !x.trim().is_empty()).map(|x| Some(x.trim().to_string())).collect(),
        None => vec![None],
    };
    (hooks.log)(format!("[DTF] {}: transferring to {}", table.name, target_name));
    let mut table_rows = 0u64;
    for (n, rs) in recordsets.iter().enumerate() {
        let filter = rs.as_ref().map(|w| format!(" WHERE {w}")).unwrap_or_default();
        let cursor = format!("atlas_tr_{n}");
        src.client
            .batch_execute(&format!("DECLARE {cursor} NO SCROLL CURSOR FOR SELECT {select_list} FROM {}{filter}", table.quoted))
            .await
            .map_err(sql_err)?;
        // Tabela inteira numa transação só quando não há lote.
        let whole = batch.is_none();
        if whole {
            dst.client.batch_execute("BEGIN").await.map_err(sql_err)?;
        }
        let mut failed = false;
        loop {
            if (hooks.cancelled)() {
                if whole {
                    let _ = dst.client.batch_execute("ROLLBACK").await;
                }
                let _ = src.client.batch_execute(&format!("CLOSE {cursor}")).await;
                return Err(DatabaseError::new(ErrorKind::Canceled, "transfer canceled"));
            }
            let rows = match fetch_batch(src, &cursor, batch.unwrap_or(DEFAULT_BATCH), columns.len()).await {
                Ok(r) => r,
                Err(e) => {
                    if whole {
                        let _ = dst.client.batch_execute("ROLLBACK").await;
                    }
                    (hooks.log)(format!("[ERR] {}: {}", table.name, e.message));
                    stats.errors += 1;
                    (hooks.progress)(stats);
                    failed = true;
                    break;
                }
            };
            if rows.is_empty() {
                break;
            }
            let import = ImportSpec {
                schema: spec.target_schema.clone(),
                table: target_name.clone(),
                create: None,
                columns: columns.clone(),
                key_columns: Vec::new(),
                mode: ImportMode::Append,
                extended_insert: true,
                max_statement_bytes: 1024 * 1024,
                continue_on_error: spec.continue_on_error,
                manage_transaction: !whole,
            };
            let mut it = rows.into_iter();
            let mut feed = move || it.next().map(Feed::Row).unwrap_or(Feed::End);
            let mut lines: Vec<String> = Vec::new();
            let result = {
                let mut on_log = |l: String| lines.push(l);
                let mut on_progress = |_: &crate::import::ImportStats| {};
                run_import(dst, &import, &mut feed, ImportHooks { log: &mut on_log, progress: &mut on_progress, cancelled: hooks.cancelled }).await
            };
            for l in lines {
                (hooks.log)(l);
            }
            let st = match result {
                Ok(st) => st,
                Err(e) => {
                    if whole {
                        let _ = dst.client.batch_execute("ROLLBACK").await;
                    }
                    let _ = src.client.batch_execute(&format!("CLOSE {cursor}")).await;
                    return Err(e);
                }
            };
            stats.processed += st.processed;
            stats.transferred += st.added;
            stats.errors += st.errors;
            table_rows += st.added;
            (hooks.progress)(stats);
            if st.rolled_back {
                failed = true;
                break;
            }
        }
        let _ = src.client.batch_execute(&format!("CLOSE {cursor}")).await;
        if whole {
            if failed {
                let _ = dst.client.batch_execute("ROLLBACK").await;
            } else {
                dst.client.batch_execute("COMMIT").await.map_err(sql_err)?;
            }
        }
        if failed && !spec.continue_on_error {
            return Ok(false);
        }
    }
    (hooks.log)(format!("[DTF] {}: {} record(s) transferred ({:.2} s)", table.name, table_rows, started.elapsed().as_secs_f64()));
    Ok(true)
}

/// Executa a transferência. Em erro sem "continuar em erro" para no primeiro e devolve o que já foi feito (o erro vai no
/// registro e em `errors`); cancelamento e perda de conexão voltam como `Err`.
pub async fn run(src: &PgSession, dst: &PgSession, spec: &TransferSpec, mut hooks: Hooks<'_>) -> Result<TransferStats, DatabaseError> {
    let mut stats = TransferStats::default();
    // Texto estável dos valores, qualquer que seja a configuração da sessão de origem.
    src.client
        .batch_execute("SET bytea_output = 'hex'; SET datestyle = 'ISO, MDY'; SET intervalstyle = 'postgres'; SET extra_float_digits = 3")
        .await
        .map_err(sql_err)?;
    begin_snapshot(src).await?;
    let outcome = async {
        let opts = DumpOptions {
            scope: DumpScope::Selection {
                schema: spec.source_schema.clone(),
                tables: spec.tables.clone(),
                views: spec.views.clone(),
                functions: spec.functions.clone(),
                sequences: spec.sequences.clone(),
                types: spec.types.clone(),
            },
            structure: spec.structure,
            data: spec.data,
            clean: spec.drop_before_create,
            include_owner: false,
        };
        let planned = plan(src, &opts).await?;
        stats.objects = planned.objects as u64;
        (hooks.progress)(&stats);
        if spec.target_schema != "public" {
            dst.client.batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {}", quote_ident(&spec.target_schema))).await.map_err(sql_err)?;
        }
        let target_version = dst.server.version_num;
        let rewrite = |list: &[String]| -> Vec<String> {
            list.iter().map(|s| adapt_for_target(&rewrite_schema(s, &spec.source_schema, &spec.target_schema), target_version)).collect()
        };
        if !run_statements(dst, &rewrite(&planned.pre_data), spec.continue_on_error, &mut stats, &mut hooks).await? {
            return Ok(());
        }
        if spec.data {
            for t in &planned.data_tables {
                let ov = spec.overrides.iter().find(|o| o.table == t.name);
                if !copy_table_data(src, dst, spec, t, ov, &mut stats, &mut hooks).await? {
                    return Ok(());
                }
            }
        }
        run_statements(dst, &rewrite(&planned.post_data), spec.continue_on_error, &mut stats, &mut hooks).await?;
        Ok::<(), DatabaseError>(())
    }
    .await;
    end_snapshot(src).await;
    outcome?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::{adapt_for_target, rewrite_schema};

    #[test]
    fn sequences_lose_the_as_clause_for_old_targets() {
        let stmt = "CREATE SEQUENCE s.a\n    AS integer\n    START WITH 1\n    CACHE 1;";
        assert_eq!(adapt_for_target(stmt, 90619), "CREATE SEQUENCE s.a\n    START WITH 1\n    CACHE 1;");
        assert_eq!(adapt_for_target(stmt, 180006), stmt);
    }

    #[test]
    fn rewrites_qualified_names_outside_strings() {
        assert_eq!(rewrite_schema("CREATE TABLE public.t (a int)", "public", "novo"), "CREATE TABLE \"novo\".t (a int)");
        assert_eq!(rewrite_schema("ALTER TABLE \"Meu.Sch\".t ADD x int", "Meu.Sch", "dest"), "ALTER TABLE \"dest\".t ADD x int");
        // Texto entre aspas simples e nomes que só terminam igual não mudam.
        assert_eq!(rewrite_schema("COMMENT ON TABLE public.t IS 'veja public.x'", "public", "novo"), "COMMENT ON TABLE \"novo\".t IS 'veja public.x'");
        assert_eq!(rewrite_schema("SELECT republic.x, public.y", "public", "novo"), "SELECT republic.x, \"novo\".y");
        assert_eq!(rewrite_schema("SELECT 1", "public", "public"), "SELECT 1");
        // `DEFAULT nextval('schema.seq'::regclass)` das colunas serial também muda.
        assert_eq!(
            rewrite_schema("id integer DEFAULT nextval('public.t_id_seq'::regclass) NOT NULL, nota text DEFAULT 'public.x'", "public", "novo"),
            "id integer DEFAULT nextval('\"novo\".t_id_seq'::regclass) NOT NULL, nota text DEFAULT 'public.x'"
        );
        // O nome da sequência no setval muda; os números ficam.
        assert_eq!(
            rewrite_schema("SELECT pg_catalog.setval('public.avulsa', 10, true);", "public", "novo"),
            "SELECT pg_catalog.setval('\"novo\".avulsa', 10, true);"
        );
    }
}
