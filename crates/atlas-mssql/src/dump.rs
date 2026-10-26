//! Dump SQL do SQL Server: o DDL lido do catálogo (tabelas, chaves, índices, views e procedimentos) e os dados como `INSERT`,
//! num script que o `sqlcmd` e o Atlas executam de volta.

use super::{MssqlSession, quote_ident};
use crate::meta::{os, rows_of};
use atlas_core::{ColumnInfo, DatabaseError, TableDetail};

/// O que entra no dump.
#[derive(Debug, Default)]
pub struct DumpPlan {
    /// Tabelas (e o `DROP` antes de cada uma, com `clean`).
    pub pre_data: Vec<String>,
    /// Chaves estrangeiras, índices, views e procedimentos: depois dos dados.
    pub post_data: Vec<String>,
    /// Tabelas cujos dados entram.
    pub data_tables: Vec<DumpTable>,
    /// Quantos objetos o dump tem.
    pub objects: usize,
}

#[derive(Debug, Clone)]
pub struct DumpTable {
    pub schema: String,
    pub name: String,
    /// Colunas que recebem dados (sem as calculadas), com o tipo.
    pub columns: Vec<(String, String)>,
    pub has_identity: bool,
}

fn check_expression(definition: &str) -> String {
    definition.strip_prefix("CHECK ((").and_then(|d| d.strip_suffix("))")).unwrap_or(definition).to_string()
}

fn column_line(c: &ColumnInfo) -> String {
    let mut line = format!("{} {}", quote_ident(&c.name), c.formatted_type);
    if c.identity.is_some() {
        line.push_str(" IDENTITY(1,1)");
    }
    line.push_str(if c.not_null { " NOT NULL" } else { " NULL" });
    if let Some(d) = &c.default {
        line.push_str(&format!(" DEFAULT ({d})"));
    }
    line
}

fn fields(list: &str) -> String {
    list.split(", ").map(quote_ident).collect::<Vec<_>>().join(", ")
}

fn create_table(schema: &str, table: &str, d: &TableDetail) -> String {
    let target = format!("{}.{}", quote_ident(schema), quote_ident(table));
    let mut lines: Vec<String> = d.columns.iter().filter(|c| c.generated.is_none()).map(column_line).collect();
    let mut keys: Vec<&ColumnInfo> = d.columns.iter().filter(|c| c.key.is_some()).collect();
    keys.sort_by_key(|c| c.key);
    if !keys.is_empty() {
        let name = d.indexes.iter().find(|i| i.primary).map_or(format!("PK_{table}"), |i| i.name.clone());
        lines.push(format!("CONSTRAINT {} PRIMARY KEY ({})", quote_ident(&name), keys.iter().map(|k| quote_ident(&k.name)).collect::<Vec<_>>().join(", ")));
    }
    for c in d.constraints.iter().filter(|c| c.kind == "u") {
        lines.push(format!("CONSTRAINT {} UNIQUE ({})", quote_ident(&c.name), fields(&c.fields)));
    }
    for c in d.constraints.iter().filter(|c| c.kind == "c") {
        lines.push(format!("CONSTRAINT {} CHECK ({})", quote_ident(&c.name), check_expression(&c.definition)));
    }
    format!("CREATE TABLE {target} (\n  {}\n);", lines.join(",\n  "))
}

fn post_table(schema: &str, table: &str, d: &TableDetail) -> Vec<String> {
    let target = format!("{}.{}", quote_ident(schema), quote_ident(table));
    let mut out = Vec::new();
    for c in d.constraints.iter().filter(|c| c.kind == "f") {
        let mut sql = format!(
            "ALTER TABLE {target} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {}.{} ({})",
            quote_ident(&c.name),
            fields(&c.fields),
            quote_ident(c.ref_schema.as_deref().unwrap_or("dbo")),
            quote_ident(c.ref_table.as_deref().unwrap_or("")),
            fields(c.ref_fields.as_deref().unwrap_or(""))
        );
        if let Some(a) = c.on_delete.as_deref().filter(|a| *a != "NO ACTION") {
            sql.push_str(&format!(" ON DELETE {a}"));
        }
        if let Some(a) = c.on_update.as_deref().filter(|a| *a != "NO ACTION") {
            sql.push_str(&format!(" ON UPDATE {a}"));
        }
        out.push(format!("{sql};"));
    }
    for i in d.indexes.iter().filter(|i| !i.primary && !d.constraints.iter().any(|c| c.kind == "u" && c.name == i.name)) {
        out.push(format!("CREATE {}INDEX {} ON {target} ({});", if i.unique { "UNIQUE " } else { "" }, quote_ident(&i.name), fields(&i.fields)));
    }
    out
}

impl MssqlSession {
    /// O plano do dump: `schema` (ou todos), `tables` e `views` escolhidos (`None` = todos) e `clean` para o `DROP` antes de cada objeto.
    pub async fn dump_plan(&self, database: &str, schema: Option<&str>, tables: Option<&[String]>, views: Option<&[String]>, functions: Option<&[String]>, clean: bool) -> Result<DumpPlan, DatabaseError> {
        let schemas: Vec<String> = match schema {
            Some(sc) => vec![sc.to_string()],
            None => self.list_schemas(database, false).await?.into_iter().map(|x| x.name).collect(),
        };
        let wanted = |list: Option<&[String]>, name: &str| list.is_none_or(|l| l.iter().any(|n| n == name));
        let mut plan = DumpPlan::default();
        for sc in &schemas {
            let objects = self.list_objects(database, sc).await?;
            for t in objects.tables.iter().filter(|t| wanted(tables, &t.name)) {
                let d = self.table_detail(database, sc, &t.name).await?;
                if clean {
                    plan.pre_data.push(format!("DROP TABLE IF EXISTS {}.{};", quote_ident(sc), quote_ident(&t.name)));
                }
                plan.pre_data.push(create_table(sc, &t.name, &d));
                plan.post_data.extend(post_table(sc, &t.name, &d));
                plan.data_tables.push(DumpTable {
                    schema: sc.clone(),
                    name: t.name.clone(),
                    columns: d.columns.iter().filter(|c| c.generated.is_none()).map(|c| (c.name.clone(), c.data_type.clone())).collect(),
                    has_identity: d.columns.iter().any(|c| c.identity.is_some()),
                });
                plan.objects += 1;
            }
            let client = self.client(database).await?;
            let mut conn = client.lock().await;
            for (list, wanted_list) in [( objects.views.iter().map(|v| v.name.clone()).collect::<Vec<_>>(), views), (objects.functions.iter().map(|f| f.name.clone()).collect(), functions)] {
                let base = |x: &str| x.split('(').next().unwrap_or(x).to_string();
                for name in list.iter().filter(|n| wanted_list.is_none_or(|l| l.iter().any(|x| base(x) == base(n)))) {
                    let fnname = name.split('(').next().unwrap_or(name);
                    let object = format!("{}.{}", quote_ident(sc), quote_ident(fnname));
                    let rows = rows_of(&mut conn, "SELECT CAST(OBJECT_DEFINITION(OBJECT_ID(@P1)) AS nvarchar(max))", &[&object]).await?;
                    let Some(def) = rows.first().and_then(|r| os(r, 0)) else { continue };
                    // Cada view ou procedimento precisa ser o primeiro comando do lote: o separador é uma linha `GO`.
                    plan.post_data.push(format!("GO\n{}\nGO", def.trim_end().trim_end_matches(';')));
                    plan.objects += 1;
                }
            }
        }
        Ok(plan)
    }
}

/// O valor como literal de `INSERT` do SQL Server: número cru, `bit` como 1/0, texto com `N'...'`, binário em hexa.
pub fn literal(v: Option<&str>, sql_type: &str) -> String {
    let Some(v) = v else { return "NULL".into() };
    match sql_type.to_ascii_lowercase().as_str() {
        "bit" => if v == "t" || v == "1" { "1" } else { "0" }.into(),
        "tinyint" | "smallint" | "int" | "bigint" | "decimal" | "numeric" | "float" | "real" | "money" | "smallmoney" => v.to_string(),
        "varbinary" | "binary" | "image" | "timestamp" => format!("0x{}", v.trim_start_matches("\\x")),
        _ => format!("N'{}'", v.replace('\'', "''")),
    }
}
