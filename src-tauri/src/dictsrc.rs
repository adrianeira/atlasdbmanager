//! De onde o Data Dictionary lê os metadados: cada banco devolve o mesmo formato (tabela, view e função) para o mesmo documento.

use crate::mssql::MssqlSessions;
use crate::mysql::MysqlSessions;
use crate::sessions::Sessions;
use crate::sqlite::SqliteSessions;
use atlas_core::{DatabaseError, ErrorKind, TableDetail};
use atlas_mssql::MssqlSession;
use atlas_mysql::MysqlSession;
use atlas_pg::PgSession;
use atlas_sqlite::SqliteSession;
use std::sync::Arc;

/// Os metadados de uma função ou procedimento.
pub struct FunctionDoc {
    pub arguments: String,
    pub returns: Option<String>,
    pub language: String,
    pub comment: Option<String>,
    pub body: String,
}

pub enum DictSource {
    Pg(Box<PgSession>),
    Sqlite(Arc<SqliteSession>),
    Mssql(Arc<MssqlSession>),
    Mysql(Arc<MysqlSession>),
}

fn kv(k: &str, v: impl Into<String>) -> (String, String) {
    (k.to_string(), v.into())
}

impl DictSource {
    /// A fonte da conexão `conn_id` (pelo prefixo do id) para o banco `database`.
    pub async fn open(conn_id: &str, database: &str, pg: &Sessions, sqlite: &SqliteSessions, mssql: &MssqlSessions, mysql: &MysqlSessions) -> Result<Self, DatabaseError> {
        Ok(if conn_id.starts_with("sqlite_") {
            Self::Sqlite(sqlite.session(conn_id)?)
        } else if conn_id.starts_with("mssql_") {
            Self::Mssql(mssql.session(conn_id)?)
        } else if conn_id.starts_with("mysql_") {
            Self::Mysql(mysql.session(conn_id)?)
        } else {
            Self::Pg(Box::new(pg.dedicated(conn_id, database).await?))
        })
    }

    /// Os campos do painel "Database Information".
    pub async fn database_info(&self, database: &str) -> Result<Vec<(String, String)>, DatabaseError> {
        Ok(match self {
            Self::Pg(s) => {
                let d = atlas_pg::database_detail(s, database).await?;
                vec![
                    kv("Owner", d.owner),
                    kv("Tablespace", d.tablespace),
                    kv("Encoding", d.encoding),
                    kv("Collation Order", d.collate),
                    kv("Character Type", d.ctype),
                    kv("Connection Limit", if d.connection_limit < 0 { "N/A".to_string() } else { d.connection_limit.to_string() }),
                    kv("Comment", d.comment.unwrap_or_else(|| "--".into())),
                ]
            }
            Self::Mysql(s) => {
                let d = s.database_detail(database).await?;
                vec![kv("Encoding", d.encoding), kv("Collation Order", d.collate)]
            }
            Self::Mssql(s) => s.database_info(database).await?,
            Self::Sqlite(s) => {
                let snap = s.monitor_snapshot()?;
                let pick = |list: &[(String, String)], key: &str| list.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()).unwrap_or_default();
                vec![
                    kv("File", pick(&snap.status, "File")),
                    kv("Size (bytes)", pick(&snap.status, "Size (bytes)")),
                    kv("SQLite version", pick(&snap.status, "SQLite version")),
                    kv("Encoding", pick(&snap.variables, "encoding")),
                    kv("Page Size", pick(&snap.variables, "page_size")),
                ]
            }
        })
    }

    /// Os campos do painel "Schema Information".
    pub async fn schema_info(&self, schema: &str) -> Result<Vec<(String, String)>, DatabaseError> {
        Ok(match self {
            Self::Pg(s) => {
                let d = atlas_pg::schema_detail(s, schema).await?;
                vec![kv("Owner", d.owner), kv("Comment", d.comment.unwrap_or_else(|| "--".into()))]
            }
            // Fora do PostgreSQL o schema não tem dono nem comentário próprios para mostrar.
            _ => vec![kv("Name", schema)],
        })
    }

    pub async fn table(&self, database: &str, schema: &str, name: &str) -> Result<TableDetail, DatabaseError> {
        match self {
            Self::Pg(s) => atlas_pg::table_detail(s, schema, name).await,
            Self::Sqlite(s) => s.table_detail(name),
            Self::Mssql(s) => s.table_detail(database, schema, name).await,
            Self::Mysql(s) => s.table_detail(database, name).await,
        }
    }

    /// Comentário e definição (`SELECT`) de uma view.
    pub async fn view(&self, database: &str, schema: &str, name: &str) -> Result<(Option<String>, String), DatabaseError> {
        let d = match self {
            Self::Pg(s) => atlas_pg::view_detail(s, schema, name).await?,
            Self::Sqlite(s) => s.view_detail(name)?,
            Self::Mssql(s) => s.view_detail(database, schema, name).await?,
            Self::Mysql(s) => s.view_detail(database, name).await?,
        };
        Ok((d.comment, d.definition))
    }

    /// Uma função ou procedimento; `identity` só vale no PostgreSQL (sobrecargas).
    pub async fn function(&self, database: &str, schema: &str, name: &str, identity: &str) -> Result<FunctionDoc, DatabaseError> {
        match self {
            Self::Pg(s) => {
                let d = atlas_pg::function_detail(s, schema, name, identity).await?;
                Ok(FunctionDoc { arguments: d.arguments, returns: d.returns, language: d.language, comment: d.comment, body: d.body })
            }
            Self::Mssql(s) => {
                let (arguments, returns) = s.routine_info(database, schema, name).await?;
                let body = s.routine_definition(database, schema, name).await?;
                Ok(FunctionDoc { arguments, returns, language: "T-SQL".into(), comment: None, body })
            }
            Self::Mysql(s) => {
                let (arguments, returns, comment) = s.routine_info(database, name).await?;
                let body = s.routine_definition(database, name).await?;
                Ok(FunctionDoc { arguments, returns, language: "SQL".into(), comment, body })
            }
            Self::Sqlite(_) => Err(DatabaseError::new(ErrorKind::UnsupportedFeature, "SQLite has no stored functions")),
        }
    }
}
