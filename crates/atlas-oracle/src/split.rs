//! Separa um texto em comandos. No Oracle o `;` termina um comando SQL comum (e não vai ao servidor), mas dentro de um bloco PL/SQL
//! (`BEGIN`, `DECLARE`, `CREATE [OR REPLACE] FUNCTION/PROCEDURE/PACKAGE/TRIGGER/TYPE`) ele faz parte do texto: o bloco termina numa linha
//! só com `/` (como no SQL*Plus) ou no fim do texto.

/// Um comando pronto para enviar e onde ele começa no texto (em caracteres).
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub sql: String,
    pub start: usize,
}

/// As primeiras palavras do comando, em maiúsculas, sem espaços nem comentários antes.
pub fn words(sql: &str, n: usize) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0usize;
    let mut out = Vec::new();
    while out.len() < n {
        // Espaços e comentários.
        loop {
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if chars.get(i) == Some(&'-') && chars.get(i + 1) == Some(&'-') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            } else if chars.get(i) == Some(&'/') && chars.get(i + 1) == Some(&'*') {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i = (i + 2).min(chars.len());
            } else {
                break;
            }
        }
        if i >= chars.len() {
            break;
        }
        let mut w = String::new();
        while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '$' | '#')) {
            w.push(chars[i].to_ascii_uppercase());
            i += 1;
        }
        if w.is_empty() {
            w.push(chars[i]);
            i += 1;
        }
        out.push(w);
    }
    out
}

/// O comando é um bloco PL/SQL (o `;` não o termina)?
pub fn is_plsql(sql: &str) -> bool {
    let w = words(sql, 6);
    let first = w.first().map(String::as_str);
    if matches!(first, Some("BEGIN" | "DECLARE")) {
        return true;
    }
    if first != Some("CREATE") {
        return false;
    }
    let mut i = 1;
    if w.get(i).map(String::as_str) == Some("OR") && w.get(i + 1).map(String::as_str) == Some("REPLACE") {
        i += 2;
    }
    while matches!(w.get(i).map(String::as_str), Some("EDITIONABLE" | "NONEDITIONABLE")) {
        i += 1;
    }
    matches!(w.get(i).map(String::as_str), Some("FUNCTION" | "PROCEDURE" | "PACKAGE" | "TRIGGER" | "TYPE" | "LIBRARY"))
}

/// O comando devolve linhas (`SELECT`, `WITH`)?
pub fn returns_rows(sql: &str) -> bool {
    let w = words(sql, 1);
    matches!(w.first().map(String::as_str), Some("SELECT" | "WITH" | "("))
}

fn flush(cur: &mut String, start: usize, out: &mut Vec<Piece>, block_end: bool) {
    let mut sql = cur.trim().to_string();
    if !block_end && !is_plsql(&sql) {
        while sql.ends_with(';') {
            sql.pop();
            sql = sql.trim_end().to_string();
        }
    }
    if !sql.is_empty() && !words(&sql, 1).is_empty() {
        // `start` já aponta para o primeiro caractere que não é espaço.
        out.push(Piece { sql, start });
    }
    cur.clear();
}

/// Divide o texto em comandos. Comentários e textos entre aspas não confundem o `;` nem o `/`.
pub fn split_statements(text: &str) -> Vec<Piece> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_start = 0usize;
    let mut i = 0usize;
    let mut line_start = true;

    while i < chars.len() {
        let c = chars[i];
        if cur.trim().is_empty() {
            cur_start = i;
        }
        // Linha só com `/`: fim de um bloco PL/SQL.
        if line_start && c == '/' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '\n' && chars[j].is_whitespace() {
                j += 1;
            }
            if j >= chars.len() || chars[j] == '\n' {
                flush(&mut cur, cur_start, &mut out, true);
                i = j;
                continue;
            }
        }
        match c {
            '\'' => {
                // `q'[ ... ]'` aceita aspas dentro; o resto dobra a aspa.
                let q = cur.ends_with('q') || cur.ends_with('Q');
                let mut j = i + 1;
                cur.push(c);
                if q && j < chars.len() && !chars[j].is_whitespace() {
                    let close = match chars[j] {
                        '[' => ']',
                        '(' => ')',
                        '{' => '}',
                        '<' => '>',
                        d => d,
                    };
                    cur.push(chars[j]);
                    j += 1;
                    while j < chars.len() && !(chars[j] == close && chars.get(j + 1) == Some(&'\'')) {
                        cur.push(chars[j]);
                        j += 1;
                    }
                    if j < chars.len() {
                        cur.push(chars[j]);
                        cur.push('\'');
                        j += 2;
                    }
                } else {
                    while j < chars.len() {
                        cur.push(chars[j]);
                        if chars[j] == '\'' {
                            if chars.get(j + 1) == Some(&'\'') {
                                cur.push('\'');
                                j += 2;
                                continue;
                            }
                            j += 1;
                            break;
                        }
                        j += 1;
                    }
                }
                i = j;
                line_start = false;
                continue;
            }
            '"' => {
                cur.push(c);
                let mut j = i + 1;
                while j < chars.len() {
                    cur.push(chars[j]);
                    j += 1;
                    if chars[j - 1] == '"' {
                        break;
                    }
                }
                i = j;
                line_start = false;
                continue;
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                while i < chars.len() && chars[i] != '\n' {
                    cur.push(chars[i]);
                    i += 1;
                }
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                cur.push_str("/*");
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    cur.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    cur.push_str("*/");
                    i += 2;
                }
                line_start = false;
                continue;
            }
            ';' if !is_plsql(&cur) => {
                flush(&mut cur, cur_start, &mut out, false);
                i += 1;
                line_start = false;
                continue;
            }
            _ => {}
        }
        cur.push(c);
        line_start = c == '\n' || (line_start && c.is_whitespace());
        i += 1;
    }
    let block = is_plsql(&cur);
    flush(&mut cur, cur_start, &mut out, block);
    out
}

/// A primeira variável de ligação (`:nome` ou `:1`) do comando, com a posição (em caracteres) dos dois pontos. O Atlas manda o texto sem
/// parâmetros, então uma variável sem valor faria o servidor esperar dados que nunca vêm (a conexão trava): melhor recusar antes. Ignora
/// textos, identificadores entre aspas e comentários; num bloco PL/SQL também `:NEW`, `:OLD` e `:PARENT` (gatilhos), que não são variáveis.
pub fn unbound_placeholder(sql: &str) -> Option<(String, usize)> {
    let chars: Vec<char> = sql.chars().collect();
    let plsql = is_plsql(sql);
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '\'' => {
                let q = i > 0 && matches!(chars[i - 1], 'q' | 'Q');
                if q && i + 1 < chars.len() && !chars[i + 1].is_whitespace() {
                    let close = match chars[i + 1] {
                        '[' => ']',
                        '(' => ')',
                        '{' => '}',
                        '<' => '>',
                        d => d,
                    };
                    i += 2;
                    while i + 1 < chars.len() && !(chars[i] == close && chars[i + 1] == '\'') {
                        i += 1;
                    }
                    i += 2;
                } else {
                    i += 1;
                    while i < chars.len() {
                        if chars[i] == '\'' {
                            if chars.get(i + 1) == Some(&'\'') {
                                i += 2;
                                continue;
                            }
                            break;
                        }
                        i += 1;
                    }
                    i += 1;
                }
            }
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    i += 1;
                }
                i += 1;
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            ':' => {
                let start = i;
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_alphanumeric() || matches!(chars[j], '_' | '$' | '#')) {
                    j += 1;
                }
                if j > i + 1 {
                    let name: String = chars[i + 1..j].iter().collect();
                    let upper = name.to_uppercase();
                    if !(plsql && matches!(upper.as_str(), "NEW" | "OLD" | "PARENT")) {
                        return Some((name, start));
                    }
                }
                i = j.max(i + 1);
            }
            _ => i += 1,
        }
    }
    None
}
