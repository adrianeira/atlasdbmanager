//! Estrutura do servidor: bancos (que são também os schemas), tabelas, views, rotinas e o detalhe de uma tabela (colunas, índices,
//! chaves, gatilhos), lidos do `information_schema`.

use super::{MysqlSession, map_err};
use atlas_core::{ColumnInfo, ConstraintInfo, DatabaseError, DatabaseList, IndexInfo, ObjectInfo, RoleInfo, SchemaInfo, SchemaObjects, TableDetail, TriggerInfo};
use mysql_async::prelude::*;
use mysql_async::{Conn, Row, Value};
use std::collections::BTreeMap;

/// Os bancos do próprio MySQL: ficam escondidos (e contados) até se pedir para mostrar.
const SYSTEM_DATABASES: [&str; 4] = ["information_schema", "mysql", "performance_schema", "sys"];

pub(crate) fn text(r: &Row, i: usize) -> Option<String> {
    match r.as_ref(i)? {
        Value::NULL => None,
        Value::Bytes(b) => Some(String::from_utf8_lossy(b).into_owned()),
        Value::Int(n) => Some(n.to_string()),
        Value::UInt(n) => Some(n.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Double(f) => Some(f.to_string()),
        other => Some(format!("{other:?}")),
    }
}
pub(crate) fn s(r: &Row, i: usize) -> String {
    text(r, i).unwrap_or_default()
}
pub(crate) fn n(r: &Row, i: usize) -> i64 {
    text(r, i).and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// Lê todas as linhas de uma consulta com parâmetros posicionais (`?`).
pub(crate) async fn rows_of(conn: &mut Conn, sql: &str, params: Vec<Value>) -> Result<Vec<Row>, DatabaseError> {
    conn.exec(sql, params).await.map_err(map_err)
}

fn v(s: &str) -> Value {
    Value::Bytes(s.as_bytes().to_vec())
}

impl MysqlSession {
    pub async fn list_databases(&self, include_hidden: bool) -> Result<DatabaseList, DatabaseError> {
        let client = self.client("").await?;
        let mut conn = client.lock().await;
        let rows = rows_of(&mut conn, "SELECT schema_name FROM information_schema.schemata ORDER BY schema_name", vec![]).await?;
        let (mut databases, mut hidden) = (Vec::new(), 0u32);
        for r in &rows {
            let name = s(r, 0);
            if !include_hidden && SYSTEM_DATABASES.contains(&name.to_lowercase().as_str()) {
                hidden += 1;
            } else {
                databases.push(name);
            }
        }
        Ok(DatabaseList { databases, hidden, comments: BTreeMap::new() })
    }

    /// No MySQL o schema é o próprio banco.
    pub async fn list_schemas(&self, database: &str) -> Vec<SchemaInfo> {
        vec![SchemaInfo { name: database.to_string(), comment: None }]
    }

    /// Os usuários do servidor (se o login pode lê-los).
    pub async fn list_roles(&self) -> Result<Vec<RoleInfo>, DatabaseError> {
        let client = self.client("").await?;
        let mut conn = client.lock().await;
        let rows = rows_of(&mut conn, "SELECT user, host, super_priv FROM mysql.user ORDER BY user, host", vec![]).await.unwrap_or_default();
        Ok(rows.iter().map(|r| RoleInfo { name: format!("{}@{}", s(r, 0), s(r, 1)), can_login: true, superuser: s(r, 2) == "Y" }).collect())
    }

    pub async fn list_objects(&self, database: &str) -> Result<SchemaObjects, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let info = |name: String, rows: Option<i64>, comment: Option<String>| ObjectInfo { name, rows, comment: comment.filter(|c| !c.is_empty()), owner: None, args: None, return_type: None, language: None, data_type: None, procedure: false };
        let tables = rows_of(&mut conn, "SELECT table_name, table_rows, table_comment FROM information_schema.tables WHERE table_schema = ? AND table_type = 'BASE TABLE' ORDER BY table_name", vec![v(database)])
            .await?
            .iter()
            .map(|r| info(s(r, 0), Some(n(r, 1)), text(r, 2)))
            .collect();
        let views = rows_of(&mut conn, "SELECT table_name FROM information_schema.tables WHERE table_schema = ? AND table_type = 'VIEW' ORDER BY table_name", vec![v(database)])
            .await?
            .iter()
            .map(|r| info(s(r, 0), None, None))
            .collect();
        let functions = rows_of(&mut conn, "SELECT routine_name, routine_type, dtd_identifier FROM information_schema.routines WHERE routine_schema = ? ORDER BY routine_name", vec![v(database)])
            .await?
            .iter()
            .map(|r| ObjectInfo { name: s(r, 0), rows: None, comment: None, owner: None, args: None, return_type: text(r, 2).filter(|_| s(r, 1) == "FUNCTION"), language: Some("SQL".into()), data_type: None, procedure: s(r, 1) == "PROCEDURE" })
            .collect();
        Ok(SchemaObjects { tables, views, functions, ..Default::default() })
    }

    /// Conjunto de caracteres e collation do banco (o MySQL não tem dono, limite de conexões nem tablespace por banco).
    pub async fn database_detail(&self, database: &str) -> Result<atlas_core::DatabaseDetail, DatabaseError> {
        let client = self.client("").await?;
        let mut conn = client.lock().await;
        let rows = rows_of(&mut conn, "SELECT default_character_set_name, default_collation_name FROM information_schema.schemata WHERE schema_name = ?", vec![v(database)]).await?;
        let Some(r) = rows.first() else {
            return Err(DatabaseError::new(atlas_core::ErrorKind::DatabaseNotFound, format!("database `{database}` does not exist")));
        };
        Ok(atlas_core::DatabaseDetail {
            name: database.to_string(),
            owner: String::new(),
            encoding: s(r, 0),
            collate: s(r, 1),
            ctype: String::new(),
            tablespace: String::new(),
            connection_limit: -1,
            allow_connections: true,
            is_template: false,
            comment: None,
        })
    }

    /// Argumentos, tipo de retorno e comentário de uma função ou procedimento (para o Data Dictionary).
    pub async fn routine_info(&self, database: &str, name: &str) -> Result<(String, Option<String>, Option<String>), DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let head = rows_of(&mut conn, "SELECT dtd_identifier, routine_comment, routine_type FROM information_schema.routines WHERE routine_schema = ? AND routine_name = ?", vec![v(database), v(name)]).await?;
        let Some(h) = head.first() else {
            return Err(DatabaseError::new(atlas_core::ErrorKind::InvalidInput, format!("routine `{database}`.`{name}` does not exist")));
        };
        let params = rows_of(
            &mut conn,
            "SELECT parameter_mode, parameter_name, dtd_identifier FROM information_schema.parameters WHERE specific_schema = ? AND specific_name = ? AND parameter_name IS NOT NULL ORDER BY ordinal_position",
            vec![v(database), v(name)],
        )
        .await?;
        let arguments = params.iter().map(|r| format!("{} {} {}", text(r, 0).unwrap_or_default(), s(r, 1), s(r, 2)).trim().to_string()).collect::<Vec<_>>().join(", ");
        let returns = if s(h, 2) == "FUNCTION" { text(h, 0) } else { None };
        Ok((arguments, returns, text(h, 1).filter(|c| !c.is_empty())))
    }

    /// Todas as collations do servidor com o conjunto de caracteres de cada uma (a padrão do conjunto vem primeiro).
    pub async fn collations(&self) -> Result<Vec<[String; 2]>, DatabaseError> {
        let client = self.client("").await?;
        let mut conn = client.lock().await;
        let rows = rows_of(&mut conn, "SELECT collation_name, character_set_name FROM information_schema.collations ORDER BY character_set_name, is_default = 'Yes' DESC, collation_name", Vec::new()).await?;
        Ok(rows.iter().map(|r| [s(r, 0), s(r, 1)]).collect())
    }

    /// O texto de uma função ou procedimento (`CREATE ...`), sem o `DEFINER`.
    pub async fn routine_definition(&self, database: &str, name: &str) -> Result<String, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let kinds = rows_of(&mut conn, "SELECT routine_type FROM information_schema.routines WHERE routine_schema = ? AND routine_name = ?", vec![v(database), v(name)]).await?;
        let Some(kind) = kinds.first().map(|r| s(r, 0)) else {
            return Err(DatabaseError::new(atlas_core::ErrorKind::InvalidInput, format!("routine `{database}`.`{name}` does not exist")));
        };
        let rows = rows_of(&mut conn, &format!("SHOW CREATE {kind} {}", super::quote_ident(name)), vec![]).await?;
        let ddl = rows.first().and_then(|r| text(r, 2)).ok_or_else(|| DatabaseError::new(atlas_core::ErrorKind::PermissionDenied, "the definition is not visible to this login"))?;
        Ok(crate::dump::without_definer(&ddl))
    }

    /// Definição de uma view para o designer (só o SELECT).
    pub async fn view_detail(&self, database: &str, view: &str) -> Result<atlas_core::ViewDetail, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let rows = rows_of(&mut conn, "SELECT view_definition, check_option FROM information_schema.views WHERE table_schema = ? AND table_name = ?", vec![v(database), v(view)]).await?;
        let Some(r) = rows.first() else {
            return Err(DatabaseError::new(atlas_core::ErrorKind::InvalidInput, format!("view `{database}`.`{view}` does not exist")));
        };
        let check = text(r, 1).map(|c| c.to_lowercase()).filter(|c| c == "local" || c == "cascaded");
        Ok(atlas_core::ViewDetail { definition: s(r, 0).trim().trim_end_matches(';').to_string(), owner: None, comment: None, check_option: check, security_barrier: false, rules: Vec::new(), populated: true })
    }

    pub async fn table_detail(&self, database: &str, table: &str) -> Result<TableDetail, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let cols = rows_of(
            &mut conn,
            "SELECT column_name, data_type, column_type, character_maximum_length, numeric_precision, numeric_scale, is_nullable, column_default, extra, column_key, collation_name, column_comment, generation_expression \
               FROM information_schema.columns WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
            vec![v(database), v(table)],
        )
        .await?;
        if cols.is_empty() {
            let mut e = DatabaseError::new(atlas_core::ErrorKind::InvalidInput, format!("table `{database}`.`{table}` does not exist"));
            e.code = Some("1146".into());
            return Err(e);
        }
        let pk = rows_of(&mut conn, "SELECT column_name, seq_in_index FROM information_schema.statistics WHERE table_schema = ? AND table_name = ? AND index_name = 'PRIMARY'", vec![v(database), v(table)]).await?;
        let keys: BTreeMap<String, i32> = pk.iter().map(|r| (s(r, 0), n(r, 1) as i32)).collect();
        let columns: Vec<ColumnInfo> = cols
            .iter()
            .map(|r| {
                let (name, data_type, column_type) = (s(r, 0), s(r, 1).to_lowercase(), s(r, 2));
                let extra = s(r, 8).to_lowercase();
                let (length, decimal) = match data_type.as_str() {
                    "decimal" | "numeric" => (n(r, 4) as i32, n(r, 5) as i32),
                    _ => (n(r, 3).clamp(0, i32::MAX as i64) as i32, 0),
                };
                // O valor padrão: expressões (`DEFAULT_GENERATED`) vêm como são; literais de texto ganham aspas.
                let default = text(r, 7).map(|d| {
                    let textual = matches!(data_type.as_str(), "char" | "varchar" | "text" | "tinytext" | "mediumtext" | "longtext" | "enum" | "set" | "date" | "datetime" | "timestamp" | "time" | "year" | "binary" | "varbinary");
                    if extra.contains("default_generated") || !textual || d.eq_ignore_ascii_case("current_timestamp") { d } else { format!("'{}'", d.replace('\'', "''")) }
                });
                ColumnInfo {
                    key: keys.get(&name).copied(),
                    name,
                    data_type,
                    formatted_type: column_type,
                    length,
                    decimal,
                    not_null: s(r, 6) == "NO",
                    default,
                    comment: text(r, 11).filter(|c| !c.is_empty()),
                    collation: text(r, 10),
                    // `AUTO_INCREMENT` aceita valor explícito: é a identidade "por padrão".
                    identity: if extra.contains("auto_increment") { Some("d".into()) } else { None },
                    generated: if extra.contains("generated") && !extra.contains("default_generated") { Some("s".into()) } else { None },
                }
            })
            .collect();

        // Índices: o nome, as colunas na ordem e se é único.
        let idx = rows_of(
            &mut conn,
            "SELECT index_name, non_unique, index_type, GROUP_CONCAT(column_name ORDER BY seq_in_index SEPARATOR ', ') FROM information_schema.statistics \
              WHERE table_schema = ? AND table_name = ? GROUP BY index_name, non_unique, index_type ORDER BY index_name = 'PRIMARY' DESC, index_name",
            vec![v(database), v(table)],
        )
        .await?;
        let mut indexes = Vec::new();
        let mut constraints = Vec::new();
        for r in &idx {
            let (name, unique, method, fields) = (s(r, 0), n(r, 1) == 0, s(r, 2), s(r, 3));
            let primary = name == "PRIMARY";
            indexes.push(IndexInfo {
                name: name.clone(),
                fields: fields.clone(),
                method: method.to_lowercase(),
                unique,
                primary,
                definition: format!("CREATE {}INDEX `{name}` ON `{table}` ({fields})", if unique { "UNIQUE " } else { "" }),
                comment: None,
            });
            // No MySQL um índice único é a restrição UNIQUE.
            if unique && !primary {
                constraints.push(ConstraintInfo { name, kind: "u".into(), fields: fields.clone(), ref_schema: None, ref_table: None, ref_fields: None, on_delete: None, on_update: None, definition: format!("UNIQUE ({fields})"), comment: None });
            }
        }

        let fks = rows_of(
            &mut conn,
            "SELECT k.constraint_name, k.referenced_table_schema, k.referenced_table_name, r.delete_rule, r.update_rule, \
                    GROUP_CONCAT(k.column_name ORDER BY k.ordinal_position SEPARATOR ', '), GROUP_CONCAT(k.referenced_column_name ORDER BY k.ordinal_position SEPARATOR ', ') \
               FROM information_schema.key_column_usage k JOIN information_schema.referential_constraints r ON r.constraint_schema = k.constraint_schema AND r.constraint_name = k.constraint_name AND r.table_name = k.table_name \
              WHERE k.table_schema = ? AND k.table_name = ? AND k.referenced_table_name IS NOT NULL GROUP BY k.constraint_name, k.referenced_table_schema, k.referenced_table_name, r.delete_rule, r.update_rule ORDER BY k.constraint_name",
            vec![v(database), v(table)],
        )
        .await?;
        for r in &fks {
            let (fields, ref_fields) = (s(r, 5), s(r, 6));
            constraints.push(ConstraintInfo {
                name: s(r, 0),
                kind: "f".into(),
                fields: fields.clone(),
                ref_schema: Some(s(r, 1)),
                ref_table: Some(s(r, 2)),
                ref_fields: Some(ref_fields.clone()),
                on_delete: Some(s(r, 3)),
                on_update: Some(s(r, 4)),
                definition: format!("FOREIGN KEY ({fields}) REFERENCES `{}`.`{}` ({ref_fields})", s(r, 1), s(r, 2)),
                comment: None,
            });
        }
        // CHECK (MySQL 8.0.16 em diante; antes a consulta falha e a lista fica vazia).
        let checks = rows_of(
            &mut conn,
            "SELECT tc.constraint_name, cc.check_clause FROM information_schema.table_constraints tc JOIN information_schema.check_constraints cc ON cc.constraint_schema = tc.constraint_schema AND cc.constraint_name = tc.constraint_name \
              WHERE tc.table_schema = ? AND tc.table_name = ? AND tc.constraint_type = 'CHECK' ORDER BY tc.constraint_name",
            vec![v(database), v(table)],
        )
        .await
        .unwrap_or_default();
        for r in &checks {
            // Na forma `CHECK ((expressão))`, como o PostgreSQL escreve: é o que a interface sabe desembrulhar.
            let clause = s(r, 1);
            let inner = clause.trim().strip_prefix('(').and_then(|c| c.strip_suffix(')')).unwrap_or(clause.trim());
            constraints.push(ConstraintInfo { name: s(r, 0), kind: "c".into(), fields: String::new(), ref_schema: None, ref_table: None, ref_fields: None, on_delete: None, on_update: None, definition: format!("CHECK (({inner}))"), comment: None });
        }

        let trg = rows_of(&mut conn, "SELECT trigger_name, action_timing, event_manipulation, action_statement FROM information_schema.triggers WHERE event_object_schema = ? AND event_object_table = ? ORDER BY trigger_name", vec![v(database), v(table)]).await?;
        let triggers = trg
            .iter()
            .map(|r| TriggerInfo {
                name: s(r, 0),
                definition: s(r, 3),
                enabled: true,
                timing: s(r, 1),
                events: vec![s(r, 2)],
                for_each_row: true,
                when: None,
                function: String::new(),
                args: Vec::new(),
                update_columns: Vec::new(),
                comment: None,
                read_only: true,
            })
            .collect();
        let comment = rows_of(&mut conn, "SELECT table_comment FROM information_schema.tables WHERE table_schema = ? AND table_name = ?", vec![v(database), v(table)]).await?;
        Ok(TableDetail { columns, indexes, constraints, triggers, owner: None, comment: comment.first().and_then(|r| text(r, 0)).filter(|c| !c.is_empty()) })
    }
}
