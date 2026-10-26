//! Separa um texto em comandos. O Firebird só executa um comando por vez, então o console e o editor precisam dividir o texto.
//! Respeita aspas, comentários, `SET TERM` (como o isql) e o corpo de `EXECUTE BLOCK`, gatilhos e procedimentos
//! (`BEGIN ... END`, onde o `;` não termina o comando).

/// Um comando já separado: o texto e a posição (em caracteres) em que começa no texto original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub sql: String,
    pub start: usize,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// A primeira palavra do comando em maiúsculas, ignorando comentários e espaços ("" se não houver).
pub fn first_word(sql: &str) -> String {
    words(sql, 1).into_iter().next().unwrap_or_default()
}

/// As primeiras `n` palavras do comando em maiúsculas, ignorando comentários.
pub fn words(sql: &str, n: usize) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() && out.len() < n {
        let c = chars[i];
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else if is_word(c) {
            let start = i;
            while i < chars.len() && is_word(chars[i]) {
                i += 1;
            }
            out.push(chars[start..i].iter().collect::<String>().to_uppercase());
        } else if c == '"' || c == '\'' {
            let q = c;
            i += 1;
            while i < chars.len() && chars[i] != q {
                i += 1;
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

/// O comando cria ou altera um objeto com corpo em PSQL (`CREATE TRIGGER`, `EXECUTE BLOCK`...), onde `;` aparece dentro do `BEGIN ... END`.
fn has_body(w: &[String]) -> bool {
    let is = |i: usize, s: &str| w.get(i).is_some_and(|x| x == s);
    if is(0, "EXECUTE") && is(1, "BLOCK") {
        return true;
    }
    let mut i;
    if is(0, "CREATE") || is(0, "RECREATE") || is(0, "ALTER") {
        i = if is(1, "OR") && is(2, "ALTER") { 3 } else { 1 };
    } else {
        return false;
    }
    while w.get(i).is_some_and(|x| matches!(x.as_str(), "GLOBAL" | "TEMPORARY" | "OR")) {
        i += 1;
    }
    matches!(w.get(i).map(String::as_str), Some("TRIGGER" | "PROCEDURE" | "FUNCTION" | "PACKAGE"))
}

/// Divide `sql` em comandos. Sem `;` final o último comando também entra; comandos vazios são descartados.
pub fn split_statements(sql: &str) -> Vec<Piece> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut term = String::from(";");
    let mut i = 0;
    let mut start = 0usize;
    let mut depth: i32 = 0;
    let mut body: Option<bool> = None;
    let flush = |out: &mut Vec<Piece>, from: usize, to: usize| {
        let text: String = chars[from..to].iter().collect();
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            let lead = text.chars().take_while(|c| c.is_whitespace()).count();
            out.push(Piece { sql: trimmed.to_string(), start: from + lead });
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' || c == '"' {
            i += 1;
            while i < chars.len() {
                if chars[i] == c {
                    if chars.get(i + 1) == Some(&c) {
                        i += 2;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        if is_word(c) && (i == 0 || !is_word(chars[i - 1])) {
            let ws = i;
            while i < chars.len() && is_word(chars[i]) {
                i += 1;
            }
            let word: String = chars[ws..i].iter().collect::<String>().to_uppercase();
            // `SET TERM novo velho` troca o terminador; a linha inteira é consumida (o servidor não a conhece).
            if word == "SET" && chars[start..ws].iter().all(|c| c.is_whitespace()) {
                let line_end = chars[i..].iter().position(|c| *c == '\n').map(|p| i + p).unwrap_or(chars.len());
                let rest: String = chars[i..line_end].iter().collect();
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.len() >= 2 && parts[0].eq_ignore_ascii_case("TERM") {
                    term = parts[1].trim_end_matches(';').to_string();
                    if term.is_empty() {
                        term = ";".into();
                    }
                    // Quando o terminador novo vem seguido de `;` na mesma linha ("SET TERM ^ ;"), o `;` faz parte do comando.
                    i = line_end;
                    start = i;
                    continue;
                }
            }
            if body.is_none() && depth == 0 {
                body = Some(has_body(&words(&chars[start..].iter().take(200).collect::<String>(), 6)));
            }
            if body == Some(true) {
                match word.as_str() {
                    "BEGIN" | "CASE" => depth += 1,
                    "END" => depth -= 1,
                    _ => {}
                }
            }
            continue;
        }
        let term_chars: Vec<char> = term.chars().collect();
        if !term_chars.is_empty() && chars[i..].starts_with(&term_chars) && !(term == ";" && body == Some(true) && depth > 0) {
            flush(&mut out, start, i);
            i += term_chars.len();
            start = i;
            depth = 0;
            body = None;
            continue;
        }
        i += 1;
    }
    flush(&mut out, start, chars.len());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(sql: &str) -> Vec<String> {
        split_statements(sql).into_iter().map(|p| p.sql).collect()
    }

    #[test]
    fn plain_statements() {
        assert_eq!(texts("select 1 from rdb$database; select ';' from rdb$database;\n-- ;\nselect 2"), vec!["select 1 from rdb$database", "select ';' from rdb$database", "-- ;\nselect 2"]);
    }

    #[test]
    fn trigger_body_keeps_semicolons() {
        let sql = "create trigger t for x before insert as begin new.a = 1; if (1=1) then begin new.b = 2; end end; select 1 from rdb$database";
        assert_eq!(texts(sql).len(), 2);
    }

    #[test]
    fn set_term() {
        let sql = "set term ^ ;\ncreate procedure p as begin end^\nset term ; ^\nselect 1 from rdb$database;";
        assert_eq!(texts(sql), vec!["create procedure p as begin end", "select 1 from rdb$database"]);
    }

    #[test]
    fn execute_block() {
        let sql = "execute block as begin insert into a values (1); end; select 1 from rdb$database;";
        assert_eq!(texts(sql).len(), 2);
    }

    #[test]
    fn words_skip_comments() {
        assert_eq!(first_word("/* x */ -- y\n  select 1"), "SELECT");
    }
}
