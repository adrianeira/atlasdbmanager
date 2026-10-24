//! O cliente do PostgreSQL com registro: cada comando que passa por aqui vai para o History Log com hora, tempo e erro.
//! Os métodos que não estão aqui (`prepare`, `copy_out`, `is_closed`, `cancel_token`...) saem direto do `Client`.

use crate::history;
use std::ops::Deref;
use std::time::Instant;
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, Error, Row, SimpleQueryMessage, ToStatement};

/// O texto do comando, quando o que se passa para `query` é um texto (um `Statement` já preparado não tem).
pub trait LogText {
    fn log_text(&self) -> Option<&str>;
}

impl LogText for str {
    fn log_text(&self) -> Option<&str> {
        Some(self)
    }
}

impl LogText for String {
    fn log_text(&self) -> Option<&str> {
        Some(self)
    }
}

impl LogText for tokio_postgres::Statement {
    fn log_text(&self) -> Option<&str> {
        None
    }
}

pub struct LoggedClient {
    inner: Client,
    server: String,
    pid: i32,
}

impl Deref for LoggedClient {
    type Target = Client;
    fn deref(&self) -> &Client {
        &self.inner
    }
}

impl LoggedClient {
    pub fn new(inner: Client, server: String, pid: i32) -> Self {
        LoggedClient { inner, server, pid }
    }

    /// Registra um comando já executado (usado onde o tempo total só se conhece no fim, como nos resultados em fluxo).
    pub fn log(&self, sql: &str, secs: f64, error: Option<&Error>) {
        history::record(&self.server, self.pid, sql, secs, error.map(|e| history::format_driver_error(e, sql)));
    }

    /// Registra um comando cujo erro já veio convertido (resultados em fluxo, `COPY`).
    pub fn log_command(&self, sql: &str, secs: f64, error: Option<&atlas_core::DatabaseError>) {
        history::record(&self.server, self.pid, sql, secs, error.map(|e| history::format_atlas_error(e, sql)));
    }

    fn done<T>(&self, sql: Option<&str>, started: Instant, result: &Result<T, Error>) {
        if let Some(sql) = sql {
            self.log(sql, started.elapsed().as_secs_f64(), result.as_ref().err());
        }
    }

    pub async fn batch_execute(&self, sql: &str) -> Result<(), Error> {
        let started = Instant::now();
        let r = self.inner.batch_execute(sql).await;
        self.done(Some(sql), started, &r);
        r
    }

    pub async fn query<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<Vec<Row>, Error>
    where
        T: ?Sized + ToStatement + LogText,
    {
        let started = Instant::now();
        let r = self.inner.query(statement, params).await;
        self.done(statement.log_text(), started, &r);
        r
    }

    pub async fn query_one<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<Row, Error>
    where
        T: ?Sized + ToStatement + LogText,
    {
        let started = Instant::now();
        let r = self.inner.query_one(statement, params).await;
        self.done(statement.log_text(), started, &r);
        r
    }

    pub async fn query_opt<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<Option<Row>, Error>
    where
        T: ?Sized + ToStatement + LogText,
    {
        let started = Instant::now();
        let r = self.inner.query_opt(statement, params).await;
        self.done(statement.log_text(), started, &r);
        r
    }

    pub async fn execute<T>(&self, statement: &T, params: &[&(dyn ToSql + Sync)]) -> Result<u64, Error>
    where
        T: ?Sized + ToStatement + LogText,
    {
        let started = Instant::now();
        let r = self.inner.execute(statement, params).await;
        self.done(statement.log_text(), started, &r);
        r
    }

    pub async fn simple_query(&self, sql: &str) -> Result<Vec<SimpleQueryMessage>, Error> {
        let started = Instant::now();
        let r = self.inner.simple_query(sql).await;
        self.done(Some(sql), started, &r);
        r
    }
}
