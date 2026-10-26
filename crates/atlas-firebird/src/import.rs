//! Gravação das linhas de uma importação numa tabela do Firebird, em uma transação, nos seis modos do assistente.
//!
//! As linhas chegam prontas (texto já convertido, `None` = NULL) e vão como parâmetros `?`: nenhum valor entra no texto do SQL e o
//! servidor converte para o tipo da coluna. O Firebird não tem `INSERT` com várias linhas, então cada linha é um comando. Com
//! "continuar em erro" cada comando roda sob um `SAVEPOINT`, então uma linha ruim é registrada e pulada sem perder o resto; sem isso
//! o primeiro erro desfaz tudo.

use super::{Conn, map_err, quote_ident};
use atlas_core::{DatabaseError, ErrorKind};
use rsfbclient::{Execute, Queryable, Row, SqlType};
use serde::Deserialize;
use std::time::{Duration, Instant};

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
    /// Tipo da coluna (`integer`, `boolean`, `numeric(10,2)`...).
    pub declared_type: String,
}

#[derive(Debug, Clone)]
pub struct ImportSpec {
    pub table: String,
    /// Tabela nova: colunas (nome, tipo do assistente) criadas antes de importar.
    pub create: Option<Vec<(String, String)>>,
    /// Colunas de destino na ordem dos valores de cada linha.
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
    pub log: &'a mut dyn FnMut(String),
    pub progress: &'a mut dyn FnMut(&ImportStats),
    pub cancelled: &'a dyn Fn() -> bool,
}

/// O tipo do Firebird para um tipo do PostgreSQL vindo da inferência do assistente.
pub fn declared_type(pg_type: &str, boolean_ok: bool) -> String {
    let t = pg_type.trim().to_lowercase();
    let base = t.split('(').next().unwrap_or(&t).trim().to_string();
    let args = t.find('(').map(|i| t[i..].to_string()).unwrap_or_default();
    match base.as_str() {
        "smallint" | "int2" => "SMALLINT".into(),
        "integer" | "int" | "int4" | "serial" => "INTEGER".into(),
        "bigint" | "int8" | "bigserial" => "BIGINT".into(),
        "numeric" | "decimal" => {
            // O Firebird guarda no máximo 18 dígitos em NUMERIC (38 no 4); sem tamanho vira um número com casas.
            let digits = args.trim_matches(|c| c == '(' || c == ')');
            let mut parts = digits.split(',').map(|p| p.trim().parse::<i32>().ok());
            match (parts.next().flatten(), parts.next().flatten()) {
                (Some(p), Some(s)) => format!("NUMERIC({}, {})", p.min(18), s.min(p.min(18))),
                (Some(p), None) => format!("NUMERIC({}, 0)", p.min(18)),
                _ => "NUMERIC(18, 6)".into(),
            }
        }
        "real" | "float4" => "FLOAT".into(),
        "double precision" | "float8" | "float" | "money" => "DOUBLE PRECISION".into(),
        "boolean" | "bool" => if boolean_ok { "BOOLEAN".into() } else { "SMALLINT".into() },
        "date" => "DATE".into(),
        "time" | "time without time zone" | "time with time zone" | "timetz" => "TIME".into(),
        b if b.starts_with("timestamp") => "TIMESTAMP".into(),
        "varchar" | "character varying" | "char" | "character" | "bpchar" => {
            let n = args.trim_matches(|c| c == '(' || c == ')').trim().parse::<u32>().unwrap_or(255);
            if n == 0 || n > 8191 { "BLOB SUB_TYPE TEXT".into() } else if base.starts_with("char") && base != "character varying" || base == "bpchar" { format!("CHAR({n})") } else { format!("VARCHAR({n})") }
        }
        "bytea" => "BLOB".into(),
        "uuid" => "VARCHAR(36)".into(),
        _ => "BLOB SUB_TYPE TEXT".into(),
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
    /// Coluna booleana: o assistente entrega `true`/`false`; sem o tipo `BOOLEAN` (Firebird 2.5) vira 1/0.
    bools: Vec<bool>,
    boolean_ok: bool,
}

fn params(values: &[Option<String>]) -> Vec<SqlType> {
    values.iter().map(|v| v.as_ref().map_or(SqlType::Null, |s| SqlType::Text(s.clone()))).collect()
}

impl Runner<'_, '_> {
    fn column_list(&self) -> String {
        self.spec.columns.iter().map(|c| quote_ident(&c.name)).collect::<Vec<_>>().join(", ")
    }

    fn insert_sql(&self) -> String {
        let marks = vec!["?"; self.spec.columns.len()].join(", ");
        format!("INSERT INTO {} ({}) VALUES ({marks})", self.table, self.column_list())
    }

    fn key_condition(&self) -> String {
        self.spec.key_columns.iter().map(|&k| format!("{} = ?", quote_ident(&self.spec.columns[k].name))).collect::<Vec<_>>().join(" AND ")
    }

    /// `UPDATE` pelas chaves: os parâmetros são os valores não-chave (em ordem) e depois as chaves.
    fn update_sql(&self) -> Option<String> {
        let sets: Vec<String> = (0..self.spec.columns.len()).filter(|i| !self.spec.key_columns.contains(i)).map(|i| format!("{} = ?", quote_ident(&self.spec.columns[i].name))).collect();
        if sets.is_empty() {
            return None;
        }
        Some(format!("UPDATE {} SET {} WHERE {}", self.table, sets.join(", "), self.key_condition()))
    }

    fn update_params(&self, values: &[Option<String>]) -> Vec<Option<String>> {
        let mut out: Vec<Option<String>> = (0..values.len()).filter(|i| !self.spec.key_columns.contains(i)).map(|i| values[i].clone()).collect();
        out.extend(self.spec.key_columns.iter().map(|&k| values[k].clone()));
        out
    }

    fn key_params(&self, values: &[Option<String>]) -> Vec<Option<String>> {
        self.spec.key_columns.iter().map(|&k| values[k].clone()).collect()
    }

    /// O valor como o servidor o recebe: o booleano de texto vira `true`/`false` (ou 1/0 sem o tipo `BOOLEAN`).
    fn stored(&self, col: usize, v: &Option<String>) -> Option<String> {
        match v {
            Some(s) if self.bools[col] => {
                let on = s == "true" || s == "t" || s == "1";
                Some(if self.boolean_ok { if on { "true" } else { "false" } } else if on { "1" } else { "0" }.to_string())
            }
            other => other.clone(),
        }
    }

    fn execute(&mut self, sql: &str, values: &[Option<String>]) -> Result<u64, DatabaseError> {
        self.conn.execute(sql, params(values)).map(|n| n as u64).map_err(map_err)
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

    /// Um comando por linha sob SAVEPOINT (quando se continua em erro). `Some(n)` = linhas afetadas; `None` = a linha foi pulada.
    fn guarded(&mut self, sql: &str, values: &[Option<String>], row_no: u64) -> Result<Option<u64>, DatabaseError> {
        if !self.spec.continue_on_error {
            return self.execute(sql, values).map(Some).map_err(|e| row_error(e, row_no));
        }
        self.conn.execute("SAVEPOINT atlas_row", ()).map_err(map_err)?;
        match self.execute(sql, values) {
            Ok(n) => {
                self.conn.execute("RELEASE SAVEPOINT atlas_row", ()).map_err(map_err)?;
                Ok(Some(n))
            }
            Err(e) => {
                self.conn.execute("ROLLBACK TO SAVEPOINT atlas_row", ()).map_err(map_err)?;
                let _ = self.conn.execute("RELEASE SAVEPOINT atlas_row", ());
                self.log_error(format!("row {row_no}: {}", e.message));
                Ok(None)
            }
        }
    }

    /// A chave já existe na tabela?
    fn exists(&mut self, key_sql: &str, keys: &[Option<String>]) -> Result<bool, DatabaseError> {
        let rows: Vec<Row> = self.conn.query(key_sql, params(keys)).map_err(map_err)?;
        Ok(!rows.is_empty())
    }

    fn run(&mut self, feed: &mut dyn FnMut() -> Feed) -> Result<(), DatabaseError> {
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
        let insert_one = self.insert_sql();
        let delete_sql = (!spec.key_columns.is_empty()).then(|| format!("DELETE FROM {} WHERE {}", self.table, self.key_condition()));
        let exists_sql = (!spec.key_columns.is_empty()).then(|| format!("SELECT 1 FROM {} WHERE {}", self.table, self.key_condition()));

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
                            if let Some(n) = self.guarded(&insert_one, &values, no)? {
                                self.stats.added += n;
                            }
                        }
                        ImportMode::Update => {
                            let p = self.update_params(&values);
                            if let Some(n) = self.guarded(update_sql.as_deref().unwrap_or_default(), &p, no)? {
                                self.stats.updated += n;
                            }
                        }
                        ImportMode::AppendUpdate => {
                            let p = self.update_params(&values);
                            if let Some(n) = self.guarded(update_sql.as_deref().unwrap_or_default(), &p, no)? {
                                if n > 0 {
                                    self.stats.updated += n;
                                } else if let Some(a) = self.guarded(&insert_one, &values, no)? {
                                    self.stats.added += a;
                                }
                            }
                        }
                        ImportMode::AppendNoUpdate => {
                            let keys = self.key_params(&values);
                            if !self.exists(exists_sql.as_deref().unwrap_or_default(), &keys)? {
                                if let Some(a) = self.guarded(&insert_one, &values, no)? {
                                    self.stats.added += a;
                                }
                            }
                        }
                        ImportMode::Delete => {
                            let keys = self.key_params(&values);
                            if let Some(n) = self.guarded(delete_sql.as_deref().unwrap_or_default(), &keys, no)? {
                                self.stats.deleted += n;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn row_error(e: DatabaseError, row_no: u64) -> DatabaseError {
    DatabaseError { message: format!("row {row_no}: {}", e.message), ..e }
}

/// Importa as linhas de `feed` para a tabela. Devolve as contagens; em erro (sem "continuar em erro") desfaz tudo e devolve
/// `rolled_back`. Cancelamento volta como `Err`.
pub(crate) fn run(conn: &mut Conn, boolean_ok: bool, spec: &ImportSpec, feed: &mut dyn FnMut() -> Feed, hooks: Hooks<'_>) -> Result<ImportStats, DatabaseError> {
    let table = quote_ident(&spec.table);
    let bools: Vec<bool> = spec.columns.iter().map(|c| c.declared_type.to_ascii_lowercase().contains("bool")).collect();
    conn.begin_transaction().map_err(map_err)?;
    let mut runner = Runner { conn, spec, hooks, stats: ImportStats::default(), last_progress: Instant::now(), row_no: 0, table, bools, boolean_ok };
    let result = (|| {
        if let Some(cols) = &spec.create {
            let defs: Vec<String> = cols.iter().map(|(n, t)| format!("{} {}", quote_ident(n), declared_type(t, boolean_ok))).collect();
            let create = format!("CREATE TABLE {} ({})", runner.table, defs.join(", "));
            runner.execute(&create, &[])?;
            // A tabela nova só vale para o resto da transação depois de gravada: o Firebird não deixa usar uma tabela criada na
            // mesma transação em comandos que a preparam antes do `COMMIT`.
            runner.conn.commit_retaining_ddl()?;
            (runner.hooks.log)(format!("[IMP] table {} created", runner.table));
        }
        if spec.mode == ImportMode::Copy {
            let n = runner.execute(&format!("DELETE FROM {}", runner.table), &[])?;
            runner.stats.deleted += n;
            (runner.hooks.log)(format!("[IMP] {n} record(s) deleted from the destination before repopulating"));
        }
        runner.run(feed)
    })();
    match result {
        Ok(()) => {
            runner.conn.commit().map_err(map_err)?;
            Ok(runner.stats)
        }
        Err(e) => {
            let _ = runner.conn.rollback();
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

/// `COMMIT` + nova transação no meio da importação (para o DDL valer para os comandos seguintes).
trait CommitRetaining {
    fn commit_retaining_ddl(&mut self) -> Result<(), DatabaseError>;
}

impl CommitRetaining for Conn {
    fn commit_retaining_ddl(&mut self) -> Result<(), DatabaseError> {
        self.commit().map_err(map_err)?;
        self.begin_transaction().map_err(map_err)
    }
}
