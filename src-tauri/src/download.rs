//! Baixa um arquivo de um endereço `http(s)` para a pasta temporária, para o assistente de importação ("Adicionar URL").

use atlas_core::{DatabaseError, ErrorKind};
use serde::Serialize;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tokio::io::AsyncWriteExt;

/// Maior arquivo aceito (a importação lê tudo do disco; acima disso é melhor baixar por fora).
const MAX_BYTES: u64 = 200 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);
/// Pastas de downloads antigos são apagadas na próxima vez que alguém baixa algo.
const KEEP: Duration = Duration::from_secs(24 * 3600);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Downloaded {
    pub path: String,
    pub bytes: u64,
}

fn invalid(msg: impl Into<String>) -> DatabaseError {
    DatabaseError::new(ErrorKind::InvalidInput, msg)
}

/// Nome do arquivo a partir do endereço (sem parâmetros), só com caracteres seguros; `download` se não houver.
pub(crate) fn file_name_from(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let last = path.rsplit('/').next().unwrap_or("");
    let decoded: String = last.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect();
    let trimmed = decoded.trim_matches(['.', '_']);
    if trimmed.is_empty() { "download".into() } else { trimmed.chars().take(80).collect() }
}

fn downloads_root() -> PathBuf {
    std::env::temp_dir().join("atlas-downloads")
}

/// Apaga as pastas de downloads com mais de um dia.
fn purge_old(root: &PathBuf) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|age| age > KEEP);
        if old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

#[tauri::command]
pub async fn download_to_temp(url: String) -> Result<Downloaded, DatabaseError> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(invalid("the address must start with http:// or https://"));
    }
    if url.len() > 2000 || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid("the address is not valid"));
    }
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("AtlasDBManager/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))?;
    let mut resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| DatabaseError::new(ErrorKind::HostUnreachable, format!("cannot download: {}", e.without_url())))?;
    if !resp.status().is_success() {
        return Err(DatabaseError::new(ErrorKind::HostUnreachable, format!("the server answered {}", resp.status())));
    }
    if resp.content_length().is_some_and(|n| n > MAX_BYTES) {
        return Err(invalid(format!("the file is larger than {} MB", MAX_BYTES / 1024 / 1024)));
    }
    let root = downloads_root();
    purge_old(&root);
    let dir = root.join(format!("{:x}", SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
    std::fs::create_dir_all(&dir).map_err(|e| DatabaseError::new(ErrorKind::Internal, format!("cannot create {}: {e}", dir.display())))?;
    let path = dir.join(file_name_from(resp.url().as_str()));
    let mut file = tokio::fs::File::create(&path).await.map_err(|e| DatabaseError::new(ErrorKind::Internal, format!("cannot write {}: {e}", path.display())))?;
    let mut total = 0u64;
    let result: Result<(), DatabaseError> = async {
        while let Some(chunk) = resp.chunk().await.map_err(|e| DatabaseError::new(ErrorKind::HostUnreachable, format!("download interrupted: {}", e.without_url())))? {
            total += chunk.len() as u64;
            if total > MAX_BYTES {
                return Err(invalid(format!("the file is larger than {} MB", MAX_BYTES / 1024 / 1024)));
            }
            file.write_all(&chunk).await.map_err(|e| DatabaseError::new(ErrorKind::Internal, format!("cannot write the file: {e}")))?;
        }
        file.flush().await.map_err(|e| DatabaseError::new(ErrorKind::Internal, e.to_string()))
    }
    .await;
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(Downloaded { path: path.to_string_lossy().into_owned(), bytes: total })
}

#[cfg(test)]
mod tests {
    use super::file_name_from;

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name_from("https://x.com/dados/clientes.csv?token=1#a"), "clientes.csv");
        assert_eq!(file_name_from("https://x.com/a%20b/c d.json"), "c_d.json");
        assert_eq!(file_name_from("https://x.com/"), "download");
        // O que importa: nunca sai da pasta (sem separador de caminho e sem `..` sozinho), seja qual for o endereço.
        let backslash = char::from(92u8);
        let evil_win = format!("https://x.com/a/..{backslash}..{backslash}evil.csv");
        for url in ["https://x.com/..%2F..%2Fx", evil_win.as_str(), "https://x.com/a/b/../../c.txt", "https://x.com/..", "https://x.com/%2e%2e", "https://x.com/C:/Windows/x.csv", "https://x.com/a//"] {
            let n = file_name_from(url);
            assert!(!n.contains('/') && !n.contains(backslash) && !n.contains(':') && n != ".." && n != "." && !n.is_empty(), "{url} -> {n}");
        }
        assert_eq!(file_name_from(&evil_win), "evil.csv");
    }
}
