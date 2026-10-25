//! Backup e restauração com as ferramentas do próprio PostgreSQL (`pg_dump`, `pg_restore` e `psql`).
//!
//! Os backups ficam na pasta de dados do app, por conexão, banco e schema (`backups/<conexão>/<banco>/<schema>/`), cada um
//! num arquivo `.dump` (formato personalizado) com um `.json` ao lado (comentário e data), e são listados na categoria
//! Backup. As ferramentas não vêm embutidas: são procuradas na pasta informada, no `PATH` e nos locais de instalação usuais,
//! e a de maior versão é usada (o `pg_dump` recusa servidores mais novos que ele). A senha vai pela variável de ambiente
//! `PGPASSWORD`, nunca na linha de comando. O arquivo é escrito em `<destino>.part` e só vira o destino ao terminar bem.
//! O cancelamento encerra o processo; a saída das ferramentas chega à UI pelo evento `backup-log`.

use crate::export::{io_error, part_path, validate_path, PartFile};
use crate::sessions::Sessions;
use atlas_core::{ConnectionParams, DatabaseError, ErrorKind, SslMode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::Notify;

/// Backups e restaurações em andamento (para o botão Cancelar).
#[derive(Default)]
pub struct Backups {
    active: Mutex<HashMap<String, Arc<Notify>>>,
}

struct Registration<'a>(&'a Backups, String);

impl Drop for Registration<'_> {
    fn drop(&mut self) {
        self.0.active.lock().unwrap().remove(&self.1);
    }
}

impl Backups {
    fn register(&self, id: &str) -> (Arc<Notify>, Registration<'_>) {
        let cancel = Arc::new(Notify::new());
        self.active.lock().unwrap().insert(id.to_string(), cancel.clone());
        (cancel, Registration(self, id.to_string()))
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Content {
    All,
    Structure,
    Data,
}

impl Content {
    fn flag(self) -> Option<&'static str> {
        match self {
            Content::All => None,
            Content::Structure => Some("-s"),
            Content::Data => Some("-a"),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRequest {
    pub job_id: String,
    pub session_id: String,
    pub database: String,
    /// Só este schema; sem ele o backup é do banco inteiro.
    #[serde(default)]
    pub schema: Option<String>,
    pub tools_dir: Option<String>,
    /// Nome do backup, sem a extensão.
    pub file_name: String,
    #[serde(default)]
    pub comment: String,
    /// Objetos que ficam de fora (`schema.nome` de tabelas, views e sequências desmarcadas).
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Pasta de destino; sem ela vale a pasta padrão do app.
    #[serde(default)]
    pub folder: Option<String>,
}

/// Backup guardado, como a lista da categoria Backup o mostra.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    pub name: String,
    pub path: String,
    /// Arquivo de registro do backup na pasta do app (comentário e, se o backup está fora dela, o caminho dele).
    pub meta_path: String,
    pub bytes: u64,
    /// Instante da gravação, em milissegundos desde 1970.
    pub modified_ms: u64,
    pub comment: String,
    /// `schema` ou `database` (backup do banco inteiro).
    pub scope: String,
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct Sidecar {
    #[serde(default)]
    pub(crate) comment: String,
    /// Backup gravado fora da pasta do app.
    #[serde(default)]
    pub(crate) path: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreRequest {
    pub job_id: String,
    pub session_id: String,
    /// Banco de destino (ignorado ao extrair o SQL).
    pub database: String,
    pub tools_dir: Option<String>,
    /// Arquivo de backup.
    pub path: String,
    pub content: Content,
    #[serde(default)]
    pub clean: bool,
    #[serde(default)]
    pub no_owner: bool,
    #[serde(default)]
    pub no_privileges: bool,
    /// Tudo ou nada: desfaz a restauração inteira se algum comando falhar.
    #[serde(default)]
    pub single_transaction: bool,
    /// Em vez de restaurar, grava o SQL do backup neste arquivo (só para backups que não são script).
    pub extract_to: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupResult {
    pub bytes: u64,
    pub elapsed_ms: u64,
    pub path: String,
    pub tool: String,
    pub name: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LogLine<'a> {
    pub(crate) job_id: &'a str,
    pub(crate) line: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsInfo {
    /// Pasta das ferramentas escolhidas (a de maior versão), `None` se não achou.
    pub dir: Option<String>,
    pub version: Option<u32>,
    /// Outras pastas com as ferramentas, para o usuário escolher.
    pub others: Vec<ToolDir>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ToolDir {
    pub dir: String,
    pub version: u32,
}

fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

/// Pastas onde costumam estar as ferramentas do PostgreSQL.
fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    let mut scan = |root: &str, suffix: &str| {
        if let Ok(rd) = std::fs::read_dir(root) {
            for e in rd.flatten() {
                dirs.push(e.path().join(suffix));
            }
        }
    };
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(pf) = std::env::var_os(var) {
                scan(&PathBuf::from(pf).join("PostgreSQL").to_string_lossy(), "bin");
            }
        }
    } else {
        scan("/usr/lib/postgresql", "bin");
        scan("/opt/homebrew/opt", "bin");
        scan("/usr/local/opt", "bin");
        scan("/Library/PostgreSQL", "bin");
        dirs.push(PathBuf::from("/Applications/Postgres.app/Contents/Versions/latest/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    dirs
}

async fn tool_version(dir: &Path) -> Option<u32> {
    let pg_dump = dir.join(exe("pg_dump"));
    if !pg_dump.is_file() || !dir.join(exe("pg_restore")).is_file() {
        return None;
    }
    let mut cmd = Command::new(&pg_dump);
    cmd.arg("--version").stdin(Stdio::null());
    hide_window(&mut cmd);
    let out = cmd.output().await.ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // "pg_dump (PostgreSQL) 18.1" / "pg_dump (PostgreSQL) 9.6.24"
    let v = text.split_whitespace().last()?;
    v.split('.').next()?.parse().ok()
}

#[cfg(windows)]
fn hide_window(cmd: &mut Command) {
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
fn hide_window(_: &mut Command) {}

async fn find_tools(custom: Option<&str>) -> Vec<ToolDir> {
    let dirs: Vec<PathBuf> = match custom.map(str::trim).filter(|s| !s.is_empty()) {
        Some(d) => vec![PathBuf::from(d)],
        None => candidate_dirs(),
    };
    let mut found: Vec<ToolDir> = Vec::new();
    for d in dirs {
        let key = d.to_string_lossy().to_lowercase();
        if found.iter().any(|f| f.dir.to_lowercase() == key) {
            continue;
        }
        if let Some(version) = tool_version(&d).await {
            found.push(ToolDir { dir: d.to_string_lossy().into_owned(), version });
        }
    }
    found.sort_by_key(|a| std::cmp::Reverse(a.version));
    found
}

/// Procura as ferramentas (`pg_dump` e `pg_restore`); sem `custom`, nos locais usuais.
#[tauri::command]
pub async fn pg_tools_locate(custom_dir: Option<String>) -> Result<ToolsInfo, DatabaseError> {
    let mut all = find_tools(custom_dir.as_deref()).await;
    if all.is_empty() {
        return Ok(ToolsInfo { dir: None, version: None, others: Vec::new() });
    }
    let best = all.remove(0);
    Ok(ToolsInfo { dir: Some(best.dir), version: Some(best.version), others: all })
}

async fn resolve_dir(custom: Option<&str>) -> Result<PathBuf, DatabaseError> {
    let all = find_tools(custom).await;
    all.into_iter().next().map(|t| PathBuf::from(t.dir)).ok_or_else(|| {
        DatabaseError::new(ErrorKind::UnsupportedFeature, "pg_dump/pg_restore não encontrados: informe a pasta das ferramentas do PostgreSQL")
    })
}

fn ssl_env(mode: SslMode) -> &'static str {
    match mode {
        SslMode::Disable => "disable",
        SslMode::Prefer => "prefer",
        SslMode::Require => "require",
        SslMode::VerifyCa => "verify-ca",
        SslMode::VerifyFull => "verify-full",
    }
}

fn connection_args(p: &ConnectionParams, database: &str) -> Vec<String> {
    vec!["-h".into(), p.host.clone(), "-p".into(), p.port.to_string(), "-U".into(), p.user.clone(), "-w".into(), "-d".into(), database.to_string()]
}

/// Lê um fluxo linha a linha, repassa à UI e guarda as últimas (para a mensagem de erro).
fn pump<R: AsyncRead + Unpin + Send + 'static>(
    app: AppHandle,
    job: String,
    stream: R,
    tail: Arc<Mutex<Vec<String>>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = app.emit("backup-log", LogLine { job_id: &job, line: &line });
            let mut t = tail.lock().unwrap();
            t.push(line);
            if t.len() > 40 {
                t.remove(0);
            }
        }
    })
}

/// Executa a ferramenta até o fim ou até o cancelamento; erro com as últimas linhas se sair com falha.
async fn run_tool(
    app: &AppHandle,
    job_id: &str,
    cancel: &Notify,
    program: &Path,
    args: &[String],
    params: &ConnectionParams,
) -> Result<(), DatabaseError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .env("PGPASSWORD", params.password.expose())
        .env("PGSSLMODE", ssl_env(params.ssl_mode))
        .env("PGCONNECT_TIMEOUT", params.connect_timeout_secs.to_string())
        // Mensagens em inglês: o texto é mostrado como veio da ferramenta.
        .env("LC_MESSAGES", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    hide_window(&mut cmd);
    let mut child = cmd.spawn().map_err(io_error)?;
    let tail = Arc::new(Mutex::new(Vec::<String>::new()));
    let readers = [
        pump(app.clone(), job_id.to_string(), child.stdout.take().unwrap(), tail.clone()),
        pump(app.clone(), job_id.to_string(), child.stderr.take().unwrap(), tail.clone()),
    ];
    let status = tokio::select! {
        s = child.wait() => s.map_err(io_error)?,
        _ = cancel.notified() => {
            let _ = child.kill().await;
            return Err(DatabaseError::new(ErrorKind::Canceled, "canceled"));
        }
    };
    for r in readers {
        let _ = r.await;
    }
    if status.success() {
        return Ok(());
    }
    let tail = tail.lock().unwrap();
    let errors: Vec<&String> = tail.iter().filter(|l| l.to_lowercase().contains("error") || l.contains("pg_")).collect();
    let shown = if errors.is_empty() { tail.iter().rev().take(5).rev().cloned().collect::<Vec<_>>() } else { errors.iter().rev().take(5).rev().map(|s| (*s).clone()).collect() };
    Err(DatabaseError::new(ErrorKind::Server, shown.join("\n")))
}

/// Trecho de caminho seguro (sem separadores nem `..`) para nomes de conexão, banco, schema e arquivo.
pub(crate) fn safe_segment(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') { c } else { '_' }).collect();
    let t = t.trim().trim_matches('.').to_string();
    if t.is_empty() { "_".into() } else { t }
}

fn store_root(app: &AppHandle) -> Result<PathBuf, DatabaseError> {
    Ok(app.path().app_local_data_dir().map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))?.join("backups"))
}

/// Pasta dos backups do schema; os do banco inteiro ficam numa pasta própria (`schema` ausente).
pub(crate) fn store_dir(app: &AppHandle, session_id: &str, database: &str, schema: Option<&str>) -> Result<PathBuf, DatabaseError> {
    let scope = schema.map(safe_segment).unwrap_or_else(|| "__database__".into());
    Ok(store_root(app)?.join(safe_segment(session_id)).join(safe_segment(database)).join(scope))
}

#[tauri::command]
pub async fn pg_backup(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    backups: State<'_, Backups>,
    req: BackupRequest,
) -> Result<BackupResult, DatabaseError> {
    let started = Instant::now();
    let params = sessions.params(&req.session_id).await?;
    let dir = resolve_dir(req.tools_dir.as_deref()).await?;
    let store = store_dir(&app, &req.session_id, &req.database, req.schema.as_deref())?;
    std::fs::create_dir_all(&store).map_err(io_error)?;
    let folder = match req.folder.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        Some(f) => {
            let p = PathBuf::from(f);
            if !p.is_absolute() || !p.is_dir() {
                return Err(DatabaseError::new(ErrorKind::InvalidInput, "a pasta do backup precisa existir"));
            }
            p
        }
        None => store.clone(),
    };
    let name = safe_segment(&req.file_name);
    let dest = folder.join(format!("{name}.dump"));
    let meta = store.join(format!("{name}.json"));
    if dest.exists() || meta.exists() {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, format!("já existe um backup chamado {name}")));
    }
    let (cancel, _registered) = backups.register(&req.job_id);

    let mut part = PartFile::new(part_path(&dest));
    let mut args = connection_args(&params, &req.database);
    args.extend(["-v".into(), "-Fc".into(), "-f".into(), part.path.to_string_lossy().into_owned()]);
    if let Some(schema) = &req.schema {
        args.extend(["-n".into(), schema.clone()]);
    }
    for t in &req.exclude {
        args.extend(["-T".into(), t.clone()]);
    }
    run_tool(&app, &req.job_id, &cancel, &dir.join(exe("pg_dump")), &args, &params).await?;
    std::fs::rename(&part.path, &dest).map_err(io_error)?;
    part.keep = true;
    let external = (folder != store).then(|| dest.to_string_lossy().into_owned());
    let sidecar = serde_json::to_string(&Sidecar { comment: req.comment, path: external }).unwrap_or_default();
    let _ = std::fs::write(&meta, sidecar);
    Ok(BackupResult {
        bytes: std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0),
        elapsed_ms: started.elapsed().as_millis() as u64,
        path: dest.to_string_lossy().into_owned(),
        tool: "pg_dump".into(),
        name,
    })
}

/// Backups guardados do schema e do banco inteiro, do mais recente para o mais antigo.
#[tauri::command]
pub async fn pg_backup_list(app: AppHandle, session_id: String, database: String, schema: String) -> Result<Vec<BackupEntry>, DatabaseError> {
    let mut out = Vec::new();
    for (scope, folder) in [("schema", store_dir(&app, &session_id, &database, Some(&schema))?), ("database", store_dir(&app, &session_id, &database, None)?)] {
        let Ok(rd) = std::fs::read_dir(&folder) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let meta_path = p.with_extension("json");
            let sidecar = |path: &Path| std::fs::read_to_string(path).ok().and_then(|j| serde_json::from_str::<Sidecar>(&j).ok()).unwrap_or_default();
            // Backup da pasta do app (o `.dump`) ou um que está fora dela (o `.json` aponta para o arquivo).
            let (dump, comment) = match p.extension().and_then(|x| x.to_str()) {
                Some("dump") => (p.clone(), sidecar(&meta_path).comment),
                Some("json") => {
                    let sc = sidecar(&p);
                    match sc.path {
                        Some(ext) => (PathBuf::from(ext), sc.comment),
                        None => continue,
                    }
                }
                _ => continue,
            };
            let Ok(meta) = std::fs::metadata(&dump) else { continue };
            let modified_ms = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0);
            out.push(BackupEntry {
                name: dump.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                path: dump.to_string_lossy().into_owned(),
                meta_path: meta_path.to_string_lossy().into_owned(),
                bytes: meta.len(),
                modified_ms,
                comment,
                scope: scope.into(),
            });
        }
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.modified_ms));
    Ok(out)
}

/// Exclui um backup: o arquivo (na pasta do app ou no destino registrado em `meta_path`) e o registro dele.
#[tauri::command]
pub async fn pg_backup_delete(app: AppHandle, path: String, meta_path: String) -> Result<(), DatabaseError> {
    let root = store_root(&app)?.canonicalize().map_err(io_error)?;
    let inside = |p: &Path| p.canonicalize().ok().is_some_and(|c| c.starts_with(&root));
    let (dump, meta) = (PathBuf::from(&path), PathBuf::from(&meta_path));
    let registered = std::fs::read_to_string(&meta).ok().and_then(|j| serde_json::from_str::<Sidecar>(&j).ok()).and_then(|s| s.path);
    let valid = dump.extension().and_then(|x| x.to_str()) == Some("dump")
        && (inside(&dump) || registered.as_deref().is_some_and(|r| Path::new(r) == dump))
        && (inside(&meta) || !meta.exists());
    if !valid {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "arquivo que não é de um backup registrado"));
    }
    match std::fs::remove_file(&dump) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io_error(e)),
        _ => {}
    }
    let _ = std::fs::remove_file(&meta);
    Ok(())
}

/// O que há dentro do arquivo: `pg_restore` lê o personalizado e o tar; o resto é um script SQL (`psql`).
fn is_archive(path: &Path) -> bool {
    use std::io::Read;
    if path.is_dir() {
        return true;
    }
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut head = vec![0u8; 512];
    let n = f.read(&mut head).unwrap_or(0);
    head.truncate(n);
    head.starts_with(b"PGDMP") || (n >= 262 && &head[257..262] == b"ustar")
}

#[tauri::command]
pub async fn pg_restore(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    backups: State<'_, Backups>,
    req: RestoreRequest,
) -> Result<BackupResult, DatabaseError> {
    let started = Instant::now();
    let source = PathBuf::from(&req.path);
    if !source.exists() {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "o arquivo de backup não existe"));
    }
    let params = sessions.params(&req.session_id).await?;
    let dir = resolve_dir(req.tools_dir.as_deref()).await?;
    let (cancel, _registered) = backups.register(&req.job_id);
    let archive = is_archive(&source);
    let src = source.to_string_lossy().into_owned();

    if let Some(out) = &req.extract_to {
        if !archive {
            return Err(DatabaseError::new(ErrorKind::InvalidInput, "este arquivo já é um script SQL"));
        }
        let dest = validate_path(out)?;
        let mut part = PartFile::new(part_path(&dest));
        let mut args: Vec<String> = vec!["-f".into(), part.path.to_string_lossy().into_owned()];
        if let Some(f) = req.content.flag() {
            args.push(f.into());
        }
        if req.no_owner {
            args.push("-O".into());
        }
        if req.no_privileges {
            args.push("-x".into());
        }
        if req.clean {
            args.extend(["-c".into(), "--if-exists".into()]);
        }
        args.push(src);
        run_tool(&app, &req.job_id, &cancel, &dir.join(exe("pg_restore")), &args, &params).await?;
        std::fs::rename(&part.path, &dest).map_err(io_error)?;
        part.keep = true;
        return Ok(BackupResult {
            bytes: std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0),
            elapsed_ms: started.elapsed().as_millis() as u64,
            path: dest.to_string_lossy().into_owned(),
            tool: "pg_restore".into(),
            name: String::new(),
        });
    }

    let mut args = connection_args(&params, &req.database);
    let tool = if archive {
        args.push("-v".into());
        if let Some(f) = req.content.flag() {
            args.push(f.into());
        }
        if req.clean {
            args.extend(["-c".into(), "--if-exists".into()]);
        }
        if req.no_owner {
            args.push("-O".into());
        }
        if req.no_privileges {
            args.push("-x".into());
        }
        if req.single_transaction {
            args.push("-1".into());
        }
        args.push(src);
        "pg_restore"
    } else {
        // Script SQL: o conteúdo (estrutura/dados) já está decidido no arquivo.
        args.extend(["-q".into(), "-v".into(), "ON_ERROR_STOP=1".into(), "-f".into(), src]);
        if req.single_transaction {
            args.push("-1".into());
        }
        "psql"
    };
    run_tool(&app, &req.job_id, &cancel, &dir.join(exe(tool)), &args, &params).await?;
    Ok(BackupResult { bytes: std::fs::metadata(&source).map(|m| m.len()).unwrap_or(0), elapsed_ms: started.elapsed().as_millis() as u64, path: req.path, tool: tool.into(), name: String::new() })
}

#[tauri::command]
pub async fn pg_backup_cancel(backups: State<'_, Backups>, job_id: String) -> Result<(), DatabaseError> {
    if let Some(n) = backups.active.lock().unwrap().get(&job_id) {
        n.notify_one();
    }
    Ok(())
}
