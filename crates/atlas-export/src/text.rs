//! Ajudas de texto comuns aos formatos.

/// Tipos numéricos do PostgreSQL cujo texto (quando válido) pode ir sem aspas em JSON/SQL.
pub fn is_numeric_type(type_name: Option<&str>) -> bool {
    matches!(type_name, Some("int2" | "int4" | "int8" | "oid" | "float4" | "float8" | "numeric"))
}

pub fn is_bool_type(type_name: Option<&str>) -> bool {
    matches!(type_name, Some("bool"))
}

pub fn is_json_type(type_name: Option<&str>) -> bool {
    matches!(type_name, Some("json" | "jsonb"))
}

/// Gramática de número JSON: `-?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?`. Fica de fora `NaN`, `Infinity`,
/// zeros à esquerda e qualquer valor com símbolo — esses vão como texto.
pub fn is_json_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if b.get(i) == Some(&b'-') {
        i += 1;
    }
    match b.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            while matches!(b.get(i), Some(b'0'..=b'9')) {
                i += 1;
            }
        }
        _ => return false,
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while matches!(b.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let start = i;
        while matches!(b.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == b.len()
}

/// Identificador PostgreSQL entre aspas (aspas internas duplicadas).
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
