//! Estrutura do servidor: bancos, schemas, tabelas, views, procedimentos e o detalhe de uma tabela (colunas, índices, chaves, gatilhos).

use super::{Conn, MssqlSession, map_err, quote_ident, quote_literal};
use atlas_core::{
    ColumnInfo, ConstraintInfo, DatabaseError, DatabaseList, IndexInfo, ObjectInfo, RoleInfo, SchemaInfo, SchemaObjects, TableDetail, TriggerInfo,
};
use std::collections::BTreeMap;
use tiberius::{Row, ToSql};

pub(crate) fn s(r: &Row, i: usize) -> String {
    r.get::<&str, _>(i).unwrap_or("").to_string()
}
pub(crate) fn os(r: &Row, i: usize) -> Option<String> {
    r.get::<&str, _>(i).map(str::to_string)
}
pub(crate) fn n(r: &Row, i: usize) -> i32 {
    r.get::<i32, _>(i).unwrap_or(0)
}
pub(crate) fn b(r: &Row, i: usize) -> bool {
    r.get::<bool, _>(i).unwrap_or(false)
}

/// Lê todas as linhas da primeira consulta, com parâmetros `@P1...`.
pub(crate) async fn rows_of(conn: &mut Conn, sql: &str, params: &[&dyn ToSql]) -> Result<Vec<Row>, DatabaseError> {
    conn.query(sql, params).await.map_err(map_err)?.into_first_result().await.map_err(map_err)
}

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

/// Tira os parênteses de fora do valor padrão (`((0))` → `0`, `('x')` → `'x'`).
fn strip_parens(def: &str) -> String {
    let mut t = def.trim();
    while t.starts_with('(') && t.ends_with(')') {
        let inner = &t[1..t.len() - 1];
        let mut depth = 0i32;
        let balanced = inner.chars().all(|c| {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
            }
            depth >= 0
        }) && depth == 0;
        if !balanced {
            break;
        }
        t = inner.trim();
    }
    t.to_string()
}

/// O tipo como o SQL Server o escreve (`nvarchar(50)`, `decimal(10,2)`, `varchar(max)`).
fn formatted(tname: &str, max_length: i32, precision: i32, scale: i32) -> (String, i32, i32) {
    match tname {
        "nvarchar" | "nchar" => {
            let len = if max_length < 0 { 0 } else { max_length / 2 };
            (if len == 0 { format!("{tname}(max)") } else { format!("{tname}({len})") }, len, 0)
        }
        "varchar" | "char" | "varbinary" | "binary" => {
            let len = max_length.max(0);
            (if max_length < 0 { format!("{tname}(max)") } else { format!("{tname}({len})") }, len, 0)
        }
        "decimal" | "numeric" => (format!("{tname}({precision},{scale})"), precision, scale),
        "datetime2" | "time" | "datetimeoffset" => (format!("{tname}({scale})"), scale, 0),
        _ => (tname.to_string(), 0, 0),
    }
}

impl MssqlSession {
    pub async fn list_databases(&self, include_hidden: bool) -> Result<DatabaseList, DatabaseError> {
        let client = self.client("").await?;
        let mut conn = client.lock().await;
        let rows = rows_of(&mut conn, "SELECT name, CAST(database_id AS int) FROM sys.databases WHERE state = 0 AND HAS_DBACCESS(name) = 1 ORDER BY name", &[]).await?;
        let (mut databases, mut hidden) = (Vec::new(), 0u32);
        for r in &rows {
            if !include_hidden && n(r, 1) <= 4 {
                hidden += 1;
            } else {
                databases.push(s(r, 0));
            }
        }
        Ok(DatabaseList { databases, hidden, comments: BTreeMap::new() })
    }

    pub async fn list_schemas(&self, database: &str, include_hidden: bool) -> Result<Vec<SchemaInfo>, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let filter = if include_hidden { "" } else { "WHERE s.name NOT IN ('sys', 'INFORMATION_SCHEMA', 'guest') AND s.name NOT LIKE 'db[_]%'" };
        let sql = format!(
            "SELECT s.name, CAST(ep.value AS nvarchar(4000)) FROM sys.schemas s \
             LEFT JOIN sys.extended_properties ep ON ep.class = 3 AND ep.major_id = s.schema_id AND ep.name = 'MS_Description' {filter} ORDER BY s.name"
        );
        let rows = rows_of(&mut conn, &sql, &[]).await?;
        Ok(rows.iter().map(|r| SchemaInfo { name: s(r, 0), comment: os(r, 1).filter(|c| !c.is_empty()) }).collect())
    }

    pub async fn list_roles(&self) -> Result<Vec<RoleInfo>, DatabaseError> {
        let client = self.client("").await?;
        let mut conn = client.lock().await;
        let rows = rows_of(
            &mut conn,
            "SELECT name, CAST(CASE WHEN is_disabled = 0 THEN 1 ELSE 0 END AS bit), CAST(IS_SRVROLEMEMBER('sysadmin', name) AS int) FROM sys.server_principals WHERE type IN ('S', 'U', 'G') ORDER BY name",
            &[],
        )
        .await?;
        Ok(rows.iter().map(|r| RoleInfo { name: s(r, 0), can_login: b(r, 1), superuser: n(r, 2) == 1 }).collect())
    }

    pub async fn list_objects(&self, database: &str, schema: &str) -> Result<SchemaObjects, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let relations = |kind: &str| {
            format!(
                "SELECT o.name, CAST(SUM(CASE WHEN p.index_id IN (0, 1) THEN p.row_count ELSE 0 END) AS bigint), CAST(MAX(ep.value) AS nvarchar(4000)) \
                   FROM sys.{kind} o JOIN sys.schemas sc ON sc.schema_id = o.schema_id \
                   LEFT JOIN sys.dm_db_partition_stats p ON p.object_id = o.object_id \
                   LEFT JOIN sys.extended_properties ep ON ep.class = 1 AND ep.major_id = o.object_id AND ep.minor_id = 0 AND ep.name = 'MS_Description' \
                  WHERE sc.name = @P1 AND o.is_ms_shipped = 0 GROUP BY o.name ORDER BY o.name"
            )
        };
        let to_info = |r: &Row, with_rows: bool| ObjectInfo {
            name: s(r, 0),
            rows: if with_rows { r.get::<i64, _>(1) } else { None },
            comment: os(r, 2).filter(|c| !c.is_empty()),
            owner: None,
            args: None,
            return_type: None,
            language: None,
            data_type: None,
            procedure: false,
        };
        let tables = rows_of(&mut conn, &relations("tables"), &[&schema]).await?.iter().map(|r| to_info(r, true)).collect();
        let views = rows_of(&mut conn, &relations("views"), &[&schema]).await?.iter().map(|r| to_info(r, false)).collect();
        let routines = rows_of(
            &mut conn,
            "SELECT o.name, o.type FROM sys.objects o JOIN sys.schemas sc ON sc.schema_id = o.schema_id WHERE sc.name = @P1 AND o.type IN ('P', 'FN', 'IF', 'TF') AND o.is_ms_shipped = 0 ORDER BY o.name",
            &[&schema],
        )
        .await?;
        let functions = routines
            .iter()
            .map(|r| ObjectInfo { name: s(r, 0), rows: None, comment: None, owner: None, args: None, return_type: None, language: Some("T-SQL".into()), data_type: None, procedure: s(r, 1).trim() == "P" })
            .collect();
        let sequences = rows_of(&mut conn, "SELECT q.name FROM sys.sequences q JOIN sys.schemas sc ON sc.schema_id = q.schema_id WHERE sc.name = @P1 ORDER BY q.name", &[&schema])
            .await?
            .iter()
            .map(|r| ObjectInfo { name: s(r, 0), rows: None, comment: None, owner: None, args: None, return_type: None, language: None, data_type: None, procedure: false })
            .collect();
        Ok(SchemaObjects { tables, views, functions, sequences, ..Default::default() })
    }

    /// A definição de uma view (só o `SELECT`).
    pub async fn view_detail(&self, database: &str, schema: &str, view: &str) -> Result<atlas_core::ViewDetail, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let object = quote_literal(&format!("{}.{}", quote_ident(schema), quote_ident(view)));
        let rows = rows_of(&mut conn, &format!("SELECT CAST(OBJECT_DEFINITION(OBJECT_ID({object}, 'V')) AS nvarchar(max))"), &[]).await?;
        let sql = rows.first().and_then(|r| os(r, 0));
        let Some(sql) = sql else {
            return Err(DatabaseError::new(atlas_core::ErrorKind::InvalidInput, format!("view {schema}.{view} does not exist")));
        };
        let definition = select_of(&sql).ok_or_else(|| DatabaseError::new(atlas_core::ErrorKind::Internal, "could not read the view definition"))?;
        Ok(atlas_core::ViewDetail { definition: definition.to_string(), owner: None, comment: None, check_option: None, security_barrier: false, rules: Vec::new(), populated: true })
    }

    pub async fn table_detail(&self, database: &str, schema: &str, table: &str) -> Result<TableDetail, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let full = format!("{}.{}", quote_ident(schema), quote_ident(table));
        let object = quote_literal(&full);
        let exists = rows_of(&mut conn, &format!("SELECT CAST(OBJECT_ID({object}) AS int)"), &[]).await?;
        if exists.first().and_then(|r| r.get::<i32, _>(0)).is_none() {
            let mut e = DatabaseError::new(atlas_core::ErrorKind::InvalidInput, format!("table {full} does not exist"));
            e.code = Some("208".into());
            return Err(e);
        }

        // Colunas: tipo, tamanho, obrigatoriedade, identidade, coluna calculada, valor padrão e collation.
        let cols = rows_of(
            &mut conn,
            &format!(
                "SELECT c.name, t.name, CAST(c.max_length AS int), CAST(c.precision AS int), CAST(c.scale AS int), CAST(c.is_nullable AS bit), CAST(c.is_identity AS bit), \
                        CAST(c.is_computed AS bit), CAST(OBJECT_DEFINITION(c.default_object_id) AS nvarchar(4000)), c.collation_name, CAST(ep.value AS nvarchar(4000)) \
                   FROM sys.columns c JOIN sys.types t ON t.user_type_id = c.user_type_id \
                   LEFT JOIN sys.extended_properties ep ON ep.class = 1 AND ep.major_id = c.object_id AND ep.minor_id = c.column_id AND ep.name = 'MS_Description' \
                  WHERE c.object_id = OBJECT_ID({object}) ORDER BY c.column_id"
            ),
            &[],
        )
        .await?;
        let pk = rows_of(
            &mut conn,
            &format!(
                "SELECT c.name, CAST(ic.key_ordinal AS int) FROM sys.indexes i JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id \
                   JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id WHERE i.object_id = OBJECT_ID({object}) AND i.is_primary_key = 1"
            ),
            &[],
        )
        .await?;
        let keys: BTreeMap<String, i32> = pk.iter().map(|r| (s(r, 0), n(r, 1))).collect();
        let columns: Vec<ColumnInfo> = cols
            .iter()
            .map(|r| {
                let tname = s(r, 1);
                let (formatted_type, length, decimal) = formatted(&tname, n(r, 2), n(r, 3), n(r, 4));
                let name = s(r, 0);
                ColumnInfo {
                    key: keys.get(&name).copied(),
                    name,
                    data_type: tname,
                    formatted_type,
                    length,
                    decimal,
                    not_null: !b(r, 5),
                    default: os(r, 8).map(|d| strip_parens(&d)),
                    comment: os(r, 10).filter(|c| !c.is_empty()),
                    collation: os(r, 9),
                    // Uma coluna `IDENTITY` não recebe valor sem `SET IDENTITY_INSERT`: a interface a trata como "sempre gerada".
                    identity: if b(r, 6) { Some("a".into()) } else { None },
                    generated: if b(r, 7) { Some("s".into()) } else { None },
                }
            })
            .collect();

        // Índices (sem as colunas incluídas): campos na ordem da chave.
        let idx = rows_of(
            &mut conn,
            &format!(
                "SELECT i.name, i.type_desc, CAST(i.is_unique AS bit), CAST(i.is_primary_key AS bit), CAST(i.is_unique_constraint AS bit), \
                        STUFF((SELECT ', ' + c.name FROM sys.index_columns ic JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id \
                                WHERE ic.object_id = i.object_id AND ic.index_id = i.index_id AND ic.is_included_column = 0 ORDER BY ic.key_ordinal FOR XML PATH('')), 1, 2, '') \
                   FROM sys.indexes i WHERE i.object_id = OBJECT_ID({object}) AND i.type > 0 ORDER BY i.index_id"
            ),
            &[],
        )
        .await?;
        let mut indexes = Vec::new();
        let mut constraints = Vec::new();
        for r in &idx {
            let (name, method, unique, primary, unique_constraint, fields) = (s(r, 0), s(r, 1), b(r, 2), b(r, 3), b(r, 4), s(r, 5));
            indexes.push(IndexInfo {
                name: name.clone(),
                fields: fields.clone(),
                method: method.to_lowercase(),
                unique,
                primary,
                definition: format!("CREATE {}{} INDEX {} ON {full} ({fields})", if unique { "UNIQUE " } else { "" }, method, quote_ident(&name)),
                comment: None,
            });
            if unique_constraint {
                constraints.push(ConstraintInfo { name, kind: "u".into(), fields: fields.clone(), ref_schema: None, ref_table: None, ref_fields: None, on_delete: None, on_update: None, definition: format!("UNIQUE ({fields})"), comment: None });
            }
        }

        let fks = rows_of(
            &mut conn,
            &format!(
                "SELECT fk.name, fk.delete_referential_action_desc, fk.update_referential_action_desc, sr.name, rt.name, \
                        STUFF((SELECT ', ' + pc.name FROM sys.foreign_key_columns fc JOIN sys.columns pc ON pc.object_id = fc.parent_object_id AND pc.column_id = fc.parent_column_id \
                                WHERE fc.constraint_object_id = fk.object_id ORDER BY fc.constraint_column_id FOR XML PATH('')), 1, 2, ''), \
                        STUFF((SELECT ', ' + rc.name FROM sys.foreign_key_columns fc JOIN sys.columns rc ON rc.object_id = fc.referenced_object_id AND rc.column_id = fc.referenced_column_id \
                                WHERE fc.constraint_object_id = fk.object_id ORDER BY fc.constraint_column_id FOR XML PATH('')), 1, 2, '') \
                   FROM sys.foreign_keys fk JOIN sys.tables rt ON rt.object_id = fk.referenced_object_id JOIN sys.schemas sr ON sr.schema_id = rt.schema_id \
                  WHERE fk.parent_object_id = OBJECT_ID({object}) ORDER BY fk.name"
            ),
            &[],
        )
        .await?;
        for r in &fks {
            let (fields, ref_fields, ref_table) = (s(r, 5), s(r, 6), s(r, 4));
            constraints.push(ConstraintInfo {
                name: s(r, 0),
                kind: "f".into(),
                fields: fields.clone(),
                ref_schema: Some(s(r, 3)),
                ref_table: Some(ref_table.clone()),
                ref_fields: Some(ref_fields.clone()),
                on_delete: Some(s(r, 1).replace('_', " ")),
                on_update: Some(s(r, 2).replace('_', " ")),
                definition: format!("FOREIGN KEY ({fields}) REFERENCES {}.{} ({ref_fields})", quote_ident(&s(r, 3)), quote_ident(&ref_table)),
                comment: None,
            });
        }
        let checks = rows_of(
            &mut conn,
            &format!("SELECT name, CAST(definition AS nvarchar(4000)) FROM sys.check_constraints WHERE parent_object_id = OBJECT_ID({object}) ORDER BY name"),
            &[],
        )
        .await?;
        for r in &checks {
            constraints.push(ConstraintInfo {
                name: s(r, 0),
                kind: "c".into(),
                fields: String::new(),
                ref_schema: None,
                ref_table: None,
                ref_fields: None,
                on_delete: None,
                on_update: None,
                definition: format!("CHECK ({})", strip_parens(&s(r, 1))),
                comment: None,
            });
        }

        let trg = rows_of(
            &mut conn,
            &format!(
                "SELECT t.name, CAST(OBJECT_DEFINITION(t.object_id) AS nvarchar(max)), CAST(t.is_disabled AS bit), CAST(t.is_instead_of_trigger AS bit), \
                        STUFF((SELECT ', ' + te.type_desc FROM sys.trigger_events te WHERE te.object_id = t.object_id FOR XML PATH('')), 1, 2, '') \
                   FROM sys.triggers t WHERE t.parent_id = OBJECT_ID({object}) ORDER BY t.name"
            ),
            &[],
        )
        .await?;
        let triggers = trg
            .iter()
            .map(|r| TriggerInfo {
                name: s(r, 0),
                definition: s(r, 1),
                enabled: !b(r, 2),
                timing: if b(r, 3) { "INSTEAD OF" } else { "AFTER" }.into(),
                events: s(r, 4).split(", ").filter(|e| !e.is_empty()).map(str::to_string).collect(),
                for_each_row: false,
                when: None,
                function: String::new(),
                args: Vec::new(),
                update_columns: Vec::new(),
                comment: None,
                read_only: true,
            })
            .collect();

        let comment = rows_of(&mut conn, &format!("SELECT CAST(value AS nvarchar(4000)) FROM sys.extended_properties WHERE class = 1 AND major_id = OBJECT_ID({object}) AND minor_id = 0 AND name = 'MS_Description'"), &[]).await?;
        Ok(TableDetail { columns, indexes, constraints, triggers, owner: None, comment: comment.first().and_then(|r| os(r, 0)).filter(|c| !c.is_empty()) })
    }
}
