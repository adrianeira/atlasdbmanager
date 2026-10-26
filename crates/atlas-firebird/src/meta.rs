//! Estrutura do banco, lida das tabelas de sistema `RDB$` (valem do Firebird 2.5 ao 5; o que só existe nas versões novas é pedido
//! conforme a versão do servidor).

use crate::{Conn, map_err, quote_ident, values};
use atlas_core::{ColumnInfo, ConstraintInfo, DatabaseError, ErrorKind, IndexInfo, ObjectInfo, SchemaObjects, TableDetail, TriggerInfo, ViewDetail};
use rsfbclient::{Queryable, Row, SqlType};

/// Resultado de uma consulta ao catálogo: cada linha com as células em texto (`None` = NULL).
pub(crate) type Cells = Vec<Option<String>>;

pub(crate) fn query(conn: &mut Conn, sql: &str, params: Vec<SqlType>) -> Result<Vec<Cells>, DatabaseError> {
    let rows: Vec<Row> = conn.query(sql, params).map_err(map_err)?;
    Ok(rows.iter().map(|r| r.cols.iter().map(values::format).collect()).collect())
}

pub(crate) fn text(name: &str) -> Vec<SqlType> {
    vec![SqlType::Text(name.to_string())]
}

fn s(c: &Cells, i: usize) -> String {
    c.get(i).cloned().flatten().unwrap_or_default().trim_end().to_string()
}

fn opt(c: &Cells, i: usize) -> Option<String> {
    c.get(i).cloned().flatten().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn n(c: &Cells, i: usize) -> i32 {
    s(c, i).parse().unwrap_or(0)
}

/// Os recursos novos do catálogo (identidade, pacotes, funções em PSQL) existem a partir do Firebird 3.
pub(crate) fn is_v3(conn: &mut Conn) -> bool {
    version_major(conn) >= 3
}

fn version_major(conn: &mut Conn) -> i32 {
    query(conn, "SELECT RDB$GET_CONTEXT('SYSTEM', 'ENGINE_VERSION') FROM RDB$DATABASE", vec![])
        .ok()
        .and_then(|r| r.first().map(|c| s(c, 0)))
        .and_then(|v| v.split('.').next().and_then(|m| m.parse().ok()))
        .unwrap_or(2)
}

/* ---------- Lista de objetos ---------- */

pub fn list_objects_on(conn: &mut Conn) -> Result<SchemaObjects, DatabaseError> {
    let mut out = SchemaObjects::default();
    let v3 = is_v3(conn);
    for r in query(
        conn,
        "SELECT TRIM(RDB$RELATION_NAME), RDB$DESCRIPTION, TRIM(RDB$OWNER_NAME), CASE WHEN RDB$VIEW_BLR IS NULL THEN 0 ELSE 1 END FROM RDB$RELATIONS WHERE COALESCE(RDB$SYSTEM_FLAG, 0) = 0 ORDER BY 1",
        vec![],
    )? {
        let info = ObjectInfo { name: s(&r, 0), comment: opt(&r, 1), owner: opt(&r, 2), ..Default::default() };
        if n(&r, 3) == 1 { out.views.push(info) } else { out.tables.push(info) }
    }
    let procs = if v3 {
        "SELECT TRIM(RDB$PROCEDURE_NAME), RDB$DESCRIPTION, TRIM(RDB$OWNER_NAME) FROM RDB$PROCEDURES WHERE COALESCE(RDB$SYSTEM_FLAG, 0) = 0 AND RDB$PACKAGE_NAME IS NULL ORDER BY 1"
    } else {
        "SELECT TRIM(RDB$PROCEDURE_NAME), RDB$DESCRIPTION, TRIM(RDB$OWNER_NAME) FROM RDB$PROCEDURES WHERE COALESCE(RDB$SYSTEM_FLAG, 0) = 0 ORDER BY 1"
    };
    for r in query(conn, procs, vec![])? {
        out.functions.push(ObjectInfo { name: s(&r, 0), comment: opt(&r, 1), owner: opt(&r, 2), procedure: true, ..Default::default() });
    }
    if v3 {
        for r in query(
            conn,
            "SELECT TRIM(RDB$FUNCTION_NAME), RDB$DESCRIPTION, TRIM(RDB$OWNER_NAME) FROM RDB$FUNCTIONS WHERE COALESCE(RDB$SYSTEM_FLAG, 0) = 0 AND RDB$PACKAGE_NAME IS NULL AND RDB$MODULE_NAME IS NULL ORDER BY 1",
            vec![],
        )? {
            out.functions.push(ObjectInfo { name: s(&r, 0), comment: opt(&r, 1), owner: opt(&r, 2), procedure: false, ..Default::default() });
        }
    }
    // Os gatilhos de tabela entram na lista das rotinas (o designer de rotinas edita o texto do `CREATE OR ALTER TRIGGER`).
    for r in query(conn, "SELECT TRIM(RDB$TRIGGER_NAME), RDB$DESCRIPTION, TRIM(RDB$RELATION_NAME) FROM RDB$TRIGGERS WHERE COALESCE(RDB$SYSTEM_FLAG, 0) = 0 AND RDB$RELATION_NAME IS NOT NULL ORDER BY 1", vec![])? {
        out.functions.push(ObjectInfo { name: s(&r, 0), comment: opt(&r, 1), args: Some(format!("table {}", s(&r, 2))), return_type: Some("trigger".into()), procedure: false, ..Default::default() });
    }
    for r in query(conn, "SELECT TRIM(RDB$GENERATOR_NAME), RDB$DESCRIPTION FROM RDB$GENERATORS WHERE COALESCE(RDB$SYSTEM_FLAG, 0) = 0 ORDER BY 1", vec![])? {
        out.sequences.push(ObjectInfo { name: s(&r, 0), comment: opt(&r, 1), ..Default::default() });
    }
    Ok(out)
}

/* ---------- Tipos ---------- */

/// (nome base, tipo formatado, tamanho, casas decimais) de uma coluna a partir do catálogo, em minúsculas como os outros bancos.
pub fn describe_type(field_type: i32, sub_type: i32, length: i32, char_length: i32, precision: i32, scale: i32) -> (String, String, i32, i32) {
    let numeric = |base_precision: i32| -> (String, String, i32, i32) {
        let p = if precision > 0 { precision } else { base_precision };
        let name = if sub_type == 2 { "decimal" } else { "numeric" };
        (name.to_string(), format!("{name}({p},{})", -scale), p, -scale)
    };
    let plain = |name: &str| (name.to_string(), name.to_string(), 0, 0);
    match field_type {
        7 if scale != 0 || sub_type != 0 => numeric(4),
        8 if scale != 0 || sub_type != 0 => numeric(9),
        16 if scale != 0 || sub_type != 0 => numeric(18),
        26 if scale != 0 || sub_type != 0 => numeric(38),
        7 => plain("smallint"),
        8 => plain("integer"),
        16 => plain("bigint"),
        26 => plain("int128"),
        10 => plain("float"),
        27 => plain("double precision"),
        12 => plain("date"),
        13 => plain("time"),
        35 => plain("timestamp"),
        23 => plain("boolean"),
        24 => ("decfloat".into(), "decfloat(16)".into(), 16, 0),
        25 => ("decfloat".into(), "decfloat(34)".into(), 34, 0),
        28 => plain("time with time zone"),
        29 => plain("timestamp with time zone"),
        14 | 37 => {
            let size = if char_length > 0 { char_length } else { length };
            let base = if field_type == 14 { "char" } else { "varchar" };
            (base.to_string(), format!("{base}({size})"), size, 0)
        }
        261 => {
            let name = if sub_type == 1 { "blob sub_type text".to_string() } else if sub_type == 0 { "blob".to_string() } else { format!("blob sub_type {sub_type}") };
            (name.clone(), name, 0, 0)
        }
        _ => plain("unknown"),
    }
}

/* ---------- Tabela ---------- */

fn trigger_events(kind: i32) -> (String, Vec<String>) {
    let timing = if kind & 1 == 1 { "BEFORE" } else { "AFTER" };
    let code = kind + 1;
    let names = ["", "INSERT", "UPDATE", "DELETE"];
    let events: Vec<String> = [(code >> 1) & 3, (code >> 3) & 3, (code >> 5) & 3].into_iter().filter(|e| *e > 0).map(|e| names[e as usize].to_string()).collect();
    (timing.to_string(), events)
}

/// O texto que recria um gatilho (`CREATE OR ALTER TRIGGER`). O texto guardado já começa com `AS` (o 2.5 e o 3 o guardam assim).
pub fn trigger_definition(name: &str, table: &str, kind: i32, enabled: bool, position: i32, source: &str) -> String {
    let (timing, events) = trigger_events(kind);
    let body = if crate::split::first_word(source) == "AS" { source.trim().to_string() } else { format!("AS\n{}", source.trim()) };
    format!(
        "CREATE OR ALTER TRIGGER {} FOR {} {} {} {} POSITION {position}\n{body}",
        quote_ident(name),
        quote_ident(table),
        if enabled { "ACTIVE" } else { "INACTIVE" },
        timing,
        events.join(" OR ")
    )
}

fn index_fields(conn: &mut Conn, index: &str) -> Result<Vec<String>, DatabaseError> {
    Ok(query(conn, "SELECT TRIM(RDB$FIELD_NAME) FROM RDB$INDEX_SEGMENTS WHERE RDB$INDEX_NAME = ? ORDER BY RDB$FIELD_POSITION", text(index))?.iter().map(|r| s(r, 0)).collect())
}

pub fn table_detail_on(conn: &mut Conn, table: &str) -> Result<TableDetail, DatabaseError> {
    let v3 = is_v3(conn);
    let head = query(conn, "SELECT TRIM(RDB$OWNER_NAME), RDB$DESCRIPTION FROM RDB$RELATIONS WHERE RDB$RELATION_NAME = ?", text(table))?;
    let Some(head) = head.first() else {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, format!("table \"{table}\" does not exist")));
    };
    let (owner, comment) = (opt(head, 0), opt(head, 1));

    // Chave primária: posição de cada coluna.
    let pk_index = query(conn, "SELECT TRIM(RDB$INDEX_NAME) FROM RDB$RELATION_CONSTRAINTS WHERE RDB$RELATION_NAME = ? AND RDB$CONSTRAINT_TYPE = 'PRIMARY KEY'", text(table))?;
    let pk_fields = match pk_index.first() {
        Some(r) => index_fields(conn, &s(r, 0))?,
        None => Vec::new(),
    };

    let identity_col = if v3 { "rf.RDB$IDENTITY_TYPE" } else { "CAST(NULL AS SMALLINT)" };
    let sql = format!(
        "SELECT TRIM(rf.RDB$FIELD_NAME), f.RDB$FIELD_TYPE, COALESCE(f.RDB$FIELD_SUB_TYPE, 0), COALESCE(f.RDB$FIELD_LENGTH, 0), COALESCE(f.RDB$CHARACTER_LENGTH, 0), COALESCE(f.RDB$FIELD_PRECISION, 0), \
         COALESCE(f.RDB$FIELD_SCALE, 0), COALESCE(rf.RDB$NULL_FLAG, f.RDB$NULL_FLAG, 0), COALESCE(rf.RDB$DEFAULT_SOURCE, f.RDB$DEFAULT_SOURCE), rf.RDB$DESCRIPTION, f.RDB$COMPUTED_SOURCE, {identity_col} \
         FROM RDB$RELATION_FIELDS rf JOIN RDB$FIELDS f ON f.RDB$FIELD_NAME = rf.RDB$FIELD_SOURCE WHERE rf.RDB$RELATION_NAME = ? ORDER BY rf.RDB$FIELD_POSITION"
    );
    let mut columns = Vec::new();
    for r in query(conn, &sql, text(table))? {
        let (base, formatted, length, decimal) = describe_type(n(&r, 1), n(&r, 2), n(&r, 3), n(&r, 4), n(&r, 5), n(&r, 6));
        let default = opt(&r, 8).map(|d| d.strip_prefix("DEFAULT ").or_else(|| d.strip_prefix("default ")).unwrap_or(&d).trim().to_string());
        let name = s(&r, 0);
        let computed = opt(&r, 10);
        columns.push(ColumnInfo {
            key: pk_fields.iter().position(|f| *f == name).map(|p| p as i32 + 1),
            name,
            data_type: base,
            formatted_type: formatted,
            length,
            decimal,
            not_null: n(&r, 7) == 1,
            // Coluna calculada: o `default` leva a expressão (`COMPUTED BY`).
            default: if computed.is_some() { computed.clone() } else { default },
            comment: opt(&r, 9),
            collation: None,
            // `RDB$IDENTITY_TYPE`: 0 = GENERATED ALWAYS, 1 = GENERATED BY DEFAULT.
            identity: opt(&r, 11).and_then(|t| match t.as_str() {
                "0" => Some("a".to_string()),
                "1" => Some("d".to_string()),
                _ => None,
            }),
            generated: computed.map(|_| "s".to_string()),
            auto_source: None,
        });
    }

    // Os índices que sustentam chaves e restrições aparecem como restrições, não como índices.
    let constraint_rows = query(conn, "SELECT TRIM(RDB$CONSTRAINT_NAME), TRIM(RDB$CONSTRAINT_TYPE), TRIM(RDB$INDEX_NAME) FROM RDB$RELATION_CONSTRAINTS WHERE RDB$RELATION_NAME = ? ORDER BY RDB$CONSTRAINT_NAME", text(table))?;
    let backing: Vec<String> = constraint_rows.iter().map(|r| s(r, 2)).filter(|n| !n.is_empty()).collect();
    let mut indexes = Vec::new();
    let mut index_rows = query(
        conn,
        "SELECT TRIM(RDB$INDEX_NAME), COALESCE(RDB$UNIQUE_FLAG, 0), COALESCE(RDB$INDEX_INACTIVE, 0), COALESCE(RDB$INDEX_TYPE, 0), RDB$EXPRESSION_SOURCE, RDB$DESCRIPTION FROM RDB$INDICES WHERE RDB$RELATION_NAME = ? ORDER BY RDB$INDEX_NAME",
        text(table),
    )?;
    let pk_name = pk_index.first().map(|r| s(r, 0));
    for r in index_rows.drain(..) {
        let name = s(&r, 0);
        if backing.contains(&name) {
            continue;
        }
        let fields = index_fields(conn, &name)?;
        let expression = opt(&r, 4);
        let joined = match &expression {
            Some(e) => format!("({e})"),
            None => fields.iter().map(|f| quote_ident(f)).collect::<Vec<_>>().join(", "),
        };
        let unique = n(&r, 1) == 1;
        let descending = n(&r, 3) == 1;
        let definition = format!("CREATE {}{}INDEX {} ON {} {}", if unique { "UNIQUE " } else { "" }, if descending { "DESCENDING " } else { "" }, quote_ident(&name), quote_ident(table), if expression.is_some() { format!("COMPUTED BY {joined}") } else { format!("({joined})") });
        indexes.push(IndexInfo { primary: pk_name.as_deref() == Some(name.as_str()), name, fields: if expression.is_some() { joined.clone() } else { fields.join(", ") }, method: if descending { "btree desc".into() } else { "btree".into() }, unique, definition, comment: opt(&r, 5) });
    }

    let mut constraints = Vec::new();
    for r in constraint_rows {
        let (name, kind) = (s(&r, 0), s(&r, 1));
        let index = s(&r, 2);
        match kind.as_str() {
            "PRIMARY KEY" => {
                let fields = index_fields(conn, &index)?.join(", ");
                constraints.push(ConstraintInfo { name, kind: "p".into(), definition: format!("PRIMARY KEY ({fields})"), fields, ref_schema: None, ref_table: None, ref_fields: None, on_delete: None, on_update: None, comment: None });
            }
            "UNIQUE" => {
                let fields = index_fields(conn, &index)?.join(", ");
                constraints.push(ConstraintInfo { name, kind: "u".into(), definition: format!("UNIQUE ({fields})"), fields, ref_schema: None, ref_table: None, ref_fields: None, on_delete: None, on_update: None, comment: None });
            }
            "FOREIGN KEY" => {
                let fields = index_fields(conn, &index)?.join(", ");
                let rule = query(conn, "SELECT TRIM(RDB$CONST_NAME_UQ), TRIM(RDB$UPDATE_RULE), TRIM(RDB$DELETE_RULE) FROM RDB$REF_CONSTRAINTS WHERE RDB$CONSTRAINT_NAME = ?", text(&name))?;
                let Some(rule) = rule.first() else { continue };
                let target = query(conn, "SELECT TRIM(RDB$RELATION_NAME), TRIM(RDB$INDEX_NAME) FROM RDB$RELATION_CONSTRAINTS WHERE RDB$CONSTRAINT_NAME = ?", text(&s(rule, 0)))?;
                let Some(target) = target.first() else { continue };
                let ref_table = s(target, 0);
                let ref_fields = index_fields(conn, &s(target, 1))?.join(", ");
                let (on_update, on_delete) = (s(rule, 1), s(rule, 2));
                constraints.push(ConstraintInfo {
                    definition: format!("FOREIGN KEY ({fields}) REFERENCES {} ({ref_fields}) ON UPDATE {on_update} ON DELETE {on_delete}", quote_ident(&ref_table)),
                    name,
                    kind: "f".into(),
                    fields,
                    ref_schema: Some("main".into()),
                    ref_table: Some(ref_table),
                    ref_fields: Some(ref_fields),
                    on_delete: Some(on_delete),
                    on_update: Some(on_update),
                    comment: None,
                });
            }
            "CHECK" => {
                let src = query(conn, "SELECT t.RDB$TRIGGER_SOURCE FROM RDB$CHECK_CONSTRAINTS c JOIN RDB$TRIGGERS t ON t.RDB$TRIGGER_NAME = c.RDB$TRIGGER_NAME WHERE c.RDB$CONSTRAINT_NAME = ? ROWS 1", text(&name))?;
                let definition = src.first().map(|r| s(r, 0)).unwrap_or_default();
                constraints.push(ConstraintInfo { name, kind: "c".into(), fields: String::new(), ref_schema: None, ref_table: None, ref_fields: None, on_delete: None, on_update: None, definition, comment: None });
            }
            _ => {}
        }
    }

    let mut triggers = Vec::new();
    // Auto incremento feito por gerador + gatilho `BEFORE INSERT` (o único jeito no 2.5 e em colunas que já existiam): (coluna, gatilho, gerador).
    let mut auto: Vec<(String, String, String)> = Vec::new();
    for r in query(
        conn,
        "SELECT TRIM(RDB$TRIGGER_NAME), RDB$TRIGGER_TYPE, COALESCE(RDB$TRIGGER_INACTIVE, 0), RDB$TRIGGER_SOURCE, COALESCE(RDB$TRIGGER_SEQUENCE, 0), RDB$DESCRIPTION FROM RDB$TRIGGERS WHERE RDB$RELATION_NAME = ? AND COALESCE(RDB$SYSTEM_FLAG, 0) = 0 ORDER BY RDB$TRIGGER_SEQUENCE, RDB$TRIGGER_NAME",
        text(table),
    )? {
        let (timing, events) = trigger_events(n(&r, 1));
        let name = s(&r, 0);
        if timing == "BEFORE" && events.iter().any(|e| e == "INSERT") {
            if let Some((col, generator)) = auto_numbering(&s(&r, 3)) {
                auto.push((col, name.clone(), generator));
            }
        }
        let enabled = n(&r, 2) == 0;
        let definition = trigger_definition(&name, table, n(&r, 1), enabled, n(&r, 4), &s(&r, 3));
        triggers.push(TriggerInfo { name, definition, enabled, timing, events, for_each_row: true, when: None, function: String::new(), args: Vec::new(), update_columns: Vec::new(), comment: opt(&r, 5), read_only: true });
    }
    for (col, trigger, generator) in auto {
        if let Some(c) = columns.iter_mut().find(|c| c.name.eq_ignore_ascii_case(&col)) {
            c.identity.get_or_insert_with(|| "d".to_string());
            c.auto_source = Some(format!("{trigger}|{generator}"));
        }
    }
    Ok(TableDetail { columns, indexes, constraints, triggers, owner, comment })
}

/// O corpo de um gatilho que numera uma coluna: `NEW.col = GEN_ID(gerador, 1)` ou `NEW.col = NEXT VALUE FOR gerador` → (coluna, gerador).
fn auto_numbering(source: &str) -> Option<(String, String)> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r#"(?is)\bNEW\s*\.\s*"?([A-Za-z0-9_$]+)"?\s*=\s*(?:GEN_ID\s*\(\s*"?([A-Za-z0-9_$]+)"?\s*,\s*1\s*\)|NEXT\s+VALUE\s+FOR\s+"?([A-Za-z0-9_$]+)"?)"#).expect("valid regex")
    });
    let c = re.captures(source)?;
    let generator = c.get(2).or_else(|| c.get(3))?.as_str().to_string();
    Some((c[1].to_string(), generator))
}

/* ---------- Rotinas ---------- */

/// Parâmetros de um procedimento ou função (primeiro valor: tipo/posição), em ordem, como `"NOME" tipo`.
fn routine_params(conn: &mut Conn, sql: &str, name: &str) -> Result<Vec<(i32, String)>, DatabaseError> {
    let mut out = Vec::new();
    for r in query(conn, sql, text(name))? {
        let (base, formatted, _, _) = describe_type(n(&r, 2), n(&r, 3), n(&r, 4), n(&r, 5), n(&r, 6), n(&r, 7));
        let ty = if formatted.is_empty() { base } else { formatted };
        let mut line = format!("{} {}", quote_ident(&s(&r, 1)), ty);
        if n(&r, 8) == 1 {
            line.push_str(" NOT NULL");
        }
        out.push((n(&r, 0), line));
    }
    Ok(out)
}

fn body_of(source: &str) -> String {
    source.trim_matches(|c| c == '\n' || c == '\r').to_string()
}

/// O texto que recria um procedimento (`CREATE OR ALTER PROCEDURE`) ou, no Firebird 3 em diante, uma função.
pub fn routine_definition_on(conn: &mut Conn, name: &str) -> Result<String, DatabaseError> {
    let head = query(conn, "SELECT RDB$PROCEDURE_SOURCE FROM RDB$PROCEDURES WHERE RDB$PROCEDURE_NAME = ?", text(name))?;
    if let Some(r) = head.first() {
        let params = routine_params(
            conn,
            "SELECT p.RDB$PARAMETER_TYPE, TRIM(p.RDB$PARAMETER_NAME), f.RDB$FIELD_TYPE, COALESCE(f.RDB$FIELD_SUB_TYPE, 0), COALESCE(f.RDB$FIELD_LENGTH, 0), COALESCE(f.RDB$CHARACTER_LENGTH, 0), COALESCE(f.RDB$FIELD_PRECISION, 0), COALESCE(f.RDB$FIELD_SCALE, 0), COALESCE(p.RDB$NULL_FLAG, 0) \
             FROM RDB$PROCEDURE_PARAMETERS p JOIN RDB$FIELDS f ON f.RDB$FIELD_NAME = p.RDB$FIELD_SOURCE WHERE p.RDB$PROCEDURE_NAME = ? ORDER BY p.RDB$PARAMETER_TYPE, p.RDB$PARAMETER_NUMBER",
            name,
        )?;
        let ins: Vec<&str> = params.iter().filter(|p| p.0 == 0).map(|p| p.1.as_str()).collect();
        let outs: Vec<&str> = params.iter().filter(|p| p.0 == 1).map(|p| p.1.as_str()).collect();
        let mut out = format!("CREATE OR ALTER PROCEDURE {}", quote_ident(name));
        if !ins.is_empty() {
            out.push_str(&format!(" (\n  {}\n)", ins.join(",\n  ")));
        }
        if !outs.is_empty() {
            out.push_str(&format!("\nRETURNS (\n  {}\n)", outs.join(",\n  ")));
        }
        out.push_str(&format!("\nAS\n{}", body_of(&s(r, 0))));
        return Ok(out);
    }
    if is_v3(conn) {
        let head = query(conn, "SELECT RDB$FUNCTION_SOURCE, RDB$RETURN_ARGUMENT FROM RDB$FUNCTIONS WHERE RDB$FUNCTION_NAME = ? AND RDB$PACKAGE_NAME IS NULL", text(name))?;
        if let Some(r) = head.first() {
            let ret = n(r, 1);
            let args = routine_params(
                conn,
                "SELECT a.RDB$ARGUMENT_POSITION, TRIM(COALESCE(a.RDB$ARGUMENT_NAME, 'RETURN_VALUE')), f.RDB$FIELD_TYPE, COALESCE(f.RDB$FIELD_SUB_TYPE, 0), COALESCE(f.RDB$FIELD_LENGTH, 0), COALESCE(f.RDB$CHARACTER_LENGTH, 0), COALESCE(f.RDB$FIELD_PRECISION, 0), COALESCE(f.RDB$FIELD_SCALE, 0), COALESCE(a.RDB$NULL_FLAG, 0) \
                 FROM RDB$FUNCTION_ARGUMENTS a JOIN RDB$FIELDS f ON f.RDB$FIELD_NAME = a.RDB$FIELD_SOURCE WHERE a.RDB$FUNCTION_NAME = ? ORDER BY a.RDB$ARGUMENT_POSITION",
                name,
            )?;
            let ins: Vec<&str> = args.iter().filter(|p| p.0 != ret).map(|p| p.1.as_str()).collect();
            let returns = args.iter().find(|p| p.0 == ret).map(|p| p.1.split_once(' ').map_or(p.1.clone(), |(_, t)| t.to_string())).unwrap_or_else(|| "INTEGER".into());
            let mut out = format!("CREATE OR ALTER FUNCTION {}", quote_ident(name));
            if !ins.is_empty() {
                out.push_str(&format!(" (\n  {}\n)", ins.join(",\n  ")));
            }
            out.push_str(&format!("\nRETURNS {returns}\nAS\n{}", body_of(&s(r, 0))));
            return Ok(out);
        }
    }
    let t = query(conn, "SELECT TRIM(RDB$TRIGGER_NAME), TRIM(RDB$RELATION_NAME), RDB$TRIGGER_TYPE, COALESCE(RDB$TRIGGER_INACTIVE, 0), RDB$TRIGGER_SOURCE, COALESCE(RDB$TRIGGER_SEQUENCE, 0) FROM RDB$TRIGGERS WHERE RDB$TRIGGER_NAME = ? AND RDB$RELATION_NAME IS NOT NULL", text(name))?;
    if let Some(r) = t.first() {
        return Ok(trigger_definition(&s(r, 0), &s(r, 1), n(r, 2), n(r, 3) == 0, n(r, 5), &s(r, 4)));
    }
    Err(DatabaseError::new(ErrorKind::InvalidInput, format!("routine \"{name}\" does not exist")))
}

/* ---------- View ---------- */

pub fn view_detail_on(conn: &mut Conn, view: &str) -> Result<ViewDetail, DatabaseError> {
    let rows = query(conn, "SELECT RDB$VIEW_SOURCE, TRIM(RDB$OWNER_NAME), RDB$DESCRIPTION FROM RDB$RELATIONS WHERE RDB$RELATION_NAME = ? AND RDB$VIEW_BLR IS NOT NULL", text(view))?;
    let Some(r) = rows.first() else {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, format!("view \"{view}\" does not exist")));
    };
    Ok(ViewDetail { definition: s(r, 0).trim().trim_end_matches(';').to_string(), owner: opt(r, 1), comment: opt(r, 2), check_option: None, security_barrier: false, rules: Vec::new(), populated: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbering_trigger_is_recognized() {
        let src = "AS\nBEGIN\n  IF (NEW.ID IS NULL) THEN\n    NEW.ID = GEN_ID(G_X, 1);\nEND";
        assert_eq!(auto_numbering(src), Some(("ID".to_string(), "G_X".to_string())));
        let quoted = "BEGIN NEW.\"Id\" = GEN_ID(\"gen_t\", 1); END";
        assert_eq!(auto_numbering(quoted), Some(("Id".to_string(), "gen_t".to_string())));
        assert_eq!(auto_numbering("BEGIN NEW.A = NEXT VALUE FOR SEQ_A; END"), Some(("A".to_string(), "SEQ_A".to_string())));
        assert_eq!(auto_numbering("BEGIN NEW.A = 1; END"), None);
    }
}
