//! Gravação das linhas de uma importação numa tabela do SQL Server, em uma transação, nos seis modos do assistente.
//!
//! As linhas chegam prontas (texto já convertido, `None` = NULL) e vão como parâmetros (`@P1`...): nenhum valor entra no texto do
//! SQL e o servidor converte para o tipo da coluna. Com "continuar em erro" cada comando roda sob um ponto de retorno
//! (`SAVE TRANSACTION`), então uma linha ruim é registrada e pulada sem perder o resto; sem isso o primeiro erro desfaz tudo.

use super::{Conn, MssqlSession, map_err, quote_ident};
use atlas_core::{DatabaseError, ErrorKind};
use serde::Deserialize;
use std::time::{Duration, Instant};

/// O SQL Server aceita 2100 parâmetros e 1000 linhas por `INSERT ... VALUES`.
const MAX_PARAMS: usize = 2_000;
const MAX_VALUE_ROWS: usize = 1_000;
/// Quantos erros são descritos no registro (depois disso só entram na contagem).
const MAX_LOGGED_ERRORS: u64 = 200;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImportMode {
    Append,
    Update,
    AppendUpdate,
    AppendNoUpdate,
    Delete,
    Copy,
}

#[derive(Debug, Clone)]
pub struct TargetColumn {
    pub name: String,
    /// Tipo da coluna (`int`, `bit`, `nvarchar(50)`...).
    pub declared_type: String,
}

#[derive(Debug, Clone)]
pub struct ImportSpec {
    pub schema: String,
    pub table: String,
    /// Tabela nova: colunas (nome, tipo) criadas antes de importar.
    pub create: Option<Vec<(String, String)>>,
    pub columns: Vec<TargetColumn>,
    /// Posições (em `columns`) das colunas-chave.
    pub key_columns: Vec<usize>,
    pub mode: ImportMode,
    pub extended_insert: bool,
    pub max_statement_bytes: usize,
    pub continue_on_error: bool,
}

/// O que o leitor entrega a cada chamada.
pub enum Feed {
    Row(Vec<Option<String>>),
    /// Linha que não pôde ser lida/convertida: conta como erro e segue.
    Bad(String),
    End,
}

#[derive(Debug, Clone, Default)]
pub struct ImportStats {
    pub processed: u64,
    pub added: u64,
    pub updated: u64,
    pub deleted: u64,
    pub errors: u64,
    /// Terminou em erro e a transação foi desfeita (nada ficou gravado).
    pub rolled_back: bool,
}

/// Ganchos da execução: registro de linhas, progresso (a cada ~200 ms) e cancelamento.
pub struct Hooks<'a> {
    pub log: &'a mut (dyn FnMut(String) + Send),
    pub progress: &'a mut (dyn FnMut(&ImportStats) + Send),
    pub cancelled: &'a (dyn Fn() -> bool + Sync),
}

/// O tipo do SQL Server para um tipo do PostgreSQL vindo da inferência do assistente.
pub fn declared_type(pg_type: &str) -> String {
    let t = pg_type.trim().to_lowercase();
    let base = t.split('(').next().unwrap_or(&t).trim();
    match base {
        "integer" | "int4" => "int".into(),
        "int8" => "bigint".into(),
        "int2" => "smallint".into(),
        "numeric" | "decimal" if !t.contains('(') => "decimal(38, 10)".into(),
        "boolean" | "bool" => "bit".into(),
        "double precision" | "float8" => "float".into(),
        "float4" => "real".into(),
        _ if base.starts_with("timestamp") => "datetime2".into(),
        "text" | "jsonb" | "json" | "uuid" | "interval" | "character varying" | "varchar" => "nvarchar(max)".into(),
        "bigint" | "smallint" | "int" | "date" | "time" | "real" | "float" | "bit" | "money" | "decimal" | "numeric" | "nvarchar" | "datetime2" => t,
        _ => "nvarchar(max)".into(),
    }
}

struct Runner<'a, 'h> {
    conn: &'a mut Conn,
    spec: &'a ImportSpec,
    hooks: Hooks<'h>,
    stats: ImportStats,
    last_progress: Instant,
    row_no: u64,
    table: String,
    /// Coluna `bit`: o assistente entrega `true`/`false` e o servidor guarda 1/0.
    bools: Vec<bool>,
}

fn row_error(e: DatabaseError, row_no: u64) -> DatabaseError {
    DatabaseError { message: format!("row {row_no}: {}", e.message), ..e }
}

impl Runner<'_, '_> {
    fn column_list(&self) -> String {
        self.spec.columns.iter().map(|c| quote_ident(&c.name)).collect::<Vec<_>>().join(", ")
    }

    fn insert_sql(&self, rows: usize) -> String {
        let n = self.spec.columns.len();
        let tuples: Vec<String> = (0..rows).map(|r| format!("({})", (0..n).map(|c| format!("@P{}", r * n + c + 1)).collect::<Vec<_>>().join(", "))).collect();
        format!("INSERT INTO {} ({}) VALUES {}", self.table, self.column_list(), tuples.join(", "))
    }

    fn key_condition(&self, param_of: impl Fn(usize) -> usize) -> String {
        self.spec.key_columns.iter().map(|&k| format!("{} = @P{}", quote_ident(&self.spec.columns[k].name), param_of(k))).collect::<Vec<_>>().join(" AND ")
    }

    fn update_sql(&self) -> Option<String> {
        let sets: Vec<String> = (0..self.spec.columns.len())
            .filter(|i| !self.spec.key_columns.contains(i))
            .map(|i| format!("{} = @P{}", quote_ident(&self.spec.columns[i].name), i + 1))
            .collect();
        if sets.is_empty() {
            return None;
        }
        Some(format!("UPDATE {} SET {} WHERE {}", self.table, sets.join(", "), self.key_condition(|k| k + 1)))
    }

    fn delete_sql(&self) -> String {
        let cond = self.spec.key_columns.iter().enumerate().map(|(pos, &k)| format!("{} = @P{}", quote_ident(&self.spec.columns[k].name), pos + 1)).collect::<Vec<_>>().join(" AND ");
        format!("DELETE FROM {} WHERE {cond}", self.table)
    }

    fn insert_if_missing_sql(&self) -> String {
        let values = (0..self.spec.columns.len()).map(|c| format!("@P{}", c + 1)).collect::<Vec<_>>().join(", ");
        format!("INSERT INTO {t} ({cols}) SELECT {values} WHERE NOT EXISTS (SELECT 1 FROM {t} WHERE {cond})", t = self.table, cols = self.column_list(), cond = self.key_condition(|k| k + 1))
    }

    /// O valor como o servidor o guarda: o booleano de texto vira 1/0.
    fn stored(&self, col: usize, v: &Option<String>) -> Option<String> {
        match v {
            Some(s) if self.bools[col] => Some(if s == "true" || s == "t" || s == "1" { "1" } else { "0" }.to_string()),
            other => other.clone(),
        }
    }

    async fn script(&mut self, sql: &str) -> Result<(), DatabaseError> {
        self.conn.simple_query(sql).await.map_err(map_err)?.into_results().await.map_err(map_err)?;
        Ok(())
    }

    async fn execute(&mut self, sql: &str, values: &[Option<String>]) -> Result<u64, DatabaseError> {
        let mut q = tiberius::Query::new(sql.to_string());
        for v in values {
            q.bind(v.clone());
        }
        Ok(q.execute(self.conn).await.map_err(map_err)?.total())
    }

    fn log_error(&mut self, message: String) {
        self.stats.errors += 1;
        if self.stats.errors <= MAX_LOGGED_ERRORS {
            (self.hooks.log)(format!("[ERR] {message}"));
        } else if self.stats.errors == MAX_LOGGED_ERRORS + 1 {
            (self.hooks.log)(format!("[ERR] more than {MAX_LOGGED_ERRORS} errors: the next ones are only counted"));
        }
    }

    fn tick(&mut self) -> Result<(), DatabaseError> {
        if (self.hooks.cancelled)() {
            return Err(DatabaseError::new(ErrorKind::Canceled, "import canceled"));
        }
        if self.last_progress.elapsed() >= Duration::from_millis(200) {
            self.last_progress = Instant::now();
            (self.hooks.progress)(&self.stats);
        }
        Ok(())
    }

    /// Um comando por linha sob ponto de retorno (quando se continua em erro). `Some(n)` = linhas afetadas; `None` = a linha foi pulada.
    async fn guarded(&mut self, sql: &str, values: &[Option<String>], row_no: u64) -> Result<Option<u64>, DatabaseError> {
        if !self.spec.continue_on_error {
            return self.execute(sql, values).await.map(Some).map_err(|e| row_error(e, row_no));
        }
        self.script("SAVE TRANSACTION atlas_row").await?;
        match self.execute(sql, values).await {
            Ok(n) => Ok(Some(n)),
            Err(e) => {
                self.script("ROLLBACK TRANSACTION atlas_row").await?;
                self.log_error(format!("row {row_no}: {}", e.message));
                Ok(None)
            }
        }
    }

    /// Grava as linhas acumuladas do modo "acrescentar" (em um `INSERT` estendido quando dá).
    async fn flush_inserts(&mut self, batch: &mut Vec<(u64, Vec<Option<String>>)>) -> Result<(), DatabaseError> {
        if batch.is_empty() {
            return Ok(());
        }
        let rows = std::mem::take(batch);
        if rows.len() > 1 {
            let sql = self.insert_sql(rows.len());
            let flat: Vec<Option<String>> = rows.iter().flat_map(|(_, r)| r.iter().cloned()).collect();
            if self.spec.continue_on_error {
                self.script("SAVE TRANSACTION atlas_batch").await?;
                match self.execute(&sql, &flat).await {
                    Ok(n) => {
                        self.stats.added += n;
                        return Ok(());
                    }
                    Err(_) => {
                        // Alguma linha do lote é ruim: refaz uma a uma para achar qual.
                        self.script("ROLLBACK TRANSACTION atlas_batch").await?;
                    }
                }
            } else {
                let n = self.execute(&sql, &flat).await.map_err(|e| row_error(e, rows[0].0))?;
                self.stats.added += n;
                return Ok(());
            }
        }
        let sql = self.insert_sql(1);
        for (no, row) in &rows {
            if let Some(n) = self.guarded(&sql, row, *no).await? {
                self.stats.added += n;
            }
        }
        Ok(())
    }

    async fn run(&mut self, feed: &mut (dyn FnMut() -> Feed + Send)) -> Result<(), DatabaseError> {
        let spec = self.spec;
        let needs_keys = matches!(spec.mode, ImportMode::Update | ImportMode::AppendUpdate | ImportMode::AppendNoUpdate | ImportMode::Delete);
        if needs_keys && spec.key_columns.is_empty() {
            return Err(DatabaseError::new(ErrorKind::InvalidInput, "this import mode needs at least one key field (mark it in the field mapping)"));
        }
        if spec.columns.is_empty() {
            return Err(DatabaseError::new(ErrorKind::InvalidInput, "no field is mapped to the destination table"));
        }
        let update_sql = self.update_sql();
        if matches!(spec.mode, ImportMode::Update | ImportMode::AppendUpdate) && update_sql.is_none() {
            return Err(DatabaseError::new(ErrorKind::InvalidInput, "every mapped field is a key field: there is nothing to update"));
        }
        let insert_one = self.insert_sql(1);
        let delete_sql = (!spec.key_columns.is_empty()).then(|| self.delete_sql());
        let missing_sql = (!spec.key_columns.is_empty()).then(|| self.insert_if_missing_sql());
        let max_rows = (MAX_PARAMS / spec.columns.len()).clamp(1, MAX_VALUE_ROWS);
        let mut batch: Vec<(u64, Vec<Option<String>>)> = Vec::new();
        let mut batch_bytes = 0usize;

        loop {
            self.tick()?;
            match feed() {
                Feed::End => break,
                Feed::Bad(message) => {
                    self.row_no += 1;
                    self.stats.processed += 1;
                    let no = self.row_no;
                    if !spec.continue_on_error {
                        return Err(DatabaseError::new(ErrorKind::InvalidInput, format!("row {no}: {message}")));
                    }
                    self.log_error(format!("row {no}: {message}"));
                }
                Feed::Row(raw) => {
                    self.row_no += 1;
                    self.stats.processed += 1;
                    let no = self.row_no;
                    let values: Vec<Option<String>> = raw.iter().enumerate().map(|(i, v)| self.stored(i, v)).collect();
                    match spec.mode {
                        ImportMode::Append | ImportMode::Copy => {
                            if spec.extended_insert {
                                batch_bytes += values.iter().map(|v| v.as_ref().map_or(4, |s| s.len() + 3)).sum::<usize>() + 8 * values.len();
                                batch.push((no, values));
                                if batch.len() >= max_rows || batch_bytes >= spec.max_statement_bytes {
                                    self.flush_inserts(&mut batch).await?;
                                    batch_bytes = 0;
                                }
                            } else if let Some(n) = self.guarded(&insert_one, &values, no).await? {
                                self.stats.added += n;
                            }
                        }
                        ImportMode::Update => {
                            if let Some(n) = self.guarded(update_sql.as_deref().unwrap_or_default(), &values, no).await? {
                                self.stats.updated += n;
                            }
                        }
                        ImportMode::AppendUpdate => {
                            if let Some(n) = self.guarded(update_sql.as_deref().unwrap_or_default(), &values, no).await? {
                                if n > 0 {
                                    self.stats.updated += n;
                                } else if let Some(a) = self.guarded(&insert_one, &values, no).await? {
                                    self.stats.added += a;
                                }
                            }
                        }
                        ImportMode::AppendNoUpdate => {
                            if let Some(n) = self.guarded(missing_sql.as_deref().unwrap_or_default(), &values, no).await? {
                                self.stats.added += n;
                            }
                        }
                        ImportMode::Delete => {
                            let keys: Vec<Option<String>> = spec.key_columns.iter().map(|&k| values[k].clone()).collect();
                            if let Some(n) = self.guarded(delete_sql.as_deref().unwrap_or_default(), &keys, no).await? {
                                self.stats.deleted += n;
                            }
                        }
                    }
                }
            }
        }
        self.flush_inserts(&mut batch).await
    }
}

impl MssqlSession {
    /// Importa as linhas de `feed` numa transação só, numa conexão própria. Em erro (sem "continuar em erro") desfaz tudo e
    /// devolve `rolled_back`. Cancelamento volta como `Err`.
    pub async fn import(&self, database: &str, spec: &ImportSpec, feed: &mut (dyn FnMut() -> Feed + Send), hooks: Hooks<'_>) -> Result<ImportStats, DatabaseError> {
        let mut conn = self.dedicated(database).await?;
        let table = format!("{}.{}", quote_ident(&spec.schema), quote_ident(&spec.table));
        let bools = spec.columns.iter().map(|c| c.declared_type.eq_ignore_ascii_case("bit")).collect();
        conn.simple_query("BEGIN TRANSACTION").await.map_err(map_err)?.into_results().await.map_err(map_err)?;
        let mut runner = Runner { conn: &mut conn, spec, hooks, stats: ImportStats::default(), last_progress: Instant::now(), row_no: 0, table, bools };
        let result = async {
            if let Some(cols) = &spec.create {
                let defs: Vec<String> = cols.iter().map(|(n, t)| format!("{} {}", quote_ident(n), declared_type(t))).collect();
                let sql = format!("CREATE TABLE {} ({})", runner.table, defs.join(", "));
                runner.script(&sql).await?;
                let line = format!("[IMP] table {} created", runner.table);
                (runner.hooks.log)(line);
            }
            if spec.mode == ImportMode::Copy {
                let sql = format!("DELETE FROM {}", runner.table);
                let n = runner.execute(&sql, &[]).await?;
                runner.stats.deleted += n;
                (runner.hooks.log)(format!("[IMP] {n} record(s) deleted from the destination before repopulating"));
            }
            runner.run(feed).await
        }
        .await;
        match result {
            Ok(()) => {
                let stats = runner.stats.clone();
                runner.script("COMMIT TRANSACTION").await?;
                Ok(stats)
            }
            Err(e) => {
                let _ = runner.script("ROLLBACK TRANSACTION").await;
                if e.kind == ErrorKind::Canceled || e.kind == ErrorKind::ConnectionLost {
                    return Err(e);
                }
                (runner.hooks.log)(format!("[ERR] {}", e.message));
                runner.stats.errors += 1;
                runner.stats.rolled_back = true;
                runner.stats.added = 0;
                runner.stats.updated = 0;
                runner.stats.deleted = 0;
                Ok(runner.stats)
            }
        }
    }
}
