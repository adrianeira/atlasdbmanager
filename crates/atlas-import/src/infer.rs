//! Tipo de coluna para tabelas novas, deduzido de uma amostra dos valores do arquivo.

use crate::convert::{ConvertOptions, TargetKind, convert, parse_bool};

/// Tipo PostgreSQL que serve a todos os valores da amostra (`text` quando nada mais serve).
pub fn infer_type<'a>(samples: impl IntoIterator<Item = Option<&'a str>>, o: &ConvertOptions) -> String {
    let values: Vec<&str> = samples.into_iter().flatten().map(str::trim).filter(|v| !v.is_empty()).collect();
    if values.is_empty() {
        return "text".into();
    }
    let all = |kind: TargetKind| values.iter().all(|v| convert(v, kind, o).is_ok_and(|r| r.is_some()));

    // Lógico só quando são palavras (t/f/sim/não...): uma coluna de 0 e 1 é número.
    if values.iter().all(|v| parse_bool(v).is_some() && !v.chars().all(|c| c.is_ascii_digit())) {
        return "boolean".into();
    }
    if all(TargetKind::Int) {
        let fits_i32 = values.iter().all(|v| convert(v, TargetKind::Int, o).ok().flatten().is_some_and(|n| n.parse::<i32>().is_ok()));
        if fits_i32 {
            return "integer".into();
        }
        if values.iter().all(|v| convert(v, TargetKind::Int, o).ok().flatten().is_some_and(|n| n.parse::<i64>().is_ok())) {
            return "bigint".into();
        }
        return "numeric".into();
    }
    if all(TargetKind::Numeric) {
        return "numeric".into();
    }
    // Datas: sem hora em nenhum valor = date; senão timestamp.
    if all(TargetKind::Timestamp) {
        let has_time = values.iter().any(|v| v.len() > 10 && (v.contains(':') || v.contains('T')));
        return if has_time { "timestamp" } else { "date" }.into();
    }
    if values.iter().all(|v| (v.starts_with('{') || v.starts_with('[')) && serde_json::from_str::<serde_json::Value>(v).is_ok()) {
        return "jsonb".into();
    }
    "text".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn infer(values: &[&str]) -> String {
        infer_type(values.iter().map(|v| if v.is_empty() { None } else { Some(*v) }), &ConvertOptions::default())
    }

    #[test]
    fn picks_the_narrowest_type_that_fits_every_value() {
        assert_eq!(infer(&["1", "2", "", "300"]), "integer");
        assert_eq!(infer(&["1", "3000000000"]), "bigint");
        assert_eq!(infer(&["1", "99999999999999999999"]), "numeric");
        assert_eq!(infer(&["1", "2.5", "-3"]), "numeric");
        assert_eq!(infer(&["t", "f", "t"]), "boolean");
        assert_eq!(infer(&["sim", "não"]), "boolean");
        assert_eq!(infer(&["0", "1", "1"]), "integer");
        assert_eq!(infer(&["22/10/2026", "01/01/2025"]), "date");
        assert_eq!(infer(&["2026-01-15", "2026-02-20"]), "date");
        assert_eq!(infer(&["2026-01-15 07:30:00", "2026-02-20 08:00:00"]), "timestamp");
        assert_eq!(infer(&["22/10/2026 10:00", "22/10/2026"]), "timestamp");
        assert_eq!(infer(&["{\"a\": 1}", "[1, 2]"]), "jsonb");
        assert_eq!(infer(&["Ana", "1", "x"]), "text");
        assert_eq!(infer(&["", ""]), "text");
        // Um valor que não cabe derruba para o tipo mais amplo.
        assert_eq!(infer(&["10", "abc"]), "text");
    }
}
