use atlas_core::{DatabaseError, ErrorKind};
use tokio_postgres::error::ErrorPosition;

/// Converte erros do driver em erros tipados do Atlas (spec §4.6).
/// Mensagens vêm do servidor ou do driver; nunca contêm a senha.
pub fn map_error(err: &tokio_postgres::Error) -> DatabaseError {
    if let Some(db) = err.as_db_error() {
        let code = db.code().code().to_string();
        let kind = match code.as_str() {
            "28P01" | "28000" => ErrorKind::AuthenticationFailed,
            "3D000" => ErrorKind::DatabaseNotFound,
            "42501" => ErrorKind::PermissionDenied,
            "57014" => ErrorKind::Canceled,
            c if c.starts_with("42") => ErrorKind::SqlSyntaxError,
            c if c.starts_with("23") => ErrorKind::ConstraintViolation,
            c if c.starts_with("40") => ErrorKind::ConcurrencyConflict,
            c if c.starts_with("08") || c.starts_with("57P") => ErrorKind::ConnectionLost,
            _ => ErrorKind::Server,
        };
        let detail = match (db.detail(), db.hint()) {
            (Some(d), Some(h)) => Some(format!("{d}\nHINT: {h}")),
            (Some(d), None) => Some(d.to_string()),
            (None, Some(h)) => Some(format!("HINT: {h}")),
            (None, None) => None,
        };
        let position = match db.position() {
            Some(ErrorPosition::Original(p)) => Some(*p),
            _ => None,
        };
        return DatabaseError { kind, code: Some(code), message: db.message().to_string(), detail, position };
    }

    // Erros sem resposta do servidor: classificação pela descrição do driver.
    let text = full_chain(err);
    let lower = text.to_lowercase();
    let kind = if err.is_closed() {
        ErrorKind::ConnectionLost
    } else if lower.contains("password missing") || lower.contains("authentication") {
        ErrorKind::AuthenticationFailed
    } else if lower.contains("timeout") || lower.contains("timed out") {
        ErrorKind::Timeout
    } else if lower.contains("certificate") || lower.contains("tls") || lower.contains("ssl") {
        ErrorKind::CertificateRejected
    } else if lower.contains("connect") || lower.contains("refused") || lower.contains("resolve") || lower.contains("host") {
        ErrorKind::HostUnreachable
    } else {
        ErrorKind::Internal
    };
    DatabaseError::new(kind, text)
}

/// Mensagem com as causas encadeadas (ex.: "error connecting to server: Connection refused").
fn full_chain(err: &dyn std::error::Error) -> String {
    let mut out = err.to_string();
    let mut cur = err.source();
    while let Some(src) = cur {
        let s = src.to_string();
        if !out.contains(&s) {
            out.push_str(": ");
            out.push_str(&s);
        }
        cur = src.source();
    }
    out
}
