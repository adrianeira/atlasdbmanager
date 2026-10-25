//! Dump SQL do SQLite: os `CREATE` guardados em `sqlite_master` (na ordem em que foram criados) e a lista das tabelas cujos dados entram.

use super::{map_err, quote_ident};
use atlas_core::DatabaseError;
use rusqlite::Connection;

#[derive(Debug, Default)]
pub struct DumpPlan {
    /// Tabelas e views (e o `DROP` antes de cada uma, com `clean`).
    pub pre_data: Vec<String>,
    /// Índices e gatilhos: depois dos dados, para a carga não pagar o custo deles.
    pub post_data: Vec<String>,
    /// Tabelas cujos dados entram.
    pub data_tables: Vec<String>,
    /// Quantos objetos o dump tem (tabelas, views, índices e gatilhos).
    pub objects: usize,
}

pub(crate) fn plan_on(conn: &Connection, tables: Option<&[String]>, views: Option<&[String]>, clean: bool) -> Result<DumpPlan, DatabaseError> {
    let mut stmt = conn.prepare("SELECT type, name, tbl_name, sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY rowid").map_err(map_err)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))
        .map_err(map_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_err)?;
    let wanted = |list: Option<&[String]>, name: &str| list.is_none_or(|l| l.iter().any(|n| n == name));
    let mut plan = DumpPlan::default();
    for (kind, name, owner, sql) in rows {
        match kind.as_str() {
            "table" if wanted(tables, &name) => {
                if clean {
                    plan.pre_data.push(format!("DROP TABLE IF EXISTS {};", quote_ident(&name)));
                }
                plan.pre_data.push(format!("{sql};"));
                plan.data_tables.push(name);
                plan.objects += 1;
            }
            "view" if wanted(views, &name) => {
                if clean {
                    plan.pre_data.push(format!("DROP VIEW IF EXISTS {};", quote_ident(&name)));
                }
                plan.pre_data.push(format!("{sql};"));
                plan.objects += 1;
            }
            // Índices e gatilhos vão com a tabela deles.
            "index" | "trigger" if wanted(tables, &owner) => {
                plan.post_data.push(format!("{sql};"));
                plan.objects += 1;
            }
            _ => {}
        }
    }
    Ok(plan)
}
