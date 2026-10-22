//! Banco local `atlas.db` (spec §9 e §35): migrações versionadas, backup antes de migrar,
//! verificação de integridade e repositórios de perfis e preferências.

use atlas_core::{DatabaseError, ErrorKind};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Migrações em ordem. Nunca editar uma migração já publicada: criar a próxima.
const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("migrations/v1.sql"))];

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";

fn local_err(e: impl std::fmt::Display) -> DatabaseError {
    DatabaseError::new(ErrorKind::Internal, format!("local store: {e}"))
}

pub struct LocalStore {
    conn: Connection,
    path: PathBuf,
}

/// Perfil de conexão como a UI o vê. Não tem campo de senha: `has_password` só indica
/// que existe uma credencial no cofre do sistema para este perfil.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRecord {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub host: String,
    pub port: u16,
    pub initial_database: String,
    pub user: String,
    pub environment: String,
    /// Opções livres (sslMode, color, starred...). Chaves de senha são rejeitadas.
    #[serde(default)]
    pub options: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub has_password: bool,
}

/// Nenhuma opção pode carregar segredo (spec §9.2: proibido `password` em options_json).
fn reject_secret_keys(options: &serde_json::Map<String, serde_json::Value>) -> Result<(), DatabaseError> {
    for key in options.keys() {
        let k = key.to_lowercase();
        if k.contains("pass") || k.contains("secret") || k.contains("token") || k.contains("pwd") {
            return Err(DatabaseError::new(ErrorKind::InvalidInput, format!("option \"{key}\" is not allowed in a profile")));
        }
    }
    Ok(())
}

impl LocalStore {
    /// Abre (ou cria) o banco local, confere integridade e aplica migrações pendentes.
    pub fn open(path: &Path) -> Result<Self, DatabaseError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(local_err)?;
        }
        let existed = path.exists();
        let conn = Connection::open(path).map_err(local_err)?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;")
            .map_err(local_err)?;
        if existed {
            let check: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0)).map_err(local_err)?;
            if check != "ok" {
                // Não apaga nada automaticamente (spec §35.3): o arquivo fica para recuperação.
                return Err(DatabaseError::new(ErrorKind::Internal, format!("local store is corrupted ({}): {check}", path.display())));
            }
        }
        let mut store = Self { conn, path: path.to_path_buf() };
        store.migrate()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn schema_version(&self) -> Result<i64, DatabaseError> {
        self.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(local_err)
    }

    fn migrate(&mut self) -> Result<(), DatabaseError> {
        let current = self.schema_version()?;
        let latest = MIGRATIONS.last().map(|m| m.0).unwrap_or(0);
        if current > latest {
            // Versão do app mais antiga que o banco: recusa em vez de corromper (spec §46.3).
            return Err(DatabaseError::new(
                ErrorKind::Internal,
                format!("local store schema v{current} is newer than this Atlas version (v{latest})"),
            ));
        }
        for (version, sql) in MIGRATIONS.iter().filter(|m| m.0 > current) {
            if current > 0 {
                self.backup_before(*version)?;
            }
            let checksum = format!("{:x}", Sha256::digest(sql.as_bytes()));
            let tx = self.conn.transaction().map_err(local_err)?;
            tx.execute_batch(sql).map_err(local_err)?;
            tx.execute(
                &format!("INSERT INTO schema_migrations(version, applied_utc, checksum) VALUES (?1, {NOW}, ?2)"),
                params![version, checksum],
            )
            .map_err(local_err)?;
            tx.execute_batch(&format!("PRAGMA user_version = {version}")).map_err(local_err)?;
            tx.commit().map_err(local_err)?;
            log::info!("atlas.db migrado para v{version}");
        }
        Ok(())
    }

    /// Cópia consistente (inclui WAL) antes de migrar: `atlas.db.bak-v<N>`.
    fn backup_before(&self, version: i64) -> Result<(), DatabaseError> {
        let target = self.path.with_extension(format!("db.bak-v{}", version - 1));
        let _ = std::fs::remove_file(&target);
        self.conn
            .execute("VACUUM INTO ?1", params![target.to_string_lossy()])
            .map_err(local_err)?;
        Ok(())
    }

    /* ---------- Perfis de conexão ---------- */

    pub fn list_profiles(&self) -> Result<Vec<ProfileRecord>, DatabaseError> {
        let mut st = self
            .conn
            .prepare(
                "SELECT id, display_name, provider_id, COALESCE(host,''), COALESCE(port,0), COALESCE(database_name,''), \
                        COALESCE(username,''), environment, options_json, sort_order, secret_ref IS NOT NULL \
                   FROM connection_profiles ORDER BY sort_order, display_name",
            )
            .map_err(local_err)?;
        let rows = st
            .query_map([], |r| {
                let options: String = r.get(8)?;
                Ok(ProfileRecord {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    provider: r.get(2)?,
                    host: r.get(3)?,
                    port: r.get::<_, i64>(4)? as u16,
                    initial_database: r.get(5)?,
                    user: r.get(6)?,
                    environment: r.get(7)?,
                    options: serde_json::from_str(&options).unwrap_or_default(),
                    sort_order: r.get(9)?,
                    has_password: r.get(10)?,
                })
            })
            .map_err(local_err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(local_err)
    }

    /// Insere ou atualiza. `secret_ref` só marca que existe credencial no cofre (nunca o valor).
    pub fn upsert_profile(&self, p: &ProfileRecord, secret_ref: Option<&str>) -> Result<(), DatabaseError> {
        reject_secret_keys(&p.options)?;
        let options = serde_json::to_string(&p.options).map_err(local_err)?;
        self.conn
            .execute(
                &format!(
                    "INSERT INTO connection_profiles \
                       (id, display_name, provider_id, host, port, database_name, username, secret_ref, environment, options_json, sort_order, created_utc, updated_utc) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, {NOW}, {NOW}) \
                     ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, provider_id = excluded.provider_id, \
                       host = excluded.host, port = excluded.port, database_name = excluded.database_name, \
                       username = excluded.username, secret_ref = excluded.secret_ref, environment = excluded.environment, \
                       options_json = excluded.options_json, sort_order = excluded.sort_order, updated_utc = {NOW}"
                ),
                params![p.id, p.name, p.provider, p.host, p.port, p.initial_database, p.user, secret_ref, p.environment, options, p.sort_order],
            )
            .map_err(local_err)?;
        Ok(())
    }

    pub fn secret_ref(&self, id: &str) -> Result<Option<String>, DatabaseError> {
        self.conn
            .query_row("SELECT secret_ref FROM connection_profiles WHERE id = ?1", params![id], |r| r.get(0))
            .optional()
            .map_err(local_err)
            .map(Option::flatten)
    }

    pub fn delete_profile(&self, id: &str) -> Result<(), DatabaseError> {
        self.conn.execute("DELETE FROM connection_profiles WHERE id = ?1", params![id]).map_err(local_err)?;
        Ok(())
    }

    /* ---------- Preferências ---------- */

    pub fn get_pref(&self, key: &str) -> Result<Option<serde_json::Value>, DatabaseError> {
        let raw: Option<String> = self
            .conn
            .query_row("SELECT value_json FROM preferences WHERE key = ?1", params![key], |r| r.get(0))
            .optional()
            .map_err(local_err)?;
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
    }

    pub fn set_pref(&self, key: &str, value: &serde_json::Value) -> Result<(), DatabaseError> {
        let json = serde_json::to_string(value).map_err(local_err)?;
        self.conn
            .execute(
                &format!("INSERT INTO preferences(key, value_json, updated_utc) VALUES (?1, ?2, {NOW}) \
                          ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_utc = {NOW}"),
                params![key, json],
            )
            .map_err(local_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str) -> ProfileRecord {
        ProfileRecord {
            id: id.into(),
            name: "PostgreSQL 9.6".into(),
            provider: "postgresql".into(),
            host: "localhost".into(),
            port: 5432,
            initial_database: "postgres".into(),
            user: "postgres".into(),
            environment: "development".into(),
            options: serde_json::Map::from_iter([("color".to_string(), serde_json::json!("green"))]),
            sort_order: 0,
            has_password: false,
        }
    }

    #[test]
    fn migrates_fresh_db_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas.db");
        let store = LocalStore::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 1);
        let checksum: String = store.conn.query_row("SELECT checksum FROM schema_migrations WHERE version = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(checksum.len(), 64);
        drop(store);
        // Reabrir não reaplica a migração.
        let store = LocalStore::open(&path).unwrap();
        let n: i64 = store.conn.query_row("SELECT count(*) FROM schema_migrations", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn profiles_roundtrip_without_password() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalStore::open(&dir.path().join("atlas.db")).unwrap();
        store.upsert_profile(&profile("p1"), Some("p1")).unwrap();
        let list = store.list_profiles().unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].has_password);
        assert_eq!(list[0].options.get("color"), Some(&serde_json::json!("green")));

        let mut p = profile("p1");
        p.name = "Renomeada".into();
        store.upsert_profile(&p, None).unwrap();
        let list = store.list_profiles().unwrap();
        assert_eq!(list[0].name, "Renomeada");
        assert!(!list[0].has_password);

        store.delete_profile("p1").unwrap();
        assert!(store.list_profiles().unwrap().is_empty());
    }

    #[test]
    fn rejects_secret_like_options() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalStore::open(&dir.path().join("atlas.db")).unwrap();
        let mut p = profile("p2");
        p.options.insert("Password".into(), serde_json::json!("x"));
        assert!(store.upsert_profile(&p, None).is_err());
    }

    #[test]
    fn refuses_schema_newer_than_app() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas.db");
        let store = LocalStore::open(&path).unwrap();
        store.conn.execute_batch("PRAGMA user_version = 99").unwrap();
        drop(store);
        assert!(LocalStore::open(&path).is_err());
    }

    #[test]
    fn preferences_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalStore::open(&dir.path().join("atlas.db")).unwrap();
        assert!(store.get_pref("layout").unwrap().is_none());
        store.set_pref("layout", &serde_json::json!({"navWidth": 300})).unwrap();
        assert_eq!(store.get_pref("layout").unwrap(), Some(serde_json::json!({"navWidth": 300})));
    }
}
