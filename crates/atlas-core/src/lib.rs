//! Domínio do Atlas DB Manager: tipos compartilhados entre provedores e a aplicação.
//! Não depende de UI nem de driver (spec §3.4).

mod error;
mod model;
mod secret;

pub use error::{DatabaseError, ErrorKind};
pub use model::*;
pub use secret::Secret;
