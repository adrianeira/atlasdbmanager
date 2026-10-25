//! Diagrama ER do SQLite: as tabelas (com colunas, chave primária e chave estrangeira) e as chaves estrangeiras entre elas.

use super::{length_decimal, map_err};
use atlas_core::{DatabaseError, DiagramColumn, DiagramRelation, DiagramTable, SchemaDiagram};
use rusqlite::Connection;

pub(crate) fn schema_diagram_on(conn: &Connection) -> Result<SchemaDiagram, DatabaseError> {
    let names: Vec<String> = {
        let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name").map_err(map_err)?;
        stmt.query_map([], |r| r.get(0)).map_err(map_err)?.collect::<Result<_, _>>().map_err(map_err)?
    };
    let mut tables = Vec::with_capacity(names.len());
    let mut relations = Vec::new();
    for table in &names {
        let quoted = format!("'{}'", table.replace('\'', "''"));
        // (de, para, tabela referenciada, coluna referenciada) de cada coluna de chave estrangeira, agrupadas pelo id da chave.
        let fk_rows: Vec<(i64, String, String, Option<String>)> = {
            let mut stmt = conn.prepare(&format!("SELECT id, \"table\", \"from\", \"to\" FROM pragma_foreign_key_list({quoted}) ORDER BY id, seq")).map_err(map_err)?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(map_err)?.collect::<Result<_, _>>().map_err(map_err)?
        };
        let cols: Vec<(String, String, bool, i64)> = {
            let mut stmt = conn.prepare(&format!("SELECT name, type, \"notnull\", pk FROM pragma_table_xinfo({quoted}) WHERE hidden IN (0, 2, 3) ORDER BY cid")).map_err(map_err)?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0, r.get(3)?))).map_err(map_err)?.collect::<Result<_, _>>().map_err(map_err)?
        };
        let columns = cols
            .iter()
            .map(|(name, decl, _, pk)| {
                let (length, decimal) = length_decimal(decl);
                DiagramColumn { name: name.clone(), data_type: decl.clone(), length, decimal, is_pk: *pk > 0, is_fk: fk_rows.iter().any(|f| &f.2 == name) }
            })
            .collect();
        tables.push(DiagramTable { name: table.clone(), columns });

        let mut ids: Vec<i64> = fk_rows.iter().map(|f| f.0).collect();
        ids.dedup();
        for id in ids {
            let part: Vec<_> = fk_rows.iter().filter(|f| f.0 == id).collect();
            let from: Vec<String> = part.iter().map(|f| f.2.clone()).collect();
            relations.push(DiagramRelation {
                name: format!("fk_{table}_{id}"),
                table: table.clone(),
                required: from.iter().all(|c| cols.iter().any(|(n, _, not_null, _)| n == c && *not_null)),
                columns: from,
                ref_schema: "main".into(),
                ref_table: part[0].1.clone(),
                ref_columns: part.iter().map(|f| f.3.clone().unwrap_or_default()).collect(),
            });
        }
    }
    Ok(SchemaDiagram { tables, relations })
}
