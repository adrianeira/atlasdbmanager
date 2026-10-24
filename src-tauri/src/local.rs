//! Estado e comandos do armazenamento local (`atlas.db` + cofre de credenciais).

use atlas_core::{ConnectionParams, DatabaseError, ErrorKind, Secret};
use atlas_local::{HistoryEntry, HistoryQuery, LocalStore, NewHistory, PasswordAction, ProfileRecord, Snippet};
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

    /// Pasta dos dados do Atlas (onde ficam o `atlas.db` e o arquivo de chaves SSH conhecidas).
    pub fn data_dir(&self) -> PathBuf {
        self.path.parent().map(PathBuf::from).unwrap_or_default()
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
        // Senha do servidor SSH salva no cofre (conta `<id>:ssh`).
        if let (Some(ssh), Some(id)) = (params.ssh.as_mut(), profile_id)
            && ssh.password.is_empty()
            && let Some(saved) = atlas_local::secrets::read_password(&format!("{id}:ssh"))?
        {
            ssh.password = saved;
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

/// Arquivo de conexões: só o que identifica o servidor; as senhas nunca saem do cofre.
#[derive(serde::Serialize, serde::Deserialize)]
struct ConnectionsFile {
    format: String,
    version: u32,
    connections: Vec<ProfileRecord>,
}

const CONNECTIONS_FORMAT: &str = "atlas-connections";

/// Grava as conexões em `path` (JSON), sem senha.
#[tauri::command]
pub fn connections_export(path: String, profiles: Vec<ProfileRecord>) -> Result<usize, DatabaseError> {
    let connections: Vec<ProfileRecord> = profiles.into_iter().map(|p| ProfileRecord { has_password: false, ..p }).collect();
    let count = connections.len();
    let file = ConnectionsFile { format: CONNECTIONS_FORMAT.into(), version: 1, connections };
    let json = serde_json::to_string_pretty(&file).map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))?;
    std::fs::write(&path, json).map_err(|e| DatabaseError::new(ErrorKind::Internal, format!("cannot write {path}: {e}")))?;
    Ok(count)
}

/// Lê as conexões de um arquivo exportado; quem importa decide ids e nomes novos.
#[tauri::command]
pub fn connections_import(path: String) -> Result<Vec<ProfileRecord>, DatabaseError> {
    let text = std::fs::read_to_string(&path).map_err(|e| DatabaseError::new(ErrorKind::InvalidInput, format!("cannot read {path}: {e}")))?;
    let file: ConnectionsFile = serde_json::from_str(&text)
        .map_err(|e| DatabaseError::new(ErrorKind::InvalidInput, format!("not an Atlas connections file: {e}")))?;
    if file.format != CONNECTIONS_FORMAT {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "not an Atlas connections file"));
    }
    Ok(file.connections.into_iter().map(|p| ProfileRecord { has_password: false, ..p }).collect())
}

/// Senha (ou frase da chave) do servidor SSH do perfil: guardada no cofre; `save` falso a remove.
#[tauri::command]
pub fn ssh_password_save(id: String, password: Option<Secret>, save: bool) -> Result<(), DatabaseError> {
    let account = format!("{id}:ssh");
    match (&password, save) {
        (_, false) => atlas_local::secrets::delete_password(&account),
        (Some(p), true) if !p.is_empty() => atlas_local::secrets::store_password(&account, p),
        _ => Ok(()),
    }
}

/// Maior arquivo que dá para carregar numa célula (a célula guarda o conteúdo inteiro em hexadecimal).
const MAX_BLOB_FILE: u64 = 64 * 1024 * 1024;

/// Conteúdo de um arquivo no formato `\x...` (hexadecimal) do `bytea`, para gravar numa célula.
#[tauri::command]
pub fn file_read_hex(path: String) -> Result<String, DatabaseError> {
    let meta = std::fs::metadata(&path).map_err(|e| DatabaseError::new(ErrorKind::InvalidInput, format!("cannot read {path}: {e}")))?;
    if meta.len() > MAX_BLOB_FILE {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, format!("the file is larger than {} MB", MAX_BLOB_FILE / 1024 / 1024)));
    }
    let bytes = std::fs::read(&path).map_err(|e| DatabaseError::new(ErrorKind::InvalidInput, format!("cannot read {path}: {e}")))?;
    let mut out = String::with_capacity(bytes.len() * 2 + 2);
    out.push_str("\\x");
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    Ok(out)
}

/// Grava em `path` os bytes de um valor `\x...` (hexadecimal); devolve quantos bytes.
#[tauri::command]
pub fn file_write_hex(path: String, hex: String) -> Result<usize, DatabaseError> {
    let digits = hex.strip_prefix("\\x").unwrap_or(&hex);
    if !digits.len().is_multiple_of(2) || !digits.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "the value is not hexadecimal binary data"));
    }
    let bytes: Vec<u8> = (0..digits.len()).step_by(2).map(|i| u8::from_str_radix(&digits[i..i + 2], 16).unwrap_or(0)).collect();
    std::fs::write(&path, &bytes).map_err(|e| DatabaseError::new(ErrorKind::Internal, format!("cannot write {path}: {e}")))?;
    Ok(bytes.len())
}

#[tauri::command]
pub fn profile_delete(local: State<'_, Local>, id: String) -> Result<(), DatabaseError> {
    local.with(|s| atlas_local::delete_profile(s, &id))
}

#[tauri::command]
pub fn history_add(local: State<'_, Local>, entry: NewHistory) -> Result<String, DatabaseError> {
    local.with(|s| s.add_history(&entry))
}

#[tauri::command]
pub fn history_list(local: State<'_, Local>, query: HistoryQuery) -> Result<Vec<HistoryEntry>, DatabaseError> {
    local.with(|s| s.list_history(&query))
}

#[tauri::command]
pub fn history_favorite(local: State<'_, Local>, id: String, favorite: bool) -> Result<(), DatabaseError> {
    local.with(|s| s.set_history_favorite(&id, favorite))
}

#[tauri::command]
pub fn history_delete(local: State<'_, Local>, id: String) -> Result<(), DatabaseError> {
    local.with(|s| s.delete_history(&id))
}

/// Esvazia o histórico; com `keep_favorites` os favoritos ficam. Devolve quantas consultas foram apagadas.
#[tauri::command]
pub fn history_clear(local: State<'_, Local>, keep_favorites: bool) -> Result<u64, DatabaseError> {
    local.with(|s| s.clear_history(keep_favorites))
}

/// Snippets que acompanham o app: gravados na primeira listagem com a lista vazia, uma única vez.
const STARTER_SNIPPETS: &[(&str, &str)] = &[
    ("select", "SELECT ${1:*}\nFROM ${2:tabela}\nWHERE ${3:condicao};"),
    ("select count", "SELECT count(*) FROM ${1:tabela};"),
    ("insert", "INSERT INTO ${1:tabela} (${2:colunas})\nVALUES (${3:valores});"),
    ("update", "UPDATE ${1:tabela}\n   SET ${2:coluna} = ${3:valor}\n WHERE ${4:condicao};"),
    ("delete", "DELETE FROM ${1:tabela}\n WHERE ${2:condicao};"),
    ("create table", "CREATE TABLE ${1:nome} (\n  id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,\n  ${2:coluna} ${3:text} NOT NULL\n);"),
    ("cte (with)", "WITH ${1:nome} AS (\n  SELECT ${2:*} FROM ${3:tabela}\n)\nSELECT * FROM ${1:nome};"),
    ("explain analyze", "EXPLAIN (ANALYZE, BUFFERS)\n${1:SELECT 1};"),
];

#[tauri::command]
pub fn snippets_list(local: State<'_, Local>) -> Result<Vec<Snippet>, DatabaseError> {
    local.with(|s| {
        s.seed_snippets(STARTER_SNIPPETS)?;
        s.list_snippets()
    })
}

/// Sem `id` cria um snippet novo; com `id` atualiza. Devolve o id.
#[tauri::command]
pub fn snippet_save(local: State<'_, Local>, id: Option<String>, name: String, body: String) -> Result<String, DatabaseError> {
    local.with(|s| s.save_snippet(id.as_deref(), &name, &body))
}

#[tauri::command]
pub fn snippet_delete(local: State<'_, Local>, id: String) -> Result<(), DatabaseError> {
    local.with(|s| s.delete_snippet(&id))
}

#[tauri::command]
pub fn pref_get(local: State<'_, Local>, key: String) -> Result<Option<serde_json::Value>, DatabaseError> {
    local.with(|s| s.get_pref(&key))
}

#[tauri::command]
pub fn pref_set(local: State<'_, Local>, key: String, value: serde_json::Value) -> Result<(), DatabaseError> {
    local.with(|s| s.set_pref(&key, &value))
}
