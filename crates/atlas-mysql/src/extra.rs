//! Rótulos de chave estrangeira e diagrama ER do MySQL.

use super::{MysqlSession, quote_ident};
use crate::meta::{n, rows_of, s};
use atlas_core::{DatabaseError, DiagramColumn, DiagramRelation, DiagramTable, SchemaDiagram};
use mysql_async::Value;
use serde::Serialize;

/// Rótulos de chaves estrangeiras: para cada chave pedida, o texto da coluna de descrição da tabela referenciada.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FkLabels {
    pub label_column: Option<String>,
    pub labels: Vec<[String; 2]>,
}

/// Máximo de chaves por chamada (a grade mostra uma página por vez).
const MAX_FK_KEYS: usize = 5_000;

fn v(s: &str) -> Value {
    Value::Bytes(s.as_bytes().to_vec())
}

/// Escolhe a coluna de descrição: nomes típicos primeiro (`nome`, `name`, `descricao`...), depois a primeira coluna de texto
/// que não seja a própria chave nem pareça outra chave (`..._id`, `id_...`, `codigo`).
fn pick_label_column(columns: &[(String, String)], key: &str) -> Option<String> {
    let textual = |t: &str| matches!(t, "varchar" | "char" | "tinytext" | "text" | "mediumtext" | "longtext");
    let candidates: Vec<&(String, String)> = columns.iter().filter(|(n, t)| n != key && textual(t)).collect();
    const PREFERRED: [&str; 12] = ["nome", "name", "titulo", "title", "descricao", "description", "descr", "label", "razao_social", "fantasia", "apelido", "rotulo"];
    for want in PREFERRED {
        if let Some((n, _)) = candidates.iter().find(|(n, _)| n.to_lowercase() == want) {
            return Some(n.clone());
        }
    }
    for want in PREFERRED {
        if let Some((n, _)) = candidates.iter().find(|(n, _)| n.to_lowercase().contains(want)) {
            return Some(n.clone());
        }
    }
    candidates
        .iter()
        .find(|(n, _)| {
            let l = n.to_lowercase();
            !(l.ends_with("_id") || l.starts_with("id_") || l.starts_with("cod") || l == "id")
        })
        .map(|(n, _)| n.clone())
}

impl MysqlSession {
    pub async fn fk_labels(&self, database: &str, table: &str, key_column: &str, keys: &[String]) -> Result<FkLabels, DatabaseError> {
        let none = FkLabels { label_column: None, labels: Vec::new() };
        if keys.is_empty() {
            return Ok(none);
        }
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let cols = rows_of(&mut conn, "SELECT column_name, data_type FROM information_schema.columns WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position", vec![v(database), v(table)]).await?;
        let columns: Vec<(String, String)> = cols.iter().map(|r| (s(r, 0), s(r, 1).to_lowercase())).collect();
        let Some(label) = pick_label_column(&columns, key_column) else { return Ok(none) };
        let target = format!("{}.{}", quote_ident(database), quote_ident(table));
        let mut labels = Vec::new();
        for chunk in keys.iter().take(MAX_FK_KEYS).collect::<Vec<_>>().chunks(1000) {
            let marks = vec!["?"; chunk.len()].join(", ");
            let sql = format!(
                "SELECT CAST({k} AS CHAR), CAST({l} AS CHAR) FROM {target} WHERE CAST({k} AS CHAR) IN ({marks}) AND {l} IS NOT NULL",
                k = quote_ident(key_column),
                l = quote_ident(&label)
            );
            let rows = rows_of(&mut conn, &sql, chunk.iter().map(|k| v(k)).collect()).await?;
            labels.extend(rows.iter().map(|r| [s(r, 0), s(r, 1)]));
        }
        Ok(FkLabels { label_column: Some(label), labels })
    }

    /// Tabelas (com colunas) e chaves estrangeiras de um banco, para o diagrama ER.
    pub async fn schema_diagram(&self, database: &str) -> Result<SchemaDiagram, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let cols = rows_of(
            &mut conn,
            "SELECT c.table_name, c.column_name, c.data_type, COALESCE(c.character_maximum_length, 0), COALESCE(c.numeric_precision, 0), COALESCE(c.numeric_scale, 0), \
                    c.column_key = 'PRI', \
                    EXISTS (SELECT 1 FROM information_schema.key_column_usage k WHERE k.table_schema = c.table_schema AND k.table_name = c.table_name AND k.column_name = c.column_name AND k.referenced_table_name IS NOT NULL) \
               FROM information_schema.columns c JOIN information_schema.tables t ON t.table_schema = c.table_schema AND t.table_name = c.table_name AND t.table_type = 'BASE TABLE' \
              WHERE c.table_schema = ? ORDER BY c.table_name, c.ordinal_position",
            vec![v(database)],
        )
        .await?;
        let mut tables: Vec<DiagramTable> = Vec::new();
        for r in &cols {
            let table = s(r, 0);
            let data_type = s(r, 2).to_lowercase();
            let (length, decimal) = match data_type.as_str() {
                "decimal" | "numeric" => (n(r, 4), n(r, 5)),
                _ => (n(r, 3).clamp(0, i64::from(i32::MAX)), 0),
            };
            let column = DiagramColumn { name: s(r, 1), data_type, length: length as i32, decimal: decimal as i32, is_pk: n(r, 6) == 1, is_fk: n(r, 7) == 1 };
            match tables.last_mut() {
                Some(t) if t.name == table => t.columns.push(column),
                _ => tables.push(DiagramTable { name: table, columns: vec![column] }),
            }
        }
        let fks = rows_of(
            &mut conn,
            "SELECT k.constraint_name, k.table_name, k.referenced_table_schema, k.referenced_table_name, k.column_name, k.referenced_column_name, c.is_nullable = 'YES' \
               FROM information_schema.key_column_usage k JOIN information_schema.columns c ON c.table_schema = k.table_schema AND c.table_name = k.table_name AND c.column_name = k.column_name \
              WHERE k.table_schema = ? AND k.referenced_table_name IS NOT NULL ORDER BY k.table_name, k.constraint_name, k.ordinal_position",
            vec![v(database)],
        )
        .await?;
        let mut relations: Vec<DiagramRelation> = Vec::new();
        for r in &fks {
            let (name, nullable) = (s(r, 0), n(r, 6) == 1);
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
