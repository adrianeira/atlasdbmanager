//! Dump SQL do MySQL: o DDL vem de `SHOW CREATE` (tabelas, views, funções e procedimentos) e os dados como `INSERT`, num script
//! que o cliente `mysql` e o Atlas executam de volta. As chaves estrangeiras ficam dentro do `CREATE TABLE`; o script desliga
//! `FOREIGN_KEY_CHECKS` enquanto cria e carrega.

use super::{MysqlSession, quote_ident};
use crate::meta::{rows_of, text};
use atlas_core::DatabaseError;

/// O que entra no dump.
#[derive(Debug, Default)]
pub struct DumpPlan {
    /// Tabelas (e o `DROP` antes de cada uma, com `clean`).
    pub pre_data: Vec<String>,
    /// Views, funções e procedimentos: depois dos dados.
    pub post_data: Vec<String>,
    /// Tabelas cujos dados entram.
    pub data_tables: Vec<DumpTable>,
    /// Quantos objetos o dump tem.
    pub objects: usize,
}

#[derive(Debug, Clone)]
pub struct DumpTable {
    pub name: String,
    /// Colunas que recebem dados (sem as calculadas), com o tipo.
    pub columns: Vec<(String, String)>,
}

/// Tira o `DEFINER=...` (o usuário pode não existir onde o script rodar).
fn without_definer(sql: &str) -> String {
    let Some(start) = sql.find("DEFINER=") else { return sql.to_string() };
    // `DEFINER=`user`@`host` ` termina no primeiro espaço fora das crases.
    let mut in_quote = false;
    let mut end = None;
    for (i, ch) in sql[start..].char_indices() {
        match ch {
            '`' => in_quote = !in_quote,
            ' ' if !in_quote => {
                end = Some(start + i + 1);
                break;
            }
            _ => {}
        }
    }
    match end {
        Some(e) => format!("{}{}", &sql[..start], &sql[e..]),
        None => sql.to_string(),
    }
}

impl MysqlSession {
    /// O plano do dump do banco: `tables` e `views` escolhidos (`None` = todos) e `clean` para o `DROP` antes de cada objeto.
    pub async fn dump_plan(&self, database: &str, tables: Option<&[String]>, views: Option<&[String]>, clean: bool) -> Result<DumpPlan, DatabaseError> {
        let objects = self.list_objects(database).await?;
        let wanted = |list: Option<&[String]>, name: &str| list.is_none_or(|l| l.iter().any(|n| n == name));
        let mut plan = DumpPlan::default();
        for t in objects.tables.iter().filter(|t| wanted(tables, &t.name)) {
            let d = self.table_detail(database, &t.name).await?;
            let client = self.client(database).await?;
            let mut conn = client.lock().await;
            let rows = rows_of(&mut conn, &format!("SHOW CREATE TABLE {}", quote_ident(&t.name)), vec![]).await?;
            let Some(ddl) = rows.first().and_then(|r| text(r, 1)) else { continue };
            if clean {
                plan.pre_data.push(format!("DROP TABLE IF EXISTS {};", quote_ident(&t.name)));
            }
            plan.pre_data.push(format!("{ddl};"));
            plan.data_tables.push(DumpTable { name: t.name.clone(), columns: d.columns.iter().filter(|c| c.generated.is_none()).map(|c| (c.name.clone(), c.data_type.clone())).collect() });
            plan.objects += 1;
        }
        let client = self.client(database).await?;
        let mut conn = client.lock().await;
        for v in objects.views.iter().filter(|v| wanted(views, &v.name)) {
            let rows = rows_of(&mut conn, &format!("SHOW CREATE VIEW {}", quote_ident(&v.name)), vec![]).await?;
            let Some(ddl) = rows.first().and_then(|r| text(r, 1)) else { continue };
            if clean {
                plan.post_data.push(format!("DROP VIEW IF EXISTS {};", quote_ident(&v.name)));
            }
            plan.post_data.push(format!("{};", without_definer(&ddl)));
            plan.objects += 1;
        }
        if views.is_none() {
            for f in &objects.functions {
                let kind = if f.procedure { "PROCEDURE" } else { "FUNCTION" };
                let rows = rows_of(&mut conn, &format!("SHOW CREATE {kind} {}", quote_ident(&f.name)), vec![]).await?;
                let Some(ddl) = rows.first().and_then(|r| text(r, 2)) else { continue };
                if clean {
                    plan.post_data.push(format!("DROP {kind} IF EXISTS {};", quote_ident(&f.name)));
                }
                plan.post_data.push(format!("{};", without_definer(&ddl)));
                plan.objects += 1;
            }
        }
        Ok(plan)
    }
}

/// O valor como literal de `INSERT` do MySQL: número cru, `BIT` como 1/0, texto entre aspas (barra e aspa escapadas), binário em hexa.
pub fn literal(v: Option<&str>, sql_type: &str) -> String {
    let Some(v) = v else { return "NULL".into() };
    match sql_type.to_ascii_lowercase().as_str() {
        "bit" => if v == "t" || v == "1" { "1" } else { "0" }.into(),
        "tinyint" | "smallint" | "mediumint" | "int" | "bigint" | "decimal" | "numeric" | "float" | "double" | "year" => v.to_string(),
        "blob" | "tinyblob" | "mediumblob" | "longblob" | "binary" | "varbinary" | "geometry" | "point" | "linestring" | "polygon" => {
            let hex = v.trim_start_matches("\\x");
            if hex.is_empty() { "''".into() } else { format!("0x{hex}") }
        }
        _ => format!("'{}'", v.replace('\\', "\\\\").replace('\'', "''")),
    }
}
