//! Gravação das linhas de uma importação numa tabela, em uma transação, nos seis modos do assistente.
//!
//! As linhas chegam prontas (texto já convertido, `None` = NULL) e vão como parâmetros com `cast` para o tipo da coluna
//! (`$1::text::integer`): nenhum valor entra no texto do SQL. Com "continuar em erro" cada comando roda sob um
//! `SAVEPOINT`, então uma linha ruim é registrada e pulada sem perder o resto; sem isso o primeiro erro desfaz tudo.

use crate::connect::PgSession;
use crate::error::map_error;
use crate::exec::quote_ident;
use atlas_core::{DatabaseError, ErrorKind};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio_postgres::types::ToSql;

/// Limite de parâmetros de um comando do PostgreSQL (65535), com folga.
const MAX_PARAMS: usize = 60_000;
/// Quantos erros são descritos no registro (depois disso só entram na contagem).
const MAX_LOGGED_ERRORS: u64 = 200;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImportMode {
    /// Acrescenta as linhas.
    Append,
    /// Atualiza as linhas do destino que casam com a origem (pela chave).
    Update,
    /// Atualiza se existir; senão acrescenta.
    AppendUpdate,
    /// Acrescenta só o que ainda não existe (pela chave).
    AppendNoUpdate,
    /// Exclui do destino as linhas que casam com a origem.
    Delete,
    /// Apaga tudo no destino e repovoa com a origem.
    Copy,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetColumn {
    pub name: String,
    /// Tipo para o `cast` (`format_type`: `integer`, `numeric(19,4)`, `timestamp with time zone`...).
    pub cast_type: String,
    /// Identidade `GENERATED ALWAYS`: o `INSERT` precisa de `OVERRIDING SYSTEM VALUE`.
    #[serde(default)]
    pub identity_always: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSpec {
    pub schema: String,
    pub table: String,
    /// Tabela nova: colunas (nome, tipo) criadas antes de importar.
    #[serde(default)]
    pub create: Option<Vec<(String, String)>>,
    /// Colunas de destino na ordem dos valores de cada linha.
    pub columns: Vec<TargetColumn>,
    /// Posições (em `columns`) das colunas-chave: casam origem e destino nos modos que precisam.
    #[serde(default)]
    pub key_columns: Vec<usize>,
    pub mode: ImportMode,
    /// Várias linhas por `INSERT`.
    #[serde(default)]
    pub extended_insert: bool,
    /// Tamanho máximo estimado de um `INSERT` estendido, em bytes.
    #[serde(default = "default_statement_bytes")]
    pub max_statement_bytes: usize,
    #[serde(default)]
    pub continue_on_error: bool,
}

fn default_statement_bytes() -> usize {
    1024 * 1024
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

fn sql_err(e: tokio_postgres::Error) -> DatabaseError {
    map_error(&e)
}

fn param_refs<'a>(values: &[&'a Option<String>]) -> Vec<&'a (dyn ToSql + Sync)> {
    values.iter().map(|v| *v as &'a (dyn ToSql + Sync)).collect()
}

struct Runner<'a, 'h> {
    s: &'a PgSession,
    spec: &'a ImportSpec,
    hooks: Hooks<'h>,
    stats: ImportStats,
    last_progress: Instant,
    row_no: u64,
    prepared: HashMap<String, tokio_postgres::Statement>,
    table: String,
}

impl Runner<'_, '_> {
    fn target(&self) -> String {
        self.table.clone()
    }

    /// `$n::text::tipo` para a coluna `i`, usando o parâmetro `n` (1-based).
    fn placeholder(&self, col: usize, n: usize) -> String {
        format!("${n}::text::{}", self.spec.columns[col].cast_type)
    }

    fn column_list(&self) -> String {
        self.spec.columns.iter().map(|c| quote_ident(&c.name)).collect::<Vec<_>>().join(", ")
    }

    fn overriding(&self) -> &'static str {
        if self.spec.columns.iter().any(|c| c.identity_always) { " OVERRIDING SYSTEM VALUE" } else { "" }
    }

    fn insert_sql(&self, rows: usize) -> String {
        let n = self.spec.columns.len();
        let tuples: Vec<String> = (0..rows)
            .map(|r| format!("({})", (0..n).map(|c| self.placeholder(c, r * n + c + 1)).collect::<Vec<_>>().join(", ")))
            .collect();
        format!("INSERT INTO {} ({}){} VALUES {}", self.target(), self.column_list(), self.overriding(), tuples.join(", "))
    }

    fn key_condition(&self, first_param_of_key: impl Fn(usize) -> usize) -> String {
        self.spec
            .key_columns
            .iter()
            .map(|&k| format!("{} = {}", quote_ident(&self.spec.columns[k].name), self.placeholder(k, first_param_of_key(k))))
            .collect::<Vec<_>>()
            .join(" AND ")
    }

    fn update_sql(&self) -> Option<String> {
        let sets: Vec<String> = (0..self.spec.columns.len())
            .filter(|i| !self.spec.key_columns.contains(i))
            .map(|i| format!("{} = {}", quote_ident(&self.spec.columns[i].name), self.placeholder(i, i + 1)))
            .collect();
        if sets.is_empty() {
            return None;
        }
        Some(format!("UPDATE {} SET {} WHERE {}", self.target(), sets.join(", "), self.key_condition(|k| k + 1)))
    }

    /// `DELETE` pelas chaves: os parâmetros são só os valores das colunas-chave, na ordem de `key_columns`.
    fn delete_sql(&self) -> String {
        let cond = self
            .spec
            .key_columns
            .iter()
            .enumerate()
            .map(|(pos, &k)| format!("{} = {}", quote_ident(&self.spec.columns[k].name), self.placeholder(k, pos + 1)))
            .collect::<Vec<_>>()
            .join(" AND ");
        format!("DELETE FROM {} WHERE {cond}", self.target())
    }

    fn insert_if_missing_sql(&self) -> String {
        let n = self.spec.columns.len();
        let values = (0..n).map(|c| self.placeholder(c, c + 1)).collect::<Vec<_>>().join(", ");
        format!(
            "INSERT INTO {t} ({cols}){ov} SELECT {values} WHERE NOT EXISTS (SELECT 1 FROM {t} WHERE {cond})",
            t = self.target(),
            cols = self.column_list(),
            ov = self.overriding(),
            cond = self.key_condition(|k| k + 1)
        )
    }

    async fn statement(&mut self, sql: &str) -> Result<tokio_postgres::Statement, DatabaseError> {
        if let Some(st) = self.prepared.get(sql) {
            return Ok(st.clone());
        }
        let st = self.s.client.prepare(sql).await.map_err(sql_err)?;
        if self.prepared.len() > 16 {
            self.prepared.clear();
        }
        self.prepared.insert(sql.to_string(), st.clone());
        Ok(st)
    }

    async fn execute(&mut self, sql: &str, values: &[&Option<String>]) -> Result<u64, DatabaseError> {
        let st = self.statement(sql).await?;
        self.s.client.execute(&st, &param_refs(values)).await.map_err(sql_err)
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

    /// Um comando por linha sob SAVEPOINT (quando se continua em erro). `Ok(true)` = a linha entrou.
    async fn guarded(&mut self, sql: &str, values: &[&Option<String>], row_no: u64) -> Result<Option<u64>, DatabaseError> {
        if !self.spec.continue_on_error {
            return self.execute(sql, values).await.map(Some).map_err(|e| row_error(e, row_no));
        }
        self.s.client.batch_execute("SAVEPOINT atlas_row").await.map_err(sql_err)?;
        match self.execute(sql, values).await {
            Ok(n) => {
                self.s.client.batch_execute("RELEASE SAVEPOINT atlas_row").await.map_err(sql_err)?;
                Ok(Some(n))
            }
            Err(e) => {
                self.s.client.batch_execute("ROLLBACK TO SAVEPOINT atlas_row").await.map_err(sql_err)?;
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
            let flat: Vec<&Option<String>> = rows.iter().flat_map(|(_, r)| r.iter()).collect();
            if self.spec.continue_on_error {
                self.s.client.batch_execute("SAVEPOINT atlas_batch").await.map_err(sql_err)?;
                match self.execute(&sql, &flat).await {
                    Ok(n) => {
                        self.s.client.batch_execute("RELEASE SAVEPOINT atlas_batch").await.map_err(sql_err)?;
                        self.stats.added += n;
                        return Ok(());
                    }
                    Err(_) => {
                        // Alguma linha do lote é ruim: refaz uma a uma para achar qual.
                        self.s.client.batch_execute("ROLLBACK TO SAVEPOINT atlas_batch; RELEASE SAVEPOINT atlas_batch").await.map_err(sql_err)?;
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
            let refs: Vec<&Option<String>> = row.iter().collect();
            if let Some(n) = self.guarded(&sql, &refs, *no).await? {
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
        let (insert_one, delete_sql, missing_sql) = (self.insert_sql(1), self.delete_sql_if_keys(), self.missing_sql_if_keys());
        let max_rows = (MAX_PARAMS / spec.columns.len()).max(1);
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
                Feed::Row(values) => {
                    self.row_no += 1;
                    self.stats.processed += 1;
                    let no = self.row_no;
                    let refs: Vec<&Option<String>> = values.iter().collect();
                    match spec.mode {
                        ImportMode::Append | ImportMode::Copy => {
                            if spec.extended_insert {
                                batch_bytes += values.iter().map(|v| v.as_ref().map_or(4, |s| s.len() + 3)).sum::<usize>() + 8 * values.len();
                                batch.push((no, values));
                                if batch.len() >= max_rows || batch_bytes >= spec.max_statement_bytes {
                                    self.flush_inserts(&mut batch).await?;
                                    batch_bytes = 0;
                                }
                            } else if let Some(n) = self.guarded(&insert_one, &refs, no).await? {
                                self.stats.added += n;
                            }
                        }
                        ImportMode::Update => {
                            if let Some(n) = self.guarded(update_sql.as_deref().unwrap_or_default(), &refs, no).await? {
                                self.stats.updated += n;
                            }
                        }
                        ImportMode::AppendUpdate => {
                            if let Some(n) = self.guarded(update_sql.as_deref().unwrap_or_default(), &refs, no).await? {
                                if n > 0 {
                                    self.stats.updated += n;
                                } else if let Some(a) = self.guarded(&insert_one, &refs, no).await? {
                                    self.stats.added += a;
                                }
                            }
                        }
                        ImportMode::AppendNoUpdate => {
                            if let Some(n) = self.guarded(missing_sql.as_deref().unwrap_or_default(), &refs, no).await? {
                                self.stats.added += n;
                            }
                        }
                        ImportMode::Delete => {
                            let keys: Vec<&Option<String>> = spec.key_columns.iter().map(|&k| &values[k]).collect();
                            if let Some(n) = self.guarded(delete_sql.as_deref().unwrap_or_default(), &keys, no).await? {
                                self.stats.deleted += n;
                            }
                        }
                    }
                }
            }
        }
        self.flush_inserts(&mut batch).await?;
        Ok(())
    }

    fn delete_sql_if_keys(&self) -> Option<String> {
        (!self.spec.key_columns.is_empty()).then(|| self.delete_sql())
    }

    fn missing_sql_if_keys(&self) -> Option<String> {
        (!self.spec.key_columns.is_empty()).then(|| self.insert_if_missing_sql())
    }
}

fn row_error(e: DatabaseError, row_no: u64) -> DatabaseError {
    DatabaseError { message: format!("row {row_no}: {}", e.message), ..e }
}

/// Importa as linhas de `feed` para `spec.schema.spec.table`. Devolve as contagens; em erro (sem "continuar em erro")
/// desfaz tudo e devolve `rolled_back`. Erros de conexão e cancelamento voltam como `Err`.
pub async fn run(s: &PgSession, spec: &ImportSpec, feed: &mut (dyn FnMut() -> Feed + Send), hooks: Hooks<'_>) -> Result<ImportStats, DatabaseError> {
    let table = format!("{}.{}", quote_ident(&spec.schema), quote_ident(spec.create.as_ref().map(|_| spec.table.as_str()).unwrap_or(&spec.table)));
    let mut runner = Runner { s, spec, hooks, stats: ImportStats::default(), last_progress: Instant::now(), row_no: 0, prepared: HashMap::new(), table };
    s.client.batch_execute("BEGIN").await.map_err(sql_err)?;
    let result = async {
        if let Some(cols) = &spec.create {
            let defs: Vec<String> = cols.iter().map(|(n, t)| format!("{} {}", quote_ident(n), t)).collect();
            let sql = format!("CREATE TABLE {} ({})", runner.table, defs.join(", "));
            s.client.batch_execute(&sql).await.map_err(sql_err)?;
            (runner.hooks.log)(format!("[IMP] table {} created", runner.table));
        }
        if spec.mode == ImportMode::Copy {
            let n = s.client.execute(&format!("DELETE FROM {}", runner.table) as &str, &[]).await.map_err(sql_err)?;
            runner.stats.deleted += n;
            (runner.hooks.log)(format!("[IMP] {n} record(s) deleted from the destination before repopulating"));
        }
        runner.run(feed).await
    }
    .await;
    match result {
        Ok(()) => {
            s.client.batch_execute("COMMIT").await.map_err(sql_err)?;
            Ok(runner.stats)
        }
        Err(e) => {
            let _ = s.client.batch_execute("ROLLBACK").await;
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
