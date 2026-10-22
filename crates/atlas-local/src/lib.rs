//! Armazenamento local do Atlas: banco `atlas.db` (configurações, nunca dados de usuário)
//! e cofre de credenciais do sistema operacional para as senhas.

pub mod secrets;
mod store;

use atlas_core::{DatabaseError, Secret};
pub use store::{HistoryEntry, HistoryQuery, LocalStore, NewHistory, ProfileRecord, Snippet};

/// O que fazer com a senha ao salvar um perfil.
pub enum PasswordAction<'a> {
    /// Grava (ou substitui) a senha no cofre do sistema.
    Store(&'a Secret),
    /// Mantém a credencial que já existir no cofre.
    Keep,
    /// Remove a credencial do cofre ("Salvar senha" desmarcado).
    Forget,
}

/// Salva o perfil no `atlas.db` e a senha no cofre. O banco local guarda só a referência
/// (`secret_ref` = id do perfil), nunca o valor.
pub fn save_profile(store: &LocalStore, profile: &ProfileRecord, password: PasswordAction) -> Result<(), DatabaseError> {
    let secret_ref = match password {
        PasswordAction::Store(p) => {
            secrets::store_password(&profile.id, p)?;
            Some(profile.id.clone())
        }
        PasswordAction::Keep => store.secret_ref(&profile.id)?,
        PasswordAction::Forget => {
            secrets::delete_password(&profile.id)?;
            None
        }
    };
    store.upsert_profile(profile, secret_ref.as_deref())
}

/// Remove o perfil e a credencial correspondente.
pub fn delete_profile(store: &LocalStore, id: &str) -> Result<(), DatabaseError> {
    secrets::delete_password(id)?;
    store.delete_profile(id)
}

/// Senha salva do perfil, se houver (lida do cofre no momento de conectar).
pub fn saved_password(store: &LocalStore, id: &str) -> Result<Option<Secret>, DatabaseError> {
    match store.secret_ref(id)? {
        Some(account) => secrets::read_password(&account),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TST-SEG-001: perfil salvo com senha sentinela; a senha vai para o cofre e
    /// nenhum byte dela aparece no `atlas.db` nem no WAL. Opt-in: mexe no cofre real.
    #[test]
    fn sentinel_password_only_in_vault() {
        if std::env::var("ATLAS_TEST_KEYRING").ok().as_deref() != Some("1") {
            eprintln!("ATLAS_TEST_KEYRING não definido: teste pulado");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas.db");
        let store = LocalStore::open(&path).unwrap();
        let profile = ProfileRecord {
            id: "atlas-test-sentinel-profile".into(),
            name: "Sentinela".into(),
            provider: "postgresql".into(),
            host: "localhost".into(),
            port: 5432,
            initial_database: "postgres".into(),
            user: "postgres".into(),
            environment: "development".into(),
            options: Default::default(),
            sort_order: 0,
            has_password: false,
        };
        let sentinel = Secret::new("SENTINEL-P4ssw0rd-atlas-7781");
        save_profile(&store, &profile, PasswordAction::Store(&sentinel)).unwrap();
        assert_eq!(saved_password(&store, &profile.id).unwrap().unwrap().expose(), sentinel.expose());
        assert!(store.list_profiles().unwrap()[0].has_password);

        let mut haystack = std::fs::read(&path).unwrap();
        if let Ok(wal) = std::fs::read(path.with_extension("db-wal")) {
            haystack.extend(wal);
        }
        assert!(!String::from_utf8_lossy(&haystack).contains("SENTINEL-P4ssw0rd"), "senha vazou para o arquivo local");

        // "Salvar senha" desmarcado: some do cofre.
        save_profile(&store, &profile, PasswordAction::Forget).unwrap();
        assert!(saved_password(&store, &profile.id).unwrap().is_none());
        delete_profile(&store, &profile.id).unwrap();
    }
}
