use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Valor sensível (senha, token). `Debug`/`Display` nunca mostram o conteúdo (SEG-004)
/// e a serialização devolve sempre vazio, para que o segredo não volte à UI nem a logs.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Acesso explícito ao valor, só no ponto em que o driver precisa dele.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_empty() { "Secret(<empty>)" } else { "Secret(<redacted>)" })
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self(String::deserialize(d)?))
    }
}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_serialize_never_leak() {
        let s = Secret::new("sentinel-password-123");
        assert!(!format!("{s:?}").contains("sentinel"));
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"\"");
        assert_eq!(s.expose(), "sentinel-password-123");
    }
}
