//! Sessões abertas: uma "sessão lógica" por conexão da árvore, com uma conexão
//! física por banco aberto (PostgreSQL conecta sempre em um banco específico).

use atlas_core::{ConnectionParams, DatabaseError, ErrorKind};
use atlas_pg::PgSession;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

struct LogicalSession {
    /// Mantido só em memória enquanto a sessão estiver aberta (inclui a senha).
    params: ConnectionParams,
    by_database: HashMap<String, Arc<PgSession>>,
}

#[derive(Default)]
pub struct Sessions {
    inner: Mutex<HashMap<String, LogicalSession>>,
}

impl Sessions {
    pub async fn open(&self, id: &str, params: ConnectionParams) -> Result<Arc<PgSession>, DatabaseError> {
        let session = Arc::new(atlas_pg::connect(&params, None).await?);
        let db = session.server.current_database.clone();
        let mut map = self.inner.lock().await;
        map.insert(id.to_string(), LogicalSession { params, by_database: HashMap::from([(db, session.clone())]) });
        Ok(session)
    }

    pub async fn close(&self, id: &str) {
        // Ao soltar os `Client`s as conexões físicas são encerradas.
        self.inner.lock().await.remove(id);
    }

    pub async fn close_database(&self, id: &str, database: &str) {
        if let Some(s) = self.inner.lock().await.get_mut(id) {
            s.by_database.remove(database);
        }
    }

    /// Conexão física nova e exclusiva (para transações que não podem misturar com outras abas).
    pub async fn dedicated(&self, id: &str, database: &str) -> Result<PgSession, DatabaseError> {
        let params = {
            let map = self.inner.lock().await;
            let s = map.get(id).ok_or_else(|| DatabaseError::new(ErrorKind::SessionNotFound, "connection is not open"))?;
            s.params.clone()
        };
        atlas_pg::connect(&params, Some(database)).await
    }

    /// Conexão com `database` (o banco inicial quando `None`), abrindo-a se necessário.
    pub async fn get(&self, id: &str, database: Option<&str>) -> Result<Arc<PgSession>, DatabaseError> {
        let params = {
            let map = self.inner.lock().await;
            let s = map.get(id).ok_or_else(|| DatabaseError::new(ErrorKind::SessionNotFound, "connection is not open"))?;
            let db = database.unwrap_or(&s.params.database);
            let db = if db.is_empty() { "postgres" } else { db };
            if let Some(conn) = s.by_database.get(db).filter(|c| !c.is_closed()) {
                return Ok(conn.clone());
            }
            s.params.clone()
        };
        // Conecta fora do lock para não bloquear as outras abas durante o handshake.
        let conn = Arc::new(atlas_pg::connect(&params, database).await?);
        let mut map = self.inner.lock().await;
        let s = map.get_mut(id).ok_or_else(|| DatabaseError::new(ErrorKind::SessionNotFound, "connection is not open"))?;
        s.by_database.insert(conn.server.current_database.clone(), conn.clone());
        Ok(conn)
    }
}
