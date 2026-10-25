//! Rótulos de chave estrangeira e diagrama ER do SQL Server.

use super::{MssqlSession, quote_ident};
use crate::meta::{b, n, rows_of, s};
use atlas_core::{DatabaseError, DiagramColumn, DiagramRelation, DiagramTable, SchemaDiagram};
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

/// Escolhe a coluna de descrição: nomes típicos primeiro (`nome`, `name`, `descricao`...), depois a primeira coluna de texto
/// que não seja a própria chave nem pareça outra chave (`..._id`, `id_...`, `codigo`).
fn pick_label_column(columns: &[(String, String)], key: &str) -> Option<String> {
    let textual = |t: &str| matches!(t, "varchar" | "nvarchar" | "char" | "nchar" | "text" | "ntext");
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

impl MssqlSession {
    pub async fn fk_labels(&self, database: &str, schema: &str, table: &str, key_column: &str, keys: &[String]) -> Result<FkLabels, DatabaseError> {
        let none = FkLabels { label_column: None, labels: Vec::new() };
        if keys.is_empty() {
            return Ok(none);
        }
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let target = format!("{}.{}", quote_ident(schema), quote_ident(table));
        let cols = rows_of(&mut conn, "SELECT c.name, ty.name FROM sys.columns c JOIN sys.types ty ON ty.user_type_id = c.user_type_id WHERE c.object_id = OBJECT_ID(@P1) ORDER BY c.column_id", &[&target]).await?;
        let columns: Vec<(String, String)> = cols.iter().map(|r| (s(r, 0), s(r, 1))).collect();
        let Some(label) = pick_label_column(&columns, key_column) else { return Ok(none) };
        let mut labels = Vec::new();
        // O SQL Server aceita até 2100 parâmetros por comando.
        for chunk in keys.iter().take(MAX_FK_KEYS).collect::<Vec<_>>().chunks(1000) {
            let marks = (1..=chunk.len()).map(|i| format!("@P{i}")).collect::<Vec<_>>().join(", ");
            let sql = format!(
                "SELECT CAST({k} AS nvarchar(4000)), CAST({l} AS nvarchar(4000)) FROM {target} WHERE CAST({k} AS nvarchar(4000)) IN ({marks}) AND {l} IS NOT NULL",
                k = quote_ident(key_column),
                l = quote_ident(&label)
            );
            let mut q = tiberius::Query::new(sql);
            for k in chunk {
                q.bind((*k).clone());
            }
            let rows = q.query(&mut conn).await.map_err(super::map_err)?.into_first_result().await.map_err(super::map_err)?;
            labels.extend(rows.iter().map(|r| [s(r, 0), s(r, 1)]));
        }
        Ok(FkLabels { label_column: Some(label), labels })
    }

    /// Tabelas (com colunas) e chaves estrangeiras de um schema, para o diagrama ER.
    pub async fn schema_diagram(&self, database: &str, schema: &str) -> Result<SchemaDiagram, DatabaseError> {
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        let cols = rows_of(
            &mut conn,
            "SELECT t.name, c.name, ty.name, CAST(c.max_length AS int), CAST(c.precision AS int), CAST(c.scale AS int), \
                    CAST(CASE WHEN EXISTS (SELECT 1 FROM sys.index_columns ic JOIN sys.indexes i ON i.object_id = ic.object_id AND i.index_id = ic.index_id WHERE i.is_primary_key = 1 AND ic.object_id = c.object_id AND ic.column_id = c.column_id) THEN 1 ELSE 0 END AS bit), \
                    CAST(CASE WHEN EXISTS (SELECT 1 FROM sys.foreign_key_columns fc WHERE fc.parent_object_id = c.object_id AND fc.parent_column_id = c.column_id) THEN 1 ELSE 0 END AS bit) \
               FROM sys.tables t JOIN sys.schemas sc ON sc.schema_id = t.schema_id JOIN sys.columns c ON c.object_id = t.object_id JOIN sys.types ty ON ty.user_type_id = c.user_type_id \
              WHERE sc.name = @P1 AND t.is_ms_shipped = 0 ORDER BY t.name, c.column_id",
            &[&schema],
        )
        .await?;
        let mut tables: Vec<DiagramTable> = Vec::new();
        for r in &cols {
            let table = s(r, 0);
            let (data_type, max_length, precision, scale) = (s(r, 2), n(r, 3), n(r, 4), n(r, 5));
            let (length, decimal) = match data_type.as_str() {
                "nvarchar" | "nchar" => (if max_length < 0 { 0 } else { max_length / 2 }, 0),
                "varchar" | "char" | "varbinary" | "binary" => (max_length.max(0), 0),
                "decimal" | "numeric" => (precision, scale),
                _ => (0, 0),
            };
            let column = DiagramColumn { name: s(r, 1), data_type, length, decimal, is_pk: b(r, 6), is_fk: b(r, 7) };
            match tables.last_mut() {
                Some(t) if t.name == table => t.columns.push(column),
                _ => tables.push(DiagramTable { name: table, columns: vec![column] }),
            }
        }
        let fks = rows_of(
            &mut conn,
            "SELECT fk.name, t.name, rs.name, rt.name, fkc.constraint_column_id, pc.name, rc.name, CAST(pc.is_nullable AS bit) \
               FROM sys.foreign_keys fk JOIN sys.tables t ON t.object_id = fk.parent_object_id JOIN sys.schemas sc ON sc.schema_id = t.schema_id \
               JOIN sys.tables rt ON rt.object_id = fk.referenced_object_id JOIN sys.schemas rs ON rs.schema_id = rt.schema_id \
               JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id \
               JOIN sys.columns pc ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id \
               JOIN sys.columns rc ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id \
              WHERE sc.name = @P1 ORDER BY t.name, fk.name, fkc.constraint_column_id",
            &[&schema],
        )
        .await?;
        let mut relations: Vec<DiagramRelation> = Vec::new();
        for r in &fks {
            let (name, nullable) = (s(r, 0), b(r, 7));
            match relations.last_mut() {
                Some(rel) if rel.name == name && rel.table == s(r, 1) => {
                    rel.columns.push(s(r, 5));
                    rel.ref_columns.push(s(r, 6));
                    rel.required = rel.required && !nullable;
                }
                _ => relations.push(DiagramRelation { name, table: s(r, 1), columns: vec![s(r, 5)], ref_schema: s(r, 2), ref_table: s(r, 3), ref_columns: vec![s(r, 6)], required: !nullable }),
            }
        }
        Ok(SchemaDiagram { tables, relations })
    }
}
