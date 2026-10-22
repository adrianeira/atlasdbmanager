//! Estado e comandos do armazenamento local (`atlas.db` + cofre de credenciais).

use atlas_core::{ConnectionParams, DatabaseError, ErrorKind, Secret};
use atlas_local::{LocalStore, PasswordAction, ProfileRecord};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

/// Banco local aberto na inicialização. Se falhar (ex.: arquivo corrompido), o erro fica
/// guardado e a UI mostra o motivo, sem apagar nada.
pub struct Local {
    store: Mutex<Option<LocalStore>>,
    path: PathBuf,
    open_error: Option<String>,
}

impl Local {
    pub fn open(path: PathBuf) -> Self {
        match LocalStore::open(&path) {
            Ok(store) => Self { store: Mutex::new(Some(store)), path, open_error: None },
            Err(e) => {
                log::error!("falha ao abrir {}: {}", path.display(), e.message);
                Self { store: Mutex::new(None), path, open_error: Some(e.message) }
            }
        }
    }

    pub fn with<T>(&self, f: impl FnOnce(&LocalStore) -> Result<T, DatabaseError>) -> Result<T, DatabaseError> {
        let guard = self.store.lock().map_err(|_| DatabaseError::new(ErrorKind::Internal, "local store lock poisoned"))?;
        match guard.as_ref() {
            Some(store) => f(store),
            None => Err(DatabaseError::new(
                ErrorKind::Internal,
                self.open_error.clone().unwrap_or_else(|| "local store unavailable".into()),
            )),
        }
    }

    /// Completa a senha a partir do cofre quando a UI não enviou uma (perfil com senha salva).
    pub fn resolve_password(&self, mut params: ConnectionParams, profile_id: Option<&str>) -> Result<ConnectionParams, DatabaseError> {
        if params.password.is_empty()
            && let Some(id) = profile_id
            && let Some(saved) = self.with(|s| atlas_local::saved_password(s, id))?
        {
            params.password = saved;
        }
        Ok(params)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalStatus {
    pub path: String,
    pub schema_version: Option<i64>,
    pub error: Option<String>,
}

#[tauri::command]
pub fn local_status(local: State<'_, Local>) -> LocalStatus {
    LocalStatus {
        path: local.path.display().to_string(),
        schema_version: local.with(|s| s.schema_version()).ok(),
        error: local.open_error.clone(),
    }
}

#[tauri::command]
pub fn profiles_list(local: State<'_, Local>) -> Result<Vec<ProfileRecord>, DatabaseError> {
    local.with(|s| s.list_profiles())
}

/// `save_password` falso remove a credencial; verdadeiro com senha vazia mantém a existente.
#[tauri::command]
pub fn profile_save(
    local: State<'_, Local>,
    profile: ProfileRecord,
    password: Option<Secret>,
    save_password: bool,
) -> Result<(), DatabaseError> {
    let action = match (&password, save_password) {
        (_, false) => PasswordAction::Forget,
        (Some(p), true) if !p.is_empty() => PasswordAction::Store(p),
        _ => PasswordAction::Keep,
    };
    local.with(|s| atlas_local::save_profile(s, &profile, action))
}

#[tauri::command]
pub fn profile_delete(local: State<'_, Local>, id: String) -> Result<(), DatabaseError> {
    local.with(|s| atlas_local::delete_profile(s, &id))
}

#[tauri::command]
pub fn pref_get(local: State<'_, Local>, key: String) -> Result<Option<serde_json::Value>, DatabaseError> {
    local.with(|s| s.get_pref(&key))
}

#[tauri::command]
pub fn pref_set(local: State<'_, Local>, key: String, value: serde_json::Value) -> Result<(), DatabaseError> {
    local.with(|s| s.set_pref(&key, &value))
}
