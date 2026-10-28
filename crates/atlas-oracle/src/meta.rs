//! Estrutura do servidor: serviço (o "banco"), schemas (os usuários), tabelas, views, rotinas, sequências e o detalhe de uma tabela
//! (colunas, índices, chaves, gatilhos), lidos dos catálogos `ALL_*`. Só mostram o que o login enxerga.

use super::{OracleSession, map_err};
use atlas_core::{ColumnInfo, ConstraintInfo, DatabaseError, DatabaseList, DiagramColumn, DiagramRelation, DiagramTable, ErrorKind, IndexInfo, ObjectInfo, RoleInfo, SchemaDiagram, SchemaInfo, SchemaObjects, TableDetail, TriggerInfo, ViewDetail};
use oracle_rs::{Connection, Row, Value};
use std::collections::BTreeMap;

pub(crate) fn text(r: &Row, i: usize) -> Option<String> {
    match r.get(i)? {
        Value::Null => None,
        other => Some(other.to_string()),
    }
}
pub(crate) fn s(r: &Row, i: usize) -> String {
    text(r, i).unwrap_or_default()
}
pub(crate) fn n(r: &Row, i: usize) -> i64 {
    text(r, i).and_then(|v| v.parse::<f64>().ok()).map(|f| f as i64).unwrap_or(0)
}
fn v(s: &str) -> Value {
    Value::String(s.to_string())
}

pub(crate) async fn rows_of(conn: &Connection, sql: &str, params: &[Value]) -> Result<Vec<Row>, DatabaseError> {
    Ok(conn.query(sql, params).await.map_err(map_err)?.rows)
}

/// Os usuários que o próprio Oracle mantém (`SYS`, `SYSTEM`, `XDB`...) ficam escondidos até se pedir para mostrá-los.
const USERS_SQL: &str = "SELECT username, 'N' FROM all_users WHERE oracle_maintained = 'N' OR username = SYS_CONTEXT('USERENV', 'CURRENT_USER') ORDER BY username";
const ALL_USERS_SQL: &str = "SELECT username, oracle_maintained FROM all_users ORDER BY username";

fn xml_unescape(t: &str) -> String {
    t.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// O texto entre `<tag>` e `</tag>` dentro de `xml`.
fn xml_tag<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{tag}>"))? + start;
    Some(&xml[start..end])
}

/// Os valores padrão das colunas (`DATA_DEFAULT` é `LONG`, que o driver não lê): o `DBMS_XMLGEN` os converte em texto, em qualquer versão.
async fn defaults_via_xml(conn: &Connection, schema: &str, table: &str) -> BTreeMap<String, String> {
    let inner = format!("SELECT column_name, data_default FROM all_tab_cols WHERE owner = '{}' AND table_name = '{}' AND hidden_column = 'NO'", schema.replace('\'', "''"), table.replace('\'', "''"));
    let sql = format!("SELECT DBMS_XMLGEN.GETXML('{}') FROM dual", inner.replace('\'', "''"));
    let mut out = BTreeMap::new();
    let Ok(res) = conn.query(&sql, &[]).await else { return out };
    let (Some(col), Some(row)) = (res.columns.first(), res.rows.first()) else { return out };
    let Some(value) = row.get(0) else { return out };
    let Some(xml) = crate::values::cell(conn, col, value).await else { return out };
    for chunk in xml.split("<ROW>").skip(1) {
        if let (Some(name), Some(default)) = (xml_tag(chunk, "COLUMN_NAME"), xml_tag(chunk, "DATA_DEFAULT")) {
            let d = xml_unescape(default).trim().to_string();
            if !d.is_empty() {
                out.insert(xml_unescape(name), d);
            }
        }
    }
    out
}

/// `"SCHEMA"."SEQ"."NEXTVAL"` (ou `SEQ.nextval`) → `SCHEMA.SEQ`: o nome da sequência que numera a coluna.
pub(crate) fn sequence_of(default: &str) -> Option<String> {
    let d = default.trim();
    let upper = d.to_uppercase();
    let head = upper.strip_suffix("\"NEXTVAL\"").or_else(|| upper.strip_suffix("NEXTVAL"))?;
    let len = head.trim_end().strip_suffix('.')?.len();
    let seq = &d[..len];
    (!seq.is_empty() && !seq.contains(' ') && !seq.contains('(')).then(|| seq.split('.').map(|p| p.trim_matches('"')).collect::<Vec<_>>().join("."))
}

fn strip_parens(t: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in t.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// O tipo como o `CREATE TABLE` o escreve: `VARCHAR2(50 CHAR)`, `NUMBER(12,3)`, `RAW(16)`...
fn formatted_type(data_type: &str, data_length: i64, char_length: i64, char_used: &str, precision: Option<i64>, scale: Option<i64>) -> String {
    let base = strip_parens(data_type).to_uppercase();
    match base.as_str() {
        "VARCHAR2" | "CHAR" => format!("{base}({}{})", if char_used == "C" { char_length } else { data_length }, if char_used == "C" { " CHAR" } else { "" }),
        "NVARCHAR2" | "NCHAR" => format!("{base}({char_length})"),
        "RAW" => format!("RAW({data_length})"),
        "NUMBER" => match (precision, scale) {
            (Some(p), Some(sc)) if sc != 0 => format!("NUMBER({p},{sc})"),
            (Some(p), _) => format!("NUMBER({p})"),
            (None, Some(0)) => "NUMBER(*,0)".to_string(),
            _ => "NUMBER".to_string(),
        },
        "FLOAT" => match precision {
            Some(p) if p != 126 => format!("FLOAT({p})"),
            _ => "FLOAT".to_string(),
        },
        _ => data_type.to_string(),
    }
}

impl OracleSession {
    /// No Oracle o "banco" da conexão é o serviço.
    pub async fn list_databases(&self, _include_hidden: bool) -> Result<DatabaseList, DatabaseError> {
        Ok(DatabaseList { databases: vec![self.service.clone()], hidden: 0, comments: BTreeMap::new() })
    }

    /// Os schemas são os usuários.
    pub async fn list_schemas(&self, include_hidden: bool) -> Result<Vec<SchemaInfo>, DatabaseError> {
        let rows = rows_of(self.conn(), if include_hidden { ALL_USERS_SQL } else { USERS_SQL }, &[]).await?;
        Ok(rows.iter().map(|r| SchemaInfo { name: s(r, 0), comment: None }).collect())
    }

    pub async fn list_roles(&self) -> Result<Vec<RoleInfo>, DatabaseError> {
        let rows = rows_of(self.conn(), "SELECT username, oracle_maintained FROM all_users ORDER BY username", &[]).await?;
        Ok(rows.iter().map(|r| RoleInfo { name: s(r, 0), can_login: true, superuser: s(r, 1) == "Y" && s(r, 0) == "SYS" }).collect())
    }

    pub async fn list_objects(&self, schema: &str) -> Result<SchemaObjects, DatabaseError> {
        let conn = self.conn();
        let info = |name: String, rows: Option<i64>, comment: Option<String>| ObjectInfo { name, rows, comment: comment.filter(|c| !c.is_empty()), owner: Some(schema.to_string()), args: None, return_type: None, language: None, data_type: None, procedure: false };
        let sc = [v(schema)];
        let tables = rows_of(
            conn,
            "SELECT t.table_name, t.num_rows, c.comments FROM all_tables t LEFT JOIN all_tab_comments c ON c.owner = t.owner AND c.table_name = t.table_name \
              WHERE t.owner = :1 AND t.nested = 'NO' AND t.iot_name IS NULL AND NOT EXISTS (SELECT 1 FROM all_mviews m WHERE m.owner = t.owner AND m.mview_name = t.table_name) ORDER BY t.table_name",
            &sc,
        )
        .await?
        .iter()
        .map(|r| info(s(r, 0), text(r, 1).map(|_| n(r, 1)), text(r, 2)))
        .collect();
        let views = rows_of(conn, "SELECT v.view_name, c.comments FROM all_views v LEFT JOIN all_tab_comments c ON c.owner = v.owner AND c.table_name = v.view_name WHERE v.owner = :1 ORDER BY v.view_name", &sc)
            .await?
            .iter()
            .map(|r| info(s(r, 0), None, text(r, 1)))
            .collect();
        let matviews = rows_of(conn, "SELECT mview_name FROM all_mviews WHERE owner = :1 ORDER BY mview_name", &sc).await.unwrap_or_default().iter().map(|r| info(s(r, 0), None, None)).collect();
        let functions = rows_of(conn, "SELECT object_name, object_type FROM all_objects WHERE owner = :1 AND object_type IN ('FUNCTION', 'PROCEDURE') ORDER BY object_name", &sc)
            .await?
            .iter()
            .map(|r| ObjectInfo { language: Some("PL/SQL".into()), procedure: s(r, 1) == "PROCEDURE", ..info(s(r, 0), None, None) })
            .collect();
        let sequences = rows_of(conn, "SELECT sequence_name FROM all_sequences WHERE sequence_owner = :1 ORDER BY sequence_name", &sc).await?.iter().map(|r| info(s(r, 0), None, None)).collect();
        Ok(SchemaObjects { tables, views, matviews, functions, sequences, ..Default::default() })
    }

    pub async fn table_detail(&self, schema: &str, table: &str) -> Result<TableDetail, DatabaseError> {
        let conn = self.conn();
        let st = [v(schema), v(table)];
        const COLS: &str = "SELECT c.column_name, c.data_type, c.data_length, c.char_length, c.char_used, c.data_precision, c.data_scale, c.nullable, DEFAULTEXPR, c.identity_column, c.virtual_column, cc.comments, ic.generation_type \
               FROM all_tab_cols c LEFT JOIN all_col_comments cc ON cc.owner = c.owner AND cc.table_name = c.table_name AND cc.column_name = c.column_name \
               LEFT JOIN all_tab_identity_cols ic ON ic.owner = c.owner AND ic.table_name = c.table_name AND ic.column_name = c.column_name \
              WHERE c.owner = :1 AND c.table_name = :2 AND c.hidden_column = 'NO' ORDER BY c.column_id";
        let cols = rows_of(conn, &COLS.replace("DEFAULTEXPR", "CAST(NULL AS VARCHAR2(1))"), &st).await?;
        if cols.is_empty() {
            let mut e = DatabaseError::new(ErrorKind::InvalidInput, format!("table \"{schema}\".\"{table}\" does not exist"));
            e.code = Some("ORA-00942".into());
            return Err(e);
        }

        // Restrições: chave primária (vira a posição da coluna), únicas, estrangeiras e CHECK.
        let cons = rows_of(
            conn,
            "SELECT c.constraint_name, c.constraint_type, c.delete_rule, c.search_condition_vc, c.index_name, c.generated, \
                    (SELECT LISTAGG(cc.column_name, ', ') WITHIN GROUP (ORDER BY cc.position) FROM all_cons_columns cc WHERE cc.owner = c.owner AND cc.constraint_name = c.constraint_name), \
                    (SELECT p.owner || '.' || p.table_name FROM all_constraints p WHERE p.owner = c.r_owner AND p.constraint_name = c.r_constraint_name), \
                    (SELECT LISTAGG(pc.column_name, ', ') WITHIN GROUP (ORDER BY pc.position) FROM all_cons_columns pc WHERE pc.owner = c.r_owner AND pc.constraint_name = c.r_constraint_name) \
               FROM all_constraints c WHERE c.owner = :1 AND c.table_name = :2 AND c.constraint_type IN ('P', 'U', 'R', 'C') ORDER BY c.constraint_name",
            &st,
        )
        .await;
        let cons = match cons {
            Ok(r) => r,
            // Antes do 12.2 não há `SEARCH_CONDITION_VC`: lê sem o texto dos CHECK.
            Err(_) => rows_of(
                conn,
                "SELECT c.constraint_name, c.constraint_type, c.delete_rule, NULL, c.index_name, c.generated, \
                        (SELECT LISTAGG(cc.column_name, ', ') WITHIN GROUP (ORDER BY cc.position) FROM all_cons_columns cc WHERE cc.owner = c.owner AND cc.constraint_name = c.constraint_name), \
                        (SELECT p.owner || '.' || p.table_name FROM all_constraints p WHERE p.owner = c.r_owner AND p.constraint_name = c.r_constraint_name), \
                        (SELECT LISTAGG(pc.column_name, ', ') WITHIN GROUP (ORDER BY pc.position) FROM all_cons_columns pc WHERE pc.owner = c.r_owner AND pc.constraint_name = c.r_constraint_name) \
                   FROM all_constraints c WHERE c.owner = :1 AND c.table_name = :2 AND c.constraint_type IN ('P', 'U', 'R') ORDER BY c.constraint_name",
                &st,
            )
            .await?,
        };
        let mut keys: BTreeMap<String, i32> = BTreeMap::new();
        let mut pk_index: Option<String> = None;
        let mut constraints = Vec::new();
        for r in &cons {
            let (name, kind, fields) = (s(r, 0), s(r, 1), s(r, 6));
            match kind.as_str() {
                "P" => {
                    for (i, c) in fields.split(", ").enumerate() {
                        keys.insert(c.to_string(), i as i32 + 1);
                    }
                    pk_index = text(r, 4);
                }
                "U" => constraints.push(ConstraintInfo { name, kind: "u".into(), fields: fields.clone(), ref_schema: None, ref_table: None, ref_fields: None, on_delete: None, on_update: None, definition: format!("UNIQUE ({fields})"), comment: None }),
                "R" => {
                    let target = s(r, 7);
                    let (ref_owner, ref_table) = target.split_once('.').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or_default();
                    let ref_fields = s(r, 8);
                    constraints.push(ConstraintInfo {
                        name,
                        kind: "f".into(),
                        fields: fields.clone(),
                        ref_schema: Some(ref_owner.clone()),
                        ref_table: Some(ref_table.clone()),
                        ref_fields: Some(ref_fields.clone()),
                        on_delete: Some(s(r, 2)),
                        on_update: Some("NO ACTION".into()),
                        definition: format!("FOREIGN KEY ({fields}) REFERENCES \"{ref_owner}\".\"{ref_table}\" ({ref_fields})"),
                        comment: None,
                    });
                }
                _ => {
                    // `NOT NULL` também é uma restrição CHECK no catálogo; ela já aparece na coluna.
                    let cond = s(r, 3);
                    let not_null = s(r, 5) == "GENERATED NAME" && cond.trim_end().to_uppercase().ends_with("IS NOT NULL");
                    if !cond.is_empty() && !not_null {
                        constraints.push(ConstraintInfo { name, kind: "c".into(), fields: String::new(), ref_schema: None, ref_table: None, ref_fields: None, on_delete: None, on_update: None, definition: format!("CHECK (({}))", cond.trim()), comment: None });
                    }
                }
            }
        }

        let defaults = defaults_via_xml(conn, schema, table).await;
        let columns: Vec<ColumnInfo> = cols
            .iter()
            .map(|r| {
                let name = s(r, 0);
                let raw_type = s(r, 1);
                let (data_length, char_length) = (n(r, 2), n(r, 3));
                let (precision, scale) = (text(r, 5).map(|_| n(r, 5)), text(r, 6).map(|_| n(r, 6)));
                let data_type = strip_parens(&raw_type);
                let default = defaults.get(&name).cloned();
                // `DEFAULT sequência.NEXTVAL`: a numeração de uma coluna que já existia (o Oracle não a converte em identidade).
                let sequence = default.as_deref().and_then(sequence_of);
                let identity = if s(r, 9) == "YES" { Some(if s(r, 12) == "ALWAYS" { "a" } else { "d" }.to_string()) } else { sequence.as_ref().map(|_| "d".to_string()) };
                let (length, decimal) = match data_type.as_str() {
                    "number" | "float" => (precision.unwrap_or(0) as i32, scale.unwrap_or(0) as i32),
                    "varchar2" | "char" | "nvarchar2" | "nchar" => ((if s(r, 4) == "C" || data_type.starts_with('n') { char_length } else { data_length }) as i32, 0),
                    "raw" => (data_length as i32, 0),
                    _ if data_type.starts_with("timestamp") => (0, scale.unwrap_or(0) as i32),
                    _ => (0, 0),
                };
                ColumnInfo {
                    key: keys.get(&name).copied(),
                    formatted_type: formatted_type(&raw_type, data_length, char_length, &s(r, 4), precision, scale),
                    data_type,
                    length,
                    decimal,
                    not_null: s(r, 7) == "N",
                    // A identidade guarda no padrão a sequência interna do servidor: não é um valor do usuário.
                    default: if identity.is_some() || sequence.is_some() { None } else { default.clone() },
                    comment: text(r, 11).filter(|c| !c.is_empty()),
                    collation: None,
                    identity,
                    generated: (s(r, 10) == "YES").then(|| "s".to_string()),
                    auto_source: sequence,
                    name,
                }
            })
            .collect();

        let idx = rows_of(
            conn,
            "SELECT i.index_name, i.uniqueness, i.index_type, \
                    (SELECT LISTAGG(ic.column_name || CASE WHEN ic.descend = 'DESC' THEN ' DESC' END, ', ') WITHIN GROUP (ORDER BY ic.column_position) FROM all_ind_columns ic WHERE ic.index_owner = i.owner AND ic.index_name = i.index_name) \
               FROM all_indexes i WHERE i.table_owner = :1 AND i.table_name = :2 ORDER BY i.index_name",
            &st,
        )
        .await?;
        let indexes = idx
            .iter()
            .map(|r| {
                let (name, unique, fields) = (s(r, 0), s(r, 1) == "UNIQUE", s(r, 3));
                IndexInfo {
                    primary: pk_index.as_deref() == Some(name.as_str()),
                    definition: format!("CREATE {}INDEX \"{schema}\".\"{name}\" ON \"{schema}\".\"{table}\" ({fields})", if unique { "UNIQUE " } else { "" }),
                    method: s(r, 2).to_lowercase(),
                    name,
                    fields,
                    unique,
                    comment: None,
                }
            })
            .collect();

        let trg = rows_of(conn, "SELECT trigger_name, trigger_type, triggering_event, status FROM all_triggers WHERE table_owner = :1 AND table_name = :2 ORDER BY trigger_name", &st).await?;
        let triggers = trg
            .iter()
            .map(|r| {
                let kind = s(r, 1).to_uppercase();
                let timing = if kind.starts_with("INSTEAD OF") { "INSTEAD OF" } else if kind.starts_with("BEFORE") { "BEFORE" } else { "AFTER" };
                TriggerInfo {
                    name: s(r, 0),
                    definition: String::new(),
                    enabled: s(r, 3) == "ENABLED",
                    timing: timing.to_string(),
                    events: s(r, 2).split(" OR ").map(|e| e.trim().to_string()).collect(),
                    for_each_row: kind.contains("EACH ROW"),
                    when: None,
                    function: String::new(),
                    args: Vec::new(),
                    update_columns: Vec::new(),
                    comment: None,
                    read_only: true,
                }
            })
            .collect();

        let comment = rows_of(conn, "SELECT comments FROM all_tab_comments WHERE owner = :1 AND table_name = :2", &st).await?;
        Ok(TableDetail { columns, indexes, constraints, triggers, owner: Some(schema.to_string()), comment: comment.first().and_then(|r| text(r, 0)).filter(|c| !c.is_empty()) })
    }

    pub async fn view_detail(&self, schema: &str, view: &str) -> Result<ViewDetail, DatabaseError> {
        let conn = self.conn();
        let st = [v(schema), v(view)];
        let rows = rows_of(conn, "SELECT v.text_vc, c.comments FROM all_views v LEFT JOIN all_tab_comments c ON c.owner = v.owner AND c.table_name = v.view_name WHERE v.owner = :1 AND v.view_name = :2", &st).await?;
        let r = rows.first().ok_or_else(|| DatabaseError::new(ErrorKind::InvalidInput, format!("view \"{schema}\".\"{view}\" does not exist")))?;
        Ok(ViewDetail { definition: s(r, 0).trim().trim_end_matches(';').to_string(), owner: Some(schema.to_string()), comment: text(r, 1).filter(|c| !c.is_empty()), check_option: None, security_barrier: false, rules: Vec::new(), populated: true })
    }

    /// Tabelas (com colunas) e chaves estrangeiras de um schema, para o diagrama ER.
    pub async fn schema_diagram(&self, schema: &str) -> Result<SchemaDiagram, DatabaseError> {
        let conn = self.conn();
        let sc = [v(schema)];
        let cols = rows_of(
            conn,
            "SELECT c.table_name, c.column_name, c.data_type, NVL(c.char_length, 0), NVL(c.data_precision, 0), NVL(c.data_scale, 0),                     CASE WHEN EXISTS (SELECT 1 FROM all_cons_columns cc JOIN all_constraints k ON k.owner = cc.owner AND k.constraint_name = cc.constraint_name                                        WHERE k.constraint_type = 'P' AND cc.owner = c.owner AND cc.table_name = c.table_name AND cc.column_name = c.column_name) THEN 1 ELSE 0 END,                     CASE WHEN EXISTS (SELECT 1 FROM all_cons_columns cc JOIN all_constraints k ON k.owner = cc.owner AND k.constraint_name = cc.constraint_name                                        WHERE k.constraint_type = 'R' AND cc.owner = c.owner AND cc.table_name = c.table_name AND cc.column_name = c.column_name) THEN 1 ELSE 0 END                FROM all_tab_cols c JOIN all_tables t ON t.owner = c.owner AND t.table_name = c.table_name               WHERE c.owner = :1 AND c.hidden_column = 'NO' AND t.nested = 'NO' AND t.iot_name IS NULL                 AND NOT EXISTS (SELECT 1 FROM all_mviews m WHERE m.owner = t.owner AND m.mview_name = t.table_name) ORDER BY c.table_name, c.column_id",
            &sc,
        )
        .await?;
        let mut tables: Vec<DiagramTable> = Vec::new();
        for r in &cols {
            let table = s(r, 0);
            let data_type = strip_parens(&s(r, 2));
            let (length, decimal) = match data_type.as_str() {
                "number" | "float" => (n(r, 4), n(r, 5)),
                _ => (n(r, 3).clamp(0, i64::from(i32::MAX)), 0),
            };
            let column = DiagramColumn { name: s(r, 1), data_type, length: length as i32, decimal: decimal as i32, is_pk: n(r, 6) == 1, is_fk: n(r, 7) == 1 };
            match tables.last_mut() {
                Some(t) if t.name == table => t.columns.push(column),
                _ => tables.push(DiagramTable { name: table, columns: vec![column] }),
            }
        }
        let fks = rows_of(
            conn,
            "SELECT k.constraint_name, k.table_name, p.owner, p.table_name, cc.column_name, pc.column_name, c.nullable                FROM all_constraints k                JOIN all_cons_columns cc ON cc.owner = k.owner AND cc.constraint_name = k.constraint_name                JOIN all_constraints p ON p.owner = k.r_owner AND p.constraint_name = k.r_constraint_name                JOIN all_cons_columns pc ON pc.owner = p.owner AND pc.constraint_name = p.constraint_name AND pc.position = cc.position                JOIN all_tab_cols c ON c.owner = k.owner AND c.table_name = k.table_name AND c.column_name = cc.column_name               WHERE k.owner = :1 AND k.constraint_type = 'R' ORDER BY k.table_name, k.constraint_name, cc.position",
            &sc,
        )
        .await?;
        let mut relations: Vec<DiagramRelation> = Vec::new();
        for r in &fks {
            let (name, nullable) = (s(r, 0), s(r, 6) == "Y");
            match relations.last_mut() {
                Some(rel) if rel.name == name && rel.table == s(r, 1) => {
                    rel.columns.push(s(r, 4));
                    rel.ref_columns.push(s(r, 5));
                    rel.required = rel.required && !nullable;
                }
                _ => relations.push(DiagramRelation { name, table: s(r, 1), columns: vec![s(r, 4)], ref_schema: s(r, 2), ref_table: s(r, 3), ref_columns: vec![s(r, 5)], required: !nullable }),
            }
        }
        Ok(SchemaDiagram { tables, relations })
    }
}
