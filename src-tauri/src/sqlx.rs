//! Tradutor de expressões `CHECK` e de `SELECT` simples entre PostgreSQL, SQL Server, MySQL e SQLite.
//!
//! Só atravessa o que tem equivalente seguro: nomes (aspas de cada produto), textos, números, operadores de comparação e lógicos,
//! `IN`, `BETWEEN`, `LIKE`, `IS NULL`, `CASE`, as funções mais comuns e, no `SELECT`, `JOIN`, `WHERE`, `GROUP BY`, `HAVING` e
//! `ORDER BY`. Qualquer outra coisa (função do produto, `LIMIT`, `UNION`, janela, `WITH`...) faz a tradução devolver `None`, e
//! quem chamou avisa que o objeto não atravessou: uma tradução errada com cara de certa seria pior que nenhuma.

use crate::genext::Dialect;
use crate::neutral::Product;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    /// Nome entre aspas.
    Quoted(String),
    /// Palavra sem aspas (palavra-chave, função ou nome).
    Word(String),
    Str(String),
    Num(String),
    Sym(String),
}

fn tokenize(sql: &str) -> Option<Vec<Tok>> {
    let c: Vec<char> = sql.chars().collect();
    let (mut i, mut out) = (0usize, Vec::new());
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '\'' {
            let mut s = String::new();
            i += 1;
            loop {
                match c.get(i) {
                    None => return None,
                    Some('\'') if c.get(i + 1) == Some(&'\'') => {
                        s.push('\'');
                        i += 2;
                    }
                    Some('\'') => {
                        i += 1;
                        break;
                    }
                    Some(x) => {
                        s.push(*x);
                        i += 1;
                    }
                }
            }
            // `N'texto'` do SQL Server: o `N` não faz parte do texto.
            if matches!(out.last(), Some(Tok::Word(w)) if w.eq_ignore_ascii_case("n")) {
                out.pop();
            }
            out.push(Tok::Str(s));
        } else if matches!(ch, '"' | '`' | '[') {
            let close = match ch {
                '[' => ']',
                x => x,
            };
            let mut s = String::new();
            i += 1;
            loop {
                match c.get(i) {
                    None => return None,
                    Some(x) if *x == close && c.get(i + 1) == Some(&close) && close != ']' => {
                        s.push(close);
                        i += 2;
                    }
                    Some(x) if *x == close => {
                        i += 1;
                        break;
                    }
                    Some(x) => {
                        s.push(*x);
                        i += 1;
                    }
                }
            }
            out.push(Tok::Quoted(s));
        } else if ch.is_ascii_digit() {
            let start = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.') {
                i += 1;
            }
            out.push(Tok::Num(c[start..i].iter().collect()));
        } else if ch.is_alphabetic() || ch == '_' || ch == '@' || ch == '#' {
            let start = i;
            while i < c.len() && (c[i].is_alphanumeric() || matches!(c[i], '_' | '$' | '@' | '#')) {
                i += 1;
            }
            out.push(Tok::Word(c[start..i].iter().collect()));
        } else {
            let two: String = c[i..(i + 2).min(c.len())].iter().collect();
            if matches!(two.as_str(), "<>" | "!=" | "<=" | ">=" | "||" | "::") {
                out.push(Tok::Sym(two));
                i += 2;
            } else if "=<>+-*/%(),.;".contains(ch) {
                out.push(Tok::Sym(ch.to_string()));
                i += 1;
            } else {
                return None;
            }
        }
    }
    Some(out)
}

/// Palavras que passam como estão.
const KEYWORDS: &[&str] = &[
    "AND", "OR", "NOT", "IN", "IS", "NULL", "LIKE", "BETWEEN", "CASE", "WHEN", "THEN", "ELSE", "END", "SELECT", "DISTINCT", "FROM", "WHERE", "GROUP", "BY", "HAVING", "ORDER", "ASC", "DESC",
    "JOIN", "INNER", "LEFT", "RIGHT", "FULL", "OUTER", "CROSS", "ON", "AS", "EXISTS",
];

/// Palavras que fazem a tradução desistir.
const REJECT: &[&str] = &[
    "LIMIT", "OFFSET", "TOP", "FETCH", "UNION", "INTERSECT", "EXCEPT", "WITH", "OVER", "PARTITION", "WINDOW", "LATERAL", "ARRAY", "ANY", "ALL", "SOME", "ILIKE", "SIMILAR", "REGEXP", "RLIKE", "ESCAPE",
    "INTO", "FOR", "RETURNING", "USING", "NATURAL", "CAST", "CONVERT", "INTERVAL", "COLLATE",
];

/// A função como o destino a escreve (`None` = sem equivalente).
fn function(name: &str, dst: Product) -> Option<&'static str> {
    Some(match name.to_ascii_uppercase().as_str() {
        "COUNT" => "COUNT",
        "SUM" => "SUM",
        "AVG" => "AVG",
        "MIN" => "MIN",
        "MAX" => "MAX",
        "COALESCE" => "COALESCE",
        "UPPER" => "UPPER",
        "LOWER" => "LOWER",
        "ABS" => "ABS",
        "ROUND" => "ROUND",
        "TRIM" => "TRIM",
        "LENGTH" | "LEN" | "CHAR_LENGTH" | "CHARACTER_LENGTH" => match dst {
            Product::Mssql => "LEN",
            Product::Mysql => "CHAR_LENGTH",
            _ => "LENGTH",
        },
        _ => return None,
    })
}

pub struct Ctx<'a> {
    pub dst: Product,
    /// O schema da origem vira o do destino nos nomes qualificados.
    pub src_schema: &'a str,
    pub dst_schema: &'a str,
    /// `SELECT` (aceita `FROM`, `JOIN`...) ou só expressão (`CHECK`).
    pub select: bool,
}

/// Traduz o texto. `None` quando algo não tem equivalente.
pub fn translate(sql: &str, cx: &Ctx<'_>) -> Option<String> {
    let toks = tokenize(sql.trim().trim_end_matches(';'))?;
    let d: Dialect = cx.dst.dialect();
    let mut out: Vec<String> = Vec::new();
    let mut after_from = false;
    let mut i = 0usize;
    while i < toks.len() {
        match &toks[i] {
            Tok::Sym(s) if s == "::" => {
                // `expr::tipo` do PostgreSQL: o tipo some (o destino converte o que precisa).
                i += 1;
                let mut depth = 0i32;
                while i < toks.len() {
                    match &toks[i] {
                        Tok::Word(_) => {}
                        Tok::Sym(p) if p == "(" => depth += 1,
                        Tok::Sym(p) if p == ")" => {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                        }
                        Tok::Num(_) | Tok::Sym(_) if depth > 0 => {}
                        _ => break,
                    }
                    i += 1;
                }
                continue;
            }
            Tok::Sym(s) if s == ";" => {}
            Tok::Sym(s) if s == "||" => match cx.dst {
                Product::Mysql => return None,
                Product::Mssql => out.push("+".into()),
                _ => out.push("||".into()),
            },
            Tok::Sym(s) if s == "!=" => out.push("<>".into()),
            Tok::Sym(s) => out.push(s.clone()),
            Tok::Num(n) => out.push(n.clone()),
            Tok::Str(s) => out.push(d.text(s)),
            Tok::Quoted(name) => {
                let next_is_call = matches!(toks.get(i + 1), Some(Tok::Sym(p)) if p == "(");
                if next_is_call {
                    return None;
                }
                out.push(qualified(&toks, &mut i, name, cx, d, std::mem::take(&mut after_from)));
                continue;
            }
            Tok::Word(w) => {
                let up = w.to_ascii_uppercase();
                let call = matches!(toks.get(i + 1), Some(Tok::Sym(p)) if p == "(");
                if REJECT.contains(&up.as_str()) {
                    return None;
                }
                if call && !KEYWORDS.contains(&up.as_str()) {
                    out.push(function(w, cx.dst)?.to_string());
                } else if matches!(up.as_str(), "NOW" | "GETDATE" | "SYSDATETIME" | "CURRENT_TIMESTAMP" | "LOCALTIMESTAMP") {
                    // `now()` e parentes: vira a palavra que os quatro entendem.
                    out.push("CURRENT_TIMESTAMP".into());
                    if call && matches!(toks.get(i + 2), Some(Tok::Sym(p)) if p == ")") {
                        i += 2;
                    }
                } else if up == "TRUE" || up == "FALSE" {
                    out.push(if cx.dst == Product::Pg { up.to_lowercase() } else if up == "TRUE" { "1".into() } else { "0".into() });
                } else if KEYWORDS.contains(&up.as_str()) {
                    if !cx.select && matches!(up.as_str(), "SELECT" | "FROM" | "JOIN" | "GROUP" | "HAVING" | "ORDER" | "EXISTS") {
                        return None;
                    }
                    after_from = matches!(up.as_str(), "FROM" | "JOIN");
                    out.push(up);
                } else {
                    out.push(qualified(&toks, &mut i, w, cx, d, std::mem::take(&mut after_from)));
                    continue;
                }
            }
        }
        i += 1;
    }
    Some(join(&out))
}

/// Um nome (com o qualificador que vier a seguir): o schema da origem vira o do destino e tudo sai entre as aspas do destino.
fn qualified(toks: &[Tok], i: &mut usize, first: &str, cx: &Ctx<'_>, d: Dialect, is_table: bool) -> String {
    let mut parts = vec![first.to_string()];
    *i += 1;
    while matches!(toks.get(*i), Some(Tok::Sym(p)) if p == ".") {
        match toks.get(*i + 1) {
            Some(Tok::Quoted(n)) | Some(Tok::Word(n)) => {
                parts.push(n.clone());
                *i += 2;
            }
            Some(Tok::Sym(p)) if p == "*" => {
                parts.push("*".into());
                *i += 2;
            }
            _ => break,
        }
    }
    if cx.select {
        if parts.len() > 1 && parts[0].eq_ignore_ascii_case(cx.src_schema) {
            parts[0] = cx.dst_schema.to_string();
        } else if is_table && parts.len() == 1 && cx.dst != Product::Sqlite {
            // `FROM tabela` sem schema: no destino a tabela está no schema da transferência, que pode não estar no caminho de busca.
            parts.insert(0, cx.dst_schema.to_string());
        }
        // O SQLite não tem schema na frente do nome: fica `tabela` ou `tabela.coluna`.
        if cx.dst == Product::Sqlite && parts.len() > 1 && parts[0].eq_ignore_ascii_case(cx.dst_schema) && (is_table || parts.len() == 3) {
            parts.remove(0);
        }
    }
    parts.iter().map(|p| if p == "*" { "*".to_string() } else { d.q(p) }).collect::<Vec<_>>().join(".")
}

fn join(parts: &[String]) -> String {
    let mut s = String::new();
    for (i, p) in parts.iter().enumerate() {
        let tight_before = matches!(p.as_str(), ")" | "," | ".");
        let tight_after_prev = i > 0 && matches!(parts[i - 1].as_str(), "(" | ".");
        // `COUNT(` / `UPPER(`: o parêntese de chamada gruda no nome da função.
        let call_paren = p == "(" && i > 0 && parts[i - 1].chars().next().is_some_and(|c| c.is_ascii_uppercase()) && !KEYWORDS.contains(&parts[i - 1].as_str());
        if i > 0 && !tight_before && !tight_after_prev && !call_paren {
            s.push(' ');
        }
        s.push_str(p);
    }
    s
}

/// A forma de comparar dois textos: sem aspas, sem espaços, sem parênteses, sem conversões (`::tipo`) e em minúsculas. Cada servidor
/// reescreve a expressão do seu jeito (`(price >= (0)::numeric)`, `([price]>=(0))`); o que sobra é o que importa.
pub fn compare_form(sql: &str) -> String {
    let lower = sql.to_lowercase();
    let c: Vec<char> = lower.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    while i < c.len() {
        if c[i] == ':' && c.get(i + 1) == Some(&':') {
            i += 2;
            while i < c.len() && (c[i].is_alphanumeric() || matches!(c[i], '_' | ' ')) {
                i += 1;
            }
            // `::numeric(10,2)`: os argumentos do tipo também saem.
            if c.get(i) == Some(&'(') {
                while i < c.len() && c[i] != ')' {
                    i += 1;
                }
                i += 1;
            }
            continue;
        }
        if !c[i].is_whitespace() && !matches!(c[i], '"' | '`' | '[' | ']' | '\'' | '(' | ')' | ';') {
            out.push(c[i]);
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx<'a>(src: Product, dst: Product, select: bool) -> Ctx<'a> {
        let _ = src;
        Ctx { dst, src_schema: "public", dst_schema: "dbo", select }
    }

    #[test]
    fn checks_cross_over() {
        let c = cx(Product::Pg, Product::Mssql, false);
        assert_eq!(translate("((qty >= 0))", &c).as_deref(), Some("(([qty] >= 0))"));
        assert_eq!(translate("((status)::text = ANY (ARRAY['a'::text]))", &c), None);
        assert_eq!(translate("(length((name)::text) > 2)", &c).as_deref(), Some("(LEN(([name])) > 2)"));
        let m = cx(Product::Mysql, Product::Pg, false);
        assert_eq!(translate("(`qty` between 1 and 9)", &m).as_deref(), Some("(\"qty\" BETWEEN 1 AND 9)"));
        assert_eq!(translate("(`a` regexp 'x')", &m), None);
    }

    #[test]
    fn selects_cross_over() {
        let c = cx(Product::Pg, Product::Mysql, true);
        let out = translate("SELECT p.id, p.name FROM public.parent p WHERE p.id > 1 ORDER BY p.id", &c).unwrap();
        assert!(out.contains("`dbo`.`parent`") && out.contains("`p`.`id`") && out.contains("ORDER BY"), "{out}");
        assert_eq!(translate("SELECT id FROM t LIMIT 3", &c), None);
        assert_eq!(translate("SELECT a || b FROM t", &c), None);
    }

    #[test]
    fn compare_ignores_quotes_and_case() {
        assert_eq!(compare_form("((\"Qty\" >= 0))"), compare_form("(qty>=0)"));
        assert_eq!(compare_form("((price >= (0)::numeric))"), compare_form("([price]>=(0))"));
    }
}
