//! Definição de views do SQLite para o designer.

use atlas_core::{DatabaseError, ErrorKind, ViewDetail};
use rusqlite::Connection;

/// O `SELECT` de um `CREATE VIEW nome [(colunas)] AS SELECT ...`: o que vem depois do primeiro `AS` fora de aspas e parênteses.
pub(crate) fn select_of(create_sql: &str) -> Option<&str> {
    let b = create_sql.as_bytes();
    let (mut i, mut depth) = (0usize, 0i32);
    while i < b.len() {
        match b[i] {
            q @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
            }
            b'[' => {
                while i < b.len() && b[i] != b']' {
                    i += 1;
                }
            }
            b'(' => depth += 1,
            b')' => depth -= 1,
            c if depth == 0 && (c == b'A' || c == b'a') && create_sql.is_char_boundary(i) => {
                let before_ok = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
                let after = b.get(i + 2).copied();
                if before_ok && b.get(i + 1).is_some_and(|n| *n == b'S' || *n == b's') && after.is_none_or(|a| !(a.is_ascii_alphanumeric() || a == b'_')) {
                    return Some(create_sql[i + 2..].trim().trim_end_matches(';').trim_end());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

pub(crate) fn view_detail_on(conn: &Connection, view: &str) -> Result<ViewDetail, DatabaseError> {
    let sql: Option<String> = conn.query_row("SELECT sql FROM sqlite_master WHERE type = 'view' AND name = ?1", [view], |r| r.get(0)).ok();
    let Some(sql) = sql else {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, format!("view \"{view}\" does not exist")));
    };
    let definition = select_of(&sql).ok_or_else(|| DatabaseError::new(ErrorKind::Internal, "could not read the view definition"))?;
    Ok(ViewDetail { definition: definition.to_string(), owner: None, comment: None, check_option: None, security_barrier: false, rules: Vec::new(), populated: true })
}

#[cfg(test)]
mod tests {
    use super::select_of;

    #[test]
    fn finds_the_select_after_the_first_top_level_as() {
        assert_eq!(select_of("CREATE VIEW v AS SELECT * FROM t;"), Some("SELECT * FROM t"));
        assert_eq!(select_of("CREATE VIEW \"as\" (a, b) AS\n SELECT 1 AS a, 2 AS b"), Some("SELECT 1 AS a, 2 AS b"));
        assert_eq!(select_of("CREATE VIEW basis AS SELECT x FROM y"), Some("SELECT x FROM y"));
        assert_eq!(select_of("CREATE VIEW v"), None);
    }
}
