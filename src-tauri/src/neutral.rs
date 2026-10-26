//! Modelo neutro de tipos e de tabelas para levar estrutura e dados de um produto a outro (PostgreSQL, SQL Server, MySQL e SQLite).
//!
//! O tipo de cada coluna é lido na linguagem do produto de origem (`NType`) e escrito na do destino; os valores passam por uma forma
//! canônica em texto (booleano `true`/`false`, binário em hexa, data e hora sem fuso quando o destino não tem fuso). Só o que tem
//! equivalente seguro atravessa: tabelas, colunas, chave primária, únicas, chaves estrangeiras, índices simples e valores padrão
//! simples. Restrições `CHECK`, gatilhos, views e rotinas ficam de fora, porque o texto delas é da linguagem do produto.

use crate::genext::Dialect;
use atlas_core::{ColumnInfo, ConstraintInfo, IndexInfo, TableDetail};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Product {
    Pg,
    Mysql,
    Mssql,
    Sqlite,
}

impl Product {
    /// O produto de uma conexão, pelo prefixo do id (o PostgreSQL não tem prefixo).
    pub fn of_conn(id: &str) -> Product {
        if id.starts_with("mysql_") {
            Product::Mysql
        } else if id.starts_with("mssql_") {
            Product::Mssql
        } else if id.starts_with("sqlite_") {
            Product::Sqlite
        } else {
            Product::Pg
        }
    }

    pub fn dialect(self) -> Dialect {
        match self {
            Product::Pg => Dialect::Pg,
            Product::Mysql => Dialect::Mysql,
            Product::Mssql => Dialect::Mssql,
            Product::Sqlite => Dialect::Sqlite,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum NType {
    /// Bytes: 1, 2, 4 ou 8.
    Int(u8),
    /// Precisão e escala, quando o tipo as declara.
    Decimal(Option<(u32, u32)>),
    /// Bytes: 4 ou 8.
    Float(u8),
    /// Texto com tamanho máximo, ou `None` sem limite.
    Text(Option<u32>),
    Bool,
    Date,
    Time,
    /// Com fuso?
    Timestamp(bool),
    Binary,
    Uuid,
    Json,
    /// Sem equivalente conhecido: vai como texto.
    Other,
}

fn args(t: &str) -> Vec<u32> {
    t.split_once('(').and_then(|(_, r)| r.split_once(')')).map(|(a, _)| a.split(',').filter_map(|x| x.trim().parse::<u32>().ok()).collect()).unwrap_or_default()
}

fn decimal(t: &str) -> NType {
    let a = args(t);
    NType::Decimal(match a.as_slice() {
        [p, s] => Some((*p, *s)),
        [p] => Some((*p, 0)),
        _ => None,
    })
}

/// O tipo declarado na coluna (`formatted_type` do catálogo do produto) como tipo neutro.
pub fn parse(product: Product, formatted: &str) -> NType {
    let t = formatted.trim().to_lowercase();
    let base = t.split(['(', ' ']).next().unwrap_or(&t).to_string();
    let unsigned = t.contains("unsigned");
    match product {
        Product::Pg => match t.as_str() {
            "smallint" => NType::Int(2),
            "integer" => NType::Int(4),
            "bigint" => NType::Int(8),
            "real" => NType::Float(4),
            "double precision" => NType::Float(8),
            "text" | "name" | "citext" => NType::Text(None),
            "boolean" => NType::Bool,
            "date" => NType::Date,
            "bytea" => NType::Binary,
            "uuid" => NType::Uuid,
            "json" | "jsonb" => NType::Json,
            _ if t.starts_with("numeric") || t.starts_with("decimal") => decimal(&t),
            _ if t.starts_with("character varying") || t.starts_with("varchar") => NType::Text(args(&t).first().copied()),
            _ if t.starts_with("character(") || t.starts_with("char(") || t == "character" || t == "bpchar" => NType::Text(args(&t).first().copied().or(Some(1))),
            _ if t.starts_with("time ") || t == "time" => NType::Time,
            _ if t.starts_with("timestamp") => NType::Timestamp(t.contains("with time zone")),
            _ => NType::Other,
        },
        Product::Mysql => match base.as_str() {
            "tinyint" if t.starts_with("tinyint(1)") => NType::Bool,
            "bit" if t.starts_with("bit(1)") || t == "bit" => NType::Bool,
            "tinyint" => NType::Int(if unsigned { 2 } else { 1 }),
            "smallint" => NType::Int(if unsigned { 4 } else { 2 }),
            "mediumint" | "int" | "integer" => NType::Int(if unsigned { 8 } else { 4 }),
            "bigint" => if unsigned { NType::Decimal(Some((20, 0))) } else { NType::Int(8) },
            "year" => NType::Int(2),
            "decimal" | "numeric" => decimal(&t),
            "float" => NType::Float(4),
            "double" | "real" => NType::Float(8),
            "char" | "varchar" => NType::Text(args(&t).first().copied()),
            "tinytext" | "text" | "mediumtext" | "longtext" | "enum" | "set" => NType::Text(None),
            "date" => NType::Date,
            "time" => NType::Time,
            "datetime" | "timestamp" => NType::Timestamp(false),
            "blob" | "tinyblob" | "mediumblob" | "longblob" | "binary" | "varbinary" => NType::Binary,
            "json" => NType::Json,
            _ => NType::Other,
        },
        Product::Mssql => match base.as_str() {
            "tinyint" => NType::Int(2),
            "smallint" => NType::Int(2),
            "int" => NType::Int(4),
            "bigint" => NType::Int(8),
            "bit" => NType::Bool,
            "decimal" | "numeric" => decimal(&t),
            "money" => NType::Decimal(Some((19, 4))),
            "smallmoney" => NType::Decimal(Some((10, 4))),
            "float" => NType::Float(8),
            "real" => NType::Float(4),
            "char" | "varchar" | "nchar" | "nvarchar" => {
                if t.contains("(max)") || t.contains("(-1)") { NType::Text(None) } else { NType::Text(args(&t).first().copied()) }
            }
            "text" | "ntext" | "xml" => NType::Text(None),
            "date" => NType::Date,
            "time" => NType::Time,
            "datetime" | "datetime2" | "smalldatetime" => NType::Timestamp(false),
            "datetimeoffset" => NType::Timestamp(true),
            "binary" | "varbinary" | "image" => NType::Binary,
            "uniqueidentifier" => NType::Uuid,
            _ => NType::Other,
        },
        Product::Sqlite => {
            let u = t.to_uppercase();
            if u.contains("BOOL") {
                NType::Bool
            } else if u.contains("INT") {
                NType::Int(8)
            } else if u.contains("CHAR") {
                NType::Text(args(&t).first().copied())
            } else if u.contains("CLOB") || u.contains("TEXT") {
                NType::Text(None)
            } else if u.contains("REAL") || u.contains("FLOA") || u.contains("DOUB") {
                NType::Float(8)
            } else if u.contains("DEC") || u.contains("NUM") {
                decimal(&t)
            } else if u.contains("DATETIME") || u.contains("TIMESTAMP") {
                NType::Timestamp(false)
            } else if u.contains("DATE") {
                NType::Date
            } else if u.contains("TIME") {
                NType::Time
            } else if u.contains("BLOB") {
                NType::Binary
            } else if u.contains("JSON") {
                NType::Json
            } else {
                NType::Other
            }
        }
    }
}

/// O tipo neutro escrito na linguagem do produto de destino.
pub fn render(product: Product, t: &NType) -> String {
    match product {
        Product::Pg => match t {
            NType::Int(1 | 2) => "smallint".into(),
            NType::Int(4) => "integer".into(),
            NType::Int(_) => "bigint".into(),
            NType::Decimal(Some((p, s))) => format!("numeric({},{})", (*p).clamp(1, 1000), s.min(p)),
            NType::Decimal(None) => "numeric".into(),
            NType::Float(4) => "real".into(),
            NType::Float(_) => "double precision".into(),
            NType::Text(Some(n)) => format!("character varying({n})"),
            NType::Text(None) | NType::Other => "text".into(),
            NType::Bool => "boolean".into(),
            NType::Date => "date".into(),
            NType::Time => "time".into(),
            NType::Timestamp(false) => "timestamp".into(),
            NType::Timestamp(true) => "timestamptz".into(),
            NType::Binary => "bytea".into(),
            NType::Uuid => "uuid".into(),
            NType::Json => "jsonb".into(),
        },
        Product::Mysql => match t {
            NType::Int(1) => "tinyint".into(),
            NType::Int(2) => "smallint".into(),
            NType::Int(4) => "int".into(),
            NType::Int(_) => "bigint".into(),
            NType::Decimal(Some((p, s))) => format!("decimal({},{})", (*p).clamp(1, 65), (*s).min(30).min(*p)),
            NType::Decimal(None) => "decimal(38,10)".into(),
            NType::Float(4) => "float".into(),
            NType::Float(_) => "double".into(),
            NType::Text(Some(n)) if *n <= 16000 => format!("varchar({n})"),
            NType::Text(_) | NType::Other => "longtext".into(),
            NType::Bool => "tinyint(1)".into(),
            NType::Date => "date".into(),
            NType::Time => "time(6)".into(),
            NType::Timestamp(_) => "datetime(6)".into(),
            NType::Binary => "longblob".into(),
            NType::Uuid => "char(36)".into(),
            NType::Json => "json".into(),
        },
        Product::Mssql => match t {
            NType::Int(1 | 2) => "smallint".into(),
            NType::Int(4) => "int".into(),
            NType::Int(_) => "bigint".into(),
            NType::Decimal(Some((p, s))) => format!("decimal({},{})", (*p).clamp(1, 38), (*s).min(38).min(*p)),
            NType::Decimal(None) => "decimal(38,10)".into(),
            NType::Float(4) => "real".into(),
            NType::Float(_) => "float".into(),
            NType::Text(Some(n)) if *n <= 4000 => format!("nvarchar({n})"),
            NType::Text(_) | NType::Other | NType::Json => "nvarchar(max)".into(),
            NType::Bool => "bit".into(),
            NType::Date => "date".into(),
            NType::Time => "time".into(),
            NType::Timestamp(false) => "datetime2".into(),
            NType::Timestamp(true) => "datetimeoffset".into(),
            NType::Binary => "varbinary(max)".into(),
            NType::Uuid => "uniqueidentifier".into(),
        },
        Product::Sqlite => match t {
            NType::Int(_) => "integer".into(),
            NType::Decimal(_) => "numeric".into(),
            NType::Float(_) => "real".into(),
            NType::Text(Some(n)) => format!("varchar({n})"),
            NType::Text(None) | NType::Other | NType::Uuid | NType::Json => "text".into(),
            NType::Bool => "boolean".into(),
            NType::Date => "date".into(),
            NType::Time => "time".into(),
            NType::Timestamp(_) => "datetime".into(),
            NType::Binary => "blob".into(),
        },
    }
}

fn is_plain_number(s: &str) -> bool {
    let body = s.strip_prefix('-').unwrap_or(s);
    let mut parts = body.splitn(2, '.');
    let int = parts.next().unwrap_or("");
    !int.is_empty() && int.bytes().all(|b| b.is_ascii_digit()) && parts.next().is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
}

/// Tira o fuso (`+00`, `-03:00`, `Z`) de uma data e hora e troca o `T` por espaço.
fn strip_offset(v: &str) -> String {
    let v = v.trim().replace('T', " ");
    if let Some(stripped) = v.strip_suffix('Z') {
        return stripped.to_string();
    }
    if let Some(pos) = v.rfind(['+', '-'])
        && pos > 10
        && v[pos + 1..].chars().all(|c| c.is_ascii_digit() || c == ':')
    {
        return v[..pos].trim_end().to_string();
    }
    v
}

/// A forma canônica de um valor (para comparar e para converter): booleano `1`/`0`, binário em hexa minúsculo sem prefixo, data e
/// hora sem fuso nem zeros finais, número sem zeros à direita da vírgula.
pub fn canonical(t: &NType, v: &str) -> String {
    match t {
        NType::Bool => match v.trim().to_lowercase().as_str() {
            "t" | "true" | "1" | "y" | "yes" | "on" => "1".into(),
            _ => "0".into(),
        },
        NType::Binary => v.strip_prefix("\\x").or_else(|| v.strip_prefix("0x")).unwrap_or(v).to_lowercase(),
        NType::Timestamp(_) | NType::Time => {
            let s = strip_offset(v);
            match s.split_once('.') {
                Some((a, frac)) => {
                    let frac = frac.trim_end_matches('0');
                    if frac.is_empty() { a.to_string() } else { format!("{a}.{frac}") }
                }
                None => s,
            }
        }
        NType::Date => v.trim().chars().take(10).collect(),
        NType::Int(_) | NType::Decimal(_) => {
            let s = v.trim();
            if s.contains('.') && is_plain_number(s) {
                let t = s.trim_end_matches('0').trim_end_matches('.');
                if t.is_empty() || t == "-" { "0".into() } else { t.to_string() }
            } else {
                s.to_string()
            }
        }
        NType::Float(bytes) => {
            let s = v.trim();
            if *bytes == 4 { s.parse::<f32>().map(|f| format!("{f}")).unwrap_or_else(|_| s.to_string()) } else { s.parse::<f64>().map(|f| format!("{f}")).unwrap_or_else(|_| s.to_string()) }
        }
        NType::Uuid => v.trim().to_lowercase(),
        NType::Text(_) | NType::Json | NType::Other => v.to_string(),
    }
}

/// O valor como literal do banco de destino: número sem aspas, booleano, binário, e o resto como texto.
pub fn literal(d: Dialect, t: &NType, v: &Option<String>, from: &NType) -> String {
    let Some(raw) = v else { return "NULL".into() };
    let c = canonical(from, raw);
    match t {
        NType::Bool => match d {
            Dialect::Pg => if canonical(from, raw) == "1" { "true" } else { "false" }.into(),
            _ => if c == "1" { "1" } else { "0" }.into(),
        },
        NType::Binary => {
            let hex = canonical(&NType::Binary, raw);
            match d {
                Dialect::Pg => format!("'\\x{hex}'"),
                Dialect::Sqlite => format!("X'{hex}'"),
                _ => format!("0x{}", if hex.is_empty() { "00".into() } else { hex }),
            }
        }
        NType::Int(_) | NType::Decimal(_) | NType::Float(_) if is_plain_number(raw.trim()) => raw.trim().to_string(),
        NType::Timestamp(tz) => {
            let s = if *tz && matches!(from, NType::Timestamp(true)) { raw.replace('T', " ") } else { strip_offset(raw) };
            d.text(&s)
        }
        NType::Time => d.text(&strip_offset(raw)),
        NType::Date => d.text(&canonical(&NType::Date, raw)),
        _ => d.text(raw),
    }
}

// ---------------------------------------------------------------- DDL

/// O valor padrão de uma coluna escrito para o destino, ou `None` quando não dá para levá-lo (expressões do produto de origem).
pub fn default_sql(target: Product, t: &NType, d: &str) -> Option<String> {
    let mut s = d.trim().to_string();
    // `'x'::character varying`, `(0)`, `((1))`: tira a conversão e os parênteses de fora.
    if let Some(i) = s.find("::")
        && s.starts_with('\'')
    {
        s.truncate(i);
    }
    while s.len() > 2 && s.starts_with('(') && s.ends_with(')') {
        s = s[1..s.len() - 1].trim().to_string();
    }
    let low = s.to_lowercase();
    if low.starts_with("nextval(") || low.contains("identity") {
        return None;
    }
    if matches!(low.as_str(), "current_timestamp" | "now()" | "getdate()" | "sysdatetime()" | "current_timestamp()" | "localtimestamp") || low.starts_with("current_timestamp(") {
        return matches!(t, NType::Timestamp(_)).then(|| "CURRENT_TIMESTAMP".to_string());
    }
    let d = target.dialect();
    if matches!(t, NType::Bool) {
        let v = match low.as_str() {
            "true" | "1" | "b'1'" | "'t'" | "'true'" | "'1'" => "1",
            "false" | "0" | "b'0'" | "'f'" | "'false'" | "'0'" => "0",
            _ => return None,
        };
        return Some(if target == Product::Pg { if v == "1" { "true" } else { "false" }.into() } else { v.to_string() });
    }
    if is_plain_number(&s) {
        return Some(s);
    }
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') && !s[1..s.len() - 1].contains("''") && !matches!(t, NType::Binary | NType::Json | NType::Date | NType::Time | NType::Timestamp(_) | NType::Uuid) {
        return Some(d.text(&s[1..s.len() - 1]));
    }
    None
}

fn key_columns(t: &TableDetail) -> Vec<String> {
    let mut keys: Vec<(i32, &str)> = t.columns.iter().filter_map(|c| c.key.map(|k| (k, c.name.as_str()))).collect();
    keys.sort();
    keys.into_iter().map(|(_, n)| n.to_string()).collect()
}

fn split_fields(f: &str) -> Vec<String> {
    f.split(',').map(|x| x.trim().trim_matches(['"', '`', '[', ']']).to_string()).filter(|x| !x.is_empty()).collect()
}

fn simple_fields(f: &str) -> bool {
    !f.contains('(') && !f.contains(' ')
}

fn is_auto(c: &ColumnInfo) -> bool {
    c.identity.is_some() || c.default.as_deref().is_some_and(|d| d.to_lowercase().starts_with("nextval("))
}

/// A coluna como o destino a declara.
pub fn column_sql(src: Product, dst: Product, c: &ColumnInfo, pk_single_auto: bool) -> String {
    let d = dst.dialect();
    let t = parse(src, &c.formatted_type);
    let mut s = format!("{} ", d.q(&c.name));
    if dst == Product::Sqlite && pk_single_auto {
        s.push_str("INTEGER PRIMARY KEY AUTOINCREMENT");
        return s;
    }
    s.push_str(&render(dst, &t));
    let auto = is_auto(c);
    if auto {
        match dst {
            Product::Pg => s.push_str(" GENERATED BY DEFAULT AS IDENTITY"),
            Product::Mssql => s.push_str(" IDENTITY(1,1)"),
            Product::Mysql => s.push_str(" AUTO_INCREMENT"),
            Product::Sqlite => {}
        }
    }
    s.push_str(if c.not_null || auto { " NOT NULL" } else if dst == Product::Mssql { " NULL" } else { "" });
    if !auto && let Some(def) = c.default.as_deref().and_then(|x| default_sql(dst, &t, x)) {
        s.push_str(&if dst == Product::Mssql { format!(" DEFAULT ({def})") } else { format!(" DEFAULT {def}") });
    }
    s
}

/// A tabela de origem como `CREATE TABLE` (e o que vem depois dos dados: chaves estrangeiras e índices) no produto de destino.
/// Devolve também os avisos do que não atravessou.
pub fn create_table(src: Product, dst: Product, schema: &str, table: &str, detail: &TableDetail) -> (Vec<String>, Vec<String>, Vec<String>) {
    let d = dst.dialect();
    let target = d.target(schema, table);
    let pk = key_columns(detail);
    let single_auto = dst == Product::Sqlite && pk.len() == 1 && detail.columns.iter().any(|c| c.name == pk[0] && is_auto(c) && matches!(parse(src, &c.formatted_type), NType::Int(_)));
    let mut lines: Vec<String> = detail.columns.iter().filter(|c| c.generated.is_none()).map(|c| column_sql(src, dst, c, single_auto && c.name == pk[0])).collect();
    let mut warnings = Vec::new();
    if !pk.is_empty() && !single_auto {
        lines.push(format!("PRIMARY KEY ({})", pk.iter().map(|k| d.q(k)).collect::<Vec<_>>().join(", ")));
    }
    // O MySQL só numera uma coluna que seja chave.
    let mut post = Vec::new();
    let mut seen_unique: Vec<Vec<String>> = Vec::new();
    for c in detail.constraints.iter().filter(|c| c.kind == "u") {
        let cols = split_fields(&c.fields);
        seen_unique.push(cols.clone());
        lines.push(format!("UNIQUE ({})", cols.iter().map(|k| d.q(k)).collect::<Vec<_>>().join(", ")));
    }
    for c in detail.constraints.iter().filter(|c| c.kind == "c") {
        warnings.push(format!("{table}: check constraint {} was not carried over (the expression belongs to the source product)", c.name));
    }
    for c in detail.constraints.iter().filter(|c| c.kind == "f") {
        let rt = c.ref_table.clone().unwrap_or_default();
        let mut fk = format!(
            "FOREIGN KEY ({}) REFERENCES {} ({})",
            split_fields(&c.fields).iter().map(|k| d.q(k)).collect::<Vec<_>>().join(", "),
            d.target(schema, &rt),
            split_fields(c.ref_fields.as_deref().unwrap_or("")).iter().map(|k| d.q(k)).collect::<Vec<_>>().join(", ")
        );
        if let Some(a) = c.on_delete.as_deref().filter(|a| !matches!(*a, "NO ACTION" | "RESTRICT" | "")) {
            fk.push_str(&format!(" ON DELETE {a}"));
        }
        if let Some(a) = c.on_update.as_deref().filter(|a| !matches!(*a, "NO ACTION" | "RESTRICT" | "")) {
            fk.push_str(&format!(" ON UPDATE {a}"));
        }
        if dst == Product::Sqlite {
            lines.push(fk);
        } else {
            post.push(format!("ALTER TABLE {target} ADD CONSTRAINT {} {fk}", d.q(&c.name)));
        }
    }
    let create = format!("CREATE TABLE {target} (\n  {}\n)", lines.join(",\n  "));
    for i in detail.indexes.iter().filter(|i| !i.primary) {
        let cols = split_fields(&i.fields);
        if !simple_fields(&i.fields.replace(", ", ",")) && i.fields.contains('(') {
            warnings.push(format!("{table}: index {} was not carried over (expression index)", i.name));
            continue;
        }
        if seen_unique.contains(&cols) || detail.constraints.iter().any(|c| c.name == i.name && c.kind == "u") {
            continue;
        }
        let name = if matches!(dst, Product::Pg | Product::Sqlite) && !i.name.starts_with(table) { format!("{table}_{}", i.name) } else { i.name.clone() };
        post.push(format!("CREATE {}INDEX {} ON {target} ({})", if i.unique { "UNIQUE " } else { "" }, d.q(&name), cols.iter().map(|k| d.q(k)).collect::<Vec<_>>().join(", ")));
    }
    (vec![create], post, warnings)
}

/// A restrição de chave estrangeira e o índice têm o mesmo sentido nos dois lados? (para a comparação entre produtos)
#[allow(dead_code)]
pub fn same_fk(a: &ConstraintInfo, b: &ConstraintInfo) -> bool {
    split_fields(&a.fields) == split_fields(&b.fields) && a.ref_table == b.ref_table && split_fields(a.ref_fields.as_deref().unwrap_or("")) == split_fields(b.ref_fields.as_deref().unwrap_or(""))
}

#[allow(dead_code)]
pub fn same_index(a: &IndexInfo, b: &IndexInfo) -> bool {
    split_fields(&a.fields) == split_fields(&b.fields) && a.unique == b.unique
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_round_trip_between_products() {
        assert_eq!(parse(Product::Pg, "character varying(50)"), NType::Text(Some(50)));
        assert_eq!(render(Product::Mssql, &parse(Product::Pg, "character varying(50)")), "nvarchar(50)");
        assert_eq!(render(Product::Pg, &parse(Product::Mysql, "tinyint(1)")), "boolean");
        assert_eq!(render(Product::Mysql, &parse(Product::Mssql, "uniqueidentifier")), "char(36)");
        assert_eq!(render(Product::Pg, &parse(Product::Mssql, "nvarchar(max)")), "text");
        assert_eq!(render(Product::Sqlite, &parse(Product::Pg, "numeric(10,2)")), "numeric");
        assert_eq!(render(Product::Mysql, &parse(Product::Pg, "timestamp with time zone")), "datetime(6)");
    }

    #[test]
    fn values_get_a_canonical_form() {
        assert_eq!(canonical(&NType::Bool, "t"), "1");
        assert_eq!(canonical(&NType::Bool, "false"), "0");
        assert_eq!(canonical(&NType::Binary, "\\xAB01"), "ab01");
        assert_eq!(canonical(&NType::Timestamp(false), "2020-01-02T03:04:05.100+00"), "2020-01-02 03:04:05.1");
        assert_eq!(canonical(&NType::Decimal(None), "1.50"), "1.5");
        assert_eq!(canonical(&NType::Decimal(None), "10"), "10");
    }

    #[test]
    fn literals_follow_the_target() {
        let b = NType::Bool;
        assert_eq!(literal(Dialect::Pg, &b, &Some("1".into()), &b), "true");
        assert_eq!(literal(Dialect::Mysql, &b, &Some("t".into()), &b), "1");
        assert_eq!(literal(Dialect::Mssql, &NType::Text(None), &Some("it's".into()), &NType::Text(None)), "N'it''s'");
        assert_eq!(literal(Dialect::Sqlite, &NType::Binary, &Some("\\x0a".into()), &NType::Binary), "X'0a'");
        assert_eq!(default_sql(Product::Mysql, &NType::Text(None), "'abc'::text").as_deref(), Some("'abc'"));
        assert_eq!(default_sql(Product::Mssql, &NType::Int(4), "((0))").as_deref(), Some("0"));
        assert_eq!(default_sql(Product::Pg, &NType::Int(4), "nextval('t_id_seq'::regclass)"), None);
    }
}
