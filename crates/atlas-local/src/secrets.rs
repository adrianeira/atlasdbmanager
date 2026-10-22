//! Cofre de credenciais do sistema operacional (SEG-001): Gerenciador de Credenciais no
//! Windows, Keychain no macOS, Secret Service no Linux. Sempre no escopo do usuário atual,
//! sem exigir administrador. Nunca há fallback para texto puro (SEG-012 / TST-SEG-024).

use atlas_core::{DatabaseError, ErrorKind, Secret};
use keyring::{Entry, Error as KeyringError};

const SERVICE: &str = "Atlas DB Manager";

fn vault_err(e: KeyringError) -> DatabaseError {
    let msg = match &e {
        KeyringError::NoStorageAccess(_) | KeyringError::PlatformFailure(_) => {
            format!("the operating system credential vault is not available: {e}")
        }
        _ => format!("credential vault: {e}"),
    };
    DatabaseError::new(ErrorKind::Internal, msg)
}

fn entry(account: &str) -> Result<Entry, DatabaseError> {
    Entry::new(SERVICE, account).map_err(vault_err)
}

pub fn store_password(account: &str, password: &Secret) -> Result<(), DatabaseError> {
    entry(account)?.set_password(password.expose()).map_err(vault_err)
}

/// `Ok(None)` quando não há credencial salva para a conta.
pub fn read_password(account: &str) -> Result<Option<Secret>, DatabaseError> {
    match entry(account)?.get_password() {
        Ok(p) => Ok(Some(Secret::new(p))),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(e) => Err(vault_err(e)),
    }
}

pub fn delete_password(account: &str) -> Result<(), DatabaseError> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(e) => Err(vault_err(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Grava, lê e apaga uma credencial real no cofre do usuário.
    /// Opt-in (ATLAS_TEST_KEYRING=1) para não mexer no cofre em CI.
    #[test]
    fn vault_roundtrip() {
        if std::env::var("ATLAS_TEST_KEYRING").ok().as_deref() != Some("1") {
            eprintln!("ATLAS_TEST_KEYRING não definido: teste pulado");
            return;
        }
        let account = "atlas-test-roundtrip";
        store_password(account, &Secret::new("vault-sentinel-42")).unwrap();
        assert_eq!(read_password(account).unwrap().unwrap().expose(), "vault-sentinel-42");
        delete_password(account).unwrap();
        assert!(read_password(account).unwrap().is_none());
    }
}
