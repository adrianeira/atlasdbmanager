use serde::Serialize;

/// Categorias de erro tipadas (spec §4.6). A UI decide a mensagem/ação pela categoria.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    AuthenticationFailed,
    HostUnreachable,
    CertificateRejected,
    PermissionDenied,
    ConnectionLost,
    DatabaseNotFound,
    SqlSyntaxError,
    ConstraintViolation,
    Timeout,
    Canceled,
    UnsupportedFeature,
    InvalidMutationIdentity,
    ConcurrencyConflict,
    InvalidInput,
    SessionNotFound,
    Server,
    Internal,
}

/// Erro devolvido à UI: categoria + código nativo + mensagem já sanitizada (sem senha).
#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct DatabaseError {
    pub kind: ErrorKind,
    /// Código nativo do servidor (ex.: SQLSTATE `28P01`), quando houver.
    pub code: Option<String>,
    pub message: String,
    /// Detalhe técnico opcional (DETAIL/HINT do servidor).
    pub detail: Option<String>,
    /// Posição (1-based, em caracteres) do erro no SQL, quando o servidor informar.
    pub position: Option<u32>,
}

impl DatabaseError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, code: None, message: message.into(), detail: None, position: None }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}
