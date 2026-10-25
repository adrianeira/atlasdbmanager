//! Backup e restauração do SQL Server: `BACKUP DATABASE` e `RESTORE DATABASE`. O arquivo é gravado e lido pelo próprio servidor,
//! no disco dele; a lista de backups vem do histórico do `msdb`.

use super::{MssqlSession, map_err, quote_ident, quote_literal};
use crate::meta::{os, rows_of, s};
use atlas_core::DatabaseError;

/// Um backup completo registrado no histórico do servidor.
#[derive(Debug, Clone)]
pub struct BackupRow {
    pub path: String,
    /// Instante do término, em milissegundos desde 1970.
    pub finished_ms: u64,
    pub bytes: u64,
    pub name: String,
    pub description: String,
}

impl MssqlSession {
    /// A pasta padrão dos backups do servidor.
    pub async fn default_backup_dir(&self) -> Result<String, DatabaseError> {
        let client = self.client("").await?;
        let mut conn = client.lock().await;
        let rows = rows_of(&mut conn, "SELECT CAST(SERVERPROPERTY('InstanceDefaultBackupPath') AS nvarchar(512))", &[]).await?;
        Ok(rows.first().and_then(|r| os(r, 0)).unwrap_or_else(|| "/var/opt/mssql/data".into()))
    }

    /// Grava um backup completo do banco em `path` (no disco do servidor), sobrescrevendo o arquivo se ele existir.
    pub async fn backup(&self, database: &str, path: &str, name: &str, description: &str) -> Result<(), DatabaseError> {
        let mut conn = self.dedicated("master").await?;
        let sql = format!(
            "BACKUP DATABASE {} TO DISK = {} WITH INIT, NAME = {}, DESCRIPTION = {}",
            quote_ident(database),
            quote_literal(path),
            quote_literal(name),
            quote_literal(description)
        );
        conn.simple_query(sql).await.map_err(map_err)?.into_results().await.map_err(map_err)?;
        Ok(())
    }

    /// Os backups completos do banco, do mais recente ao mais antigo.
    pub async fn backup_list(&self, database: &str) -> Result<Vec<BackupRow>, DatabaseError> {
        let client = self.client("master").await?;
        let mut conn = client.lock().await;
        let rows = rows_of(
            &mut conn,
            "SELECT m.physical_device_name, CAST(DATEDIFF_BIG(SECOND, '1970-01-01', b.backup_finish_date) AS bigint), CAST(b.backup_size AS bigint), b.name, b.description \
               FROM msdb.dbo.backupset b JOIN msdb.dbo.backupmediafamily m ON m.media_set_id = b.media_set_id \
              WHERE b.database_name = @P1 AND b.type = 'D' ORDER BY b.backup_finish_date DESC",
            &[&database],
        )
        .await?;
        Ok(rows
            .iter()
            .map(|r| BackupRow {
                path: s(r, 0),
                finished_ms: r.get::<i64, _>(1).unwrap_or(0).max(0) as u64 * 1000,
                bytes: r.get::<i64, _>(2).unwrap_or(0).max(0) as u64,
                name: os(r, 3).unwrap_or_default(),
                description: os(r, 4).unwrap_or_default(),
            })
            .collect())
    }

    /// Troca o conteúdo do banco pelo do backup (`REPLACE`): as conexões do banco são derrubadas e o banco volta a aceitar todos.
    pub async fn restore(&self, database: &str, path: &str) -> Result<(), DatabaseError> {
        self.close_database(database).await;
        let mut conn = self.dedicated("master").await?;
        let db = quote_ident(database);
        let sql = format!("ALTER DATABASE {db} SET SINGLE_USER WITH ROLLBACK IMMEDIATE; RESTORE DATABASE {db} FROM DISK = {} WITH REPLACE; ALTER DATABASE {db} SET MULTI_USER", quote_literal(path));
        conn.simple_query(sql).await.map_err(map_err)?.into_results().await.map_err(map_err)?;
        Ok(())
    }
}
