//! Structure Synchronization do SQL Server, MySQL e SQLite: compara a estrutura de dois schemas do MESMO produto (tabelas com colunas,
//! chaves, restrições, índices e gatilhos; views; rotinas) e gera o DDL que deixa o destino igual à origem.
//!
//! Os formatos de resposta são os do PostgreSQL (`structsync.rs`). Cada mudança vira comandos marcados com a fase em que rodam
//! (primeiro o que se apaga, depois tabelas e colunas, depois chaves e índices, por fim chaves estrangeiras, views e rotinas) e com
//! a parte (coluna, chave...) a que pertencem, para a tela escolher o que implantar.

use crate::datasync::{DeployCounts, DeployProgress, LogEvent, ProgressEvent};
use crate::dsync::{deploy, register_cancel, unregister_cancel};
use crate::genext::{Dialect, Exec, exec_for, mssql_schema_rewrite};
use crate::mssql::MssqlSessions;
use crate::mysql::MysqlSessions;
use crate::sqlite::SqliteSessions;
use crate::structsync::{ObjectOut, StructCompareRequest, StructDeployRequest};
use atlas_core::{ColumnInfo, ConstraintInfo, DatabaseError, ErrorKind, IndexInfo, TableDetail};
use atlas_pg::{StructOp, StructPart, StructSelection};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

struct Stmt {
    phase: u8,
    order: i64,
    part: Option<usize>,
    sql: String,
}

struct Diff {
    kind: &'static str,
    name: String,
    op: StructOp,
    parts: Vec<StructPart>,
    source_ddl: String,
    target_ddl: String,
    stmts: Vec<Stmt>,
}

#[derive(Default)]
pub struct ExtStructComparisons {
    inner: Mutex<HashMap<String, Arc<Vec<Diff>>>>,
}

fn canceled() -> DatabaseError {
    DatabaseError::new(ErrorKind::Canceled, "synchronization canceled")
}

fn list(fields: &str, d: Dialect) -> String {
    fields.split(", ").filter(|f| !f.is_empty()).map(|f| d.q(f.trim_matches(['`', '"', '[', ']']))).collect::<Vec<_>>().join(", ")
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A definição da coluna como o banco a escreve (tipo, nulidade, padrão, numeração automática).
fn column_def(d: Dialect, c: &ColumnInfo, with_name: bool) -> String {
    let mut s = if with_name { format!("{} {}", d.q(&c.name), c.formatted_type) } else { c.formatted_type.clone() };
    if d == Dialect::Mssql && c.identity.is_some() {
        s.push_str(" IDENTITY(1,1)");
    }
    s.push_str(if c.not_null { " NOT NULL" } else if d == Dialect::Mssql { " NULL" } else { "" });
    if let Some(def) = &c.default {
        s.push_str(&if d == Dialect::Mssql { format!(" DEFAULT ({def})") } else { format!(" DEFAULT {def}") });
    }
    if d == Dialect::Mysql && c.identity.is_some() {
        s.push_str(" AUTO_INCREMENT");
    }
    s
}

fn same_column(d: Dialect, a: &ColumnInfo, b: &ColumnInfo) -> bool {
    norm(&a.formatted_type) == norm(&b.formatted_type)
        && a.not_null == b.not_null
        && a.default.as_deref().map(norm) == b.default.as_deref().map(norm)
        // O SQL Server não troca a numeração automática de uma coluna que já existe.
        && (d == Dialect::Mssql || a.identity.is_some() == b.identity.is_some())
}

fn check_expr(def: &str) -> String {
    def.trim().strip_prefix("CHECK ((").and_then(|d| d.strip_suffix("))")).or_else(|| def.trim().strip_prefix("CHECK (").and_then(|d| d.strip_suffix(')'))).unwrap_or(def).to_string()
}

fn pk_name(t: &TableDetail) -> Option<String> {
    t.indexes.iter().find(|i| i.primary).map(|i| i.name.clone())
}

fn pk_columns(t: &TableDetail) -> String {
    let mut keys: Vec<(i32, &str)> = t.columns.iter().filter_map(|c| c.key.map(|k| (k, c.name.as_str()))).collect();
    keys.sort();
    keys.into_iter().map(|(_, n)| n).collect::<Vec<_>>().join(", ")
}

/// Apaga a restrição `DEFAULT` (de nome gerado) de uma coluna do SQL Server.
fn mssql_drop_default(target: &str, column: &str) -> String {
    let t = Dialect::Mssql.text(target);
    let c = Dialect::Mssql.text(column);
    format!(
        "DECLARE @dc sysname = (SELECT dc.name FROM sys.default_constraints dc JOIN sys.columns c ON c.object_id = dc.parent_object_id AND c.column_id = dc.parent_column_id \
         WHERE dc.parent_object_id = OBJECT_ID({t}) AND c.name = {c}); IF @dc IS NOT NULL BEGIN DECLARE @sql nvarchar(max) = N'ALTER TABLE ' + {t} + N' DROP CONSTRAINT ' + QUOTENAME(@dc); EXEC(@sql); END;"
    )
}

fn fk_sql(d: Dialect, target: &str, c: &ConstraintInfo, src_schema: &str, dst_schema: &str) -> String {
    let rs = c.ref_schema.clone().filter(|s| s != src_schema).unwrap_or_else(|| dst_schema.to_string());
    let rt = c.ref_table.clone().unwrap_or_default();
    let mut sql = format!(
        "ALTER TABLE {target} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({})",
        d.q(&c.name),
        list(&c.fields, d),
        d.target(&rs, &rt),
        list(c.ref_fields.as_deref().unwrap_or(""), d)
    );
    if let Some(a) = c.on_delete.as_deref().filter(|a| *a != "NO ACTION" && *a != "RESTRICT") {
        sql.push_str(&format!(" ON DELETE {a}"));
    }
    if let Some(a) = c.on_update.as_deref().filter(|a| *a != "NO ACTION" && *a != "RESTRICT") {
        sql.push_str(&format!(" ON UPDATE {a}"));
    }
    sql
}

fn drop_constraint(d: Dialect, target: &str, c: &ConstraintInfo) -> String {
    match (d, c.kind.as_str()) {
        (Dialect::Mysql, "f") => format!("ALTER TABLE {target} DROP FOREIGN KEY {}", d.q(&c.name)),
        (Dialect::Mysql, "c") => format!("ALTER TABLE {target} DROP CHECK {}", d.q(&c.name)),
        (Dialect::Mysql, _) => format!("ALTER TABLE {target} DROP INDEX {}", d.q(&c.name)),
        _ => format!("ALTER TABLE {target} DROP CONSTRAINT {}", d.q(&c.name)),
    }
}

fn add_constraint(d: Dialect, target: &str, c: &ConstraintInfo, src_schema: &str, dst_schema: &str) -> String {
    match c.kind.as_str() {
        "f" => fk_sql(d, target, c, src_schema, dst_schema),
        "c" => format!("ALTER TABLE {target} ADD CONSTRAINT {} CHECK ({})", d.q(&c.name), check_expr(&c.definition)),
        _ => format!("ALTER TABLE {target} ADD CONSTRAINT {} UNIQUE ({})", d.q(&c.name), list(&c.fields, d)),
    }
}

fn same_constraint(a: &ConstraintInfo, b: &ConstraintInfo, src_schema: &str, dst_schema: &str) -> bool {
    a.kind == b.kind
        && norm(&a.fields) == norm(&b.fields)
        && a.ref_table == b.ref_table
        && norm(a.ref_fields.as_deref().unwrap_or("")) == norm(b.ref_fields.as_deref().unwrap_or(""))
        && a.on_delete == b.on_delete
        && a.on_update == b.on_update
        && (a.kind != "c" || norm(&check_expr(&a.definition)) == norm(&check_expr(&b.definition)))
        && (a.ref_schema.as_deref() == b.ref_schema.as_deref() || (a.ref_schema.as_deref() == Some(src_schema) && b.ref_schema.as_deref() == Some(dst_schema)))
}

fn index_sql(d: Dialect, target: &str, i: &IndexInfo) -> String {
    format!("CREATE {}INDEX {} ON {target} ({})", if i.unique { "UNIQUE " } else { "" }, d.q(&i.name), list(&i.fields, d))
}

fn drop_index(d: Dialect, target: &str, name: &str) -> String {
    match d {
        Dialect::Sqlite => format!("DROP INDEX {}", d.q(name)),
        _ => format!("DROP INDEX {} ON {target}", d.q(name)),
    }
}

/// Cada ponta do diff de uma tabela: as partes (para a tela) e os comandos.
struct TableDiff {
    parts: Vec<StructPart>,
    stmts: Vec<Stmt>,
}

impl TableDiff {
    fn add(&mut self, kind: &str, name: &str, op: StructOp, phase: u8, order: i64, sql: Vec<String>) {
        self.parts.push(StructPart { kind: kind.into(), name: name.into(), op });
        let part = Some(self.parts.len() - 1);
        for s in sql {
            self.stmts.push(Stmt { phase, order, part, sql: s });
        }
    }
}

/// O que muda numa tabela que existe nos dois lados.
#[allow(clippy::too_many_arguments)]
fn diff_table(d: Dialect, src: &TableDetail, dst: &TableDetail, target: &str, name: &str, src_schema: &str, dst_schema: &str, rebuild_ddl: &[String]) -> TableDiff {
    let mut out = TableDiff { parts: Vec::new(), stmts: Vec::new() };
    let mut order = 0i64;
    let mut next = || {
        order += 1;
        order
    };
    // SQLite não altera nem apaga coluna, nem mexe em chaves: qualquer dessas mudanças recria a tabela.
    let sqlite_rebuild_needed = d == Dialect::Sqlite
        && (dst.columns.iter().any(|x| !src.columns.iter().any(|c| c.name == x.name))
            || src.columns.iter().any(|c| dst.columns.iter().any(|x| x.name == c.name && !same_column(d, c, x)))
            || pk_columns(src) != pk_columns(dst)
            || src.constraints.iter().any(|c| !dst.constraints.iter().any(|x| same_constraint(c, x, src_schema, dst_schema)))
            || dst.constraints.iter().any(|c| !src.constraints.iter().any(|x| same_constraint(x, c, src_schema, dst_schema))));
    if sqlite_rebuild_needed {
        let n = next();
        out.add("options", name, StructOp::Modify, 4, n, rebuild_ddl.to_vec());
    } else {
        // Colunas.
        for c in &dst.columns {
            if !src.columns.iter().any(|s| s.name == c.name) {
                let n = next();
                let mut sql = Vec::new();
                if d == Dialect::Mssql {
                    sql.push(mssql_drop_default(target, &c.name));
                }
                sql.push(format!("ALTER TABLE {target} DROP COLUMN {}", d.q(&c.name)));
                out.add("column", &c.name, StructOp::Delete, 4, n, sql);
            }
        }
        for c in &src.columns {
            match dst.columns.iter().find(|x| x.name == c.name) {
                None => {
                    let n = next();
                    let kw = if d == Dialect::Mssql { "ADD" } else { "ADD COLUMN" };
                    out.add("column", &c.name, StructOp::Add, 4, n, vec![format!("ALTER TABLE {target} {kw} {}", column_def(d, c, true))]);
                }
                Some(x) if !same_column(d, c, x) && d != Dialect::Sqlite => {
                    let n = next();
                    let sql = match d {
                        Dialect::Mysql => vec![format!("ALTER TABLE {target} MODIFY COLUMN {}", column_def(d, c, true))],
                        _ => {
                            let mut v = vec![format!("ALTER TABLE {target} ALTER COLUMN {} {}{}", d.q(&c.name), c.formatted_type, if c.not_null { " NOT NULL" } else { " NULL" })];
                            if c.default.as_deref().map(norm) != x.default.as_deref().map(norm) {
                                v.push(mssql_drop_default(target, &c.name));
                                if let Some(def) = &c.default {
                                    v.push(format!("ALTER TABLE {target} ADD DEFAULT ({def}) FOR {}", d.q(&c.name)));
                                }
                            }
                            v
                        }
                    };
                    out.add("column", &c.name, StructOp::Modify, 4, n, sql);
                }
                _ => {}
            }
        }
        // Chave primária.
        if pk_columns(src) != pk_columns(dst) && d != Dialect::Sqlite {
            let n = next();
            let mut sql = Vec::new();
            if !pk_columns(dst).is_empty() {
                sql.push(match d {
                    Dialect::Mysql => format!("ALTER TABLE {target} DROP PRIMARY KEY"),
                    _ => format!("ALTER TABLE {target} DROP CONSTRAINT {}", d.q(&pk_name(dst).unwrap_or_default())),
                });
            }
            if !pk_columns(src).is_empty() {
                sql.push(format!("ALTER TABLE {target} ADD PRIMARY KEY ({})", list(&pk_columns(src), d)));
            }
            let op = if pk_columns(dst).is_empty() { StructOp::Add } else if pk_columns(src).is_empty() { StructOp::Delete } else { StructOp::Modify };
            out.add("primary", "PRIMARY", op, 5, n, sql);
        }
        // Restrições (o MySQL guarda as únicas como índices).
        if d != Dialect::Sqlite {
            let skip = |c: &&ConstraintInfo| !(d == Dialect::Mysql && c.kind == "u");
            for c in dst.constraints.iter().filter(skip) {
                match src.constraints.iter().find(|x| x.name == c.name) {
                    None => {
                        let n = next();
                        out.add(kind_name(c), &c.name, StructOp::Delete, 1, n, vec![drop_constraint(d, target, c)]);
                    }
                    Some(x) if !same_constraint(x, c, src_schema, dst_schema) => {
                        let n = next();
                        let phase = if c.kind == "f" { 6 } else { 5 };
                        out.add(kind_name(c), &c.name, StructOp::Modify, 1, n, vec![drop_constraint(d, target, c)]);
                        out.stmts.push(Stmt { phase, order: n, part: Some(out.parts.len() - 1), sql: add_constraint(d, target, x, src_schema, dst_schema) });
                    }
                    _ => {}
                }
            }
            for c in src.constraints.iter().filter(skip) {
                if !dst.constraints.iter().any(|x| x.name == c.name) {
                    let n = next();
                    out.add(kind_name(c), &c.name, StructOp::Add, if c.kind == "f" { 6 } else { 5 }, n, vec![add_constraint(d, target, c, src_schema, dst_schema)]);
                }
            }
        }
    }
    // Índices (a chave primária e as restrições únicas não entram: já foram tratadas). Na recriação do SQLite eles voltam com a tabela.
    if sqlite_rebuild_needed {
        return out;
    }
    let plain = |i: &&IndexInfo| !i.primary && !i.name.starts_with("sqlite_autoindex_") && !(d == Dialect::Mssql && src.constraints.iter().chain(dst.constraints.iter()).any(|c| c.kind == "u" && c.name == i.name));
    for i in dst.indexes.iter().filter(plain) {
        match src.indexes.iter().find(|x| x.name == i.name) {
            None => {
                let n = next();
                out.add("index", &i.name, StructOp::Delete, 1, n, vec![drop_index(d, target, &i.name)]);
            }
            Some(x) if norm(&x.fields) != norm(&i.fields) || x.unique != i.unique => {
                let n = next();
                out.add("index", &i.name, StructOp::Modify, 1, n, vec![drop_index(d, target, &i.name)]);
                out.stmts.push(Stmt { phase: 5, order: n, part: Some(out.parts.len() - 1), sql: index_sql(d, target, x) });
            }
            _ => {}
        }
    }
    for i in src.indexes.iter().filter(plain) {
        if !dst.indexes.iter().any(|x| x.name == i.name) {
            let n = next();
            out.add("index", &i.name, StructOp::Add, 5, n, vec![index_sql(d, target, i)]);
        }
    }
    // Gatilhos.
    {
        let drop_trigger = |t: &str| match d {
            Dialect::Mssql => format!("DROP TRIGGER {}", d.target(dst_schema, t)),
            _ => format!("DROP TRIGGER {}", d.q(t)),
        };
        let adapt = |sql: &str| if d == Dialect::Mssql { mssql_schema_rewrite(sql, src_schema, dst_schema) } else { sql.to_string() };
        for t in &dst.triggers {
            match src.triggers.iter().find(|x| x.name == t.name) {
                None => {
                    let n = next();
                    out.add("trigger", &t.name, StructOp::Delete, 1, n, vec![drop_trigger(&t.name)]);
                }
                Some(x) if norm(&x.definition) != norm(&t.definition) => {
                    let n = next();
                    out.add("trigger", &t.name, StructOp::Modify, 1, n, vec![drop_trigger(&t.name)]);
                    out.stmts.push(Stmt { phase: 7, order: n, part: Some(out.parts.len() - 1), sql: adapt(&x.definition) });
                }
                _ => {}
            }
        }
        for t in &src.triggers {
            if !dst.triggers.iter().any(|x| x.name == t.name) {
                let n = next();
                out.add("trigger", &t.name, StructOp::Add, 7, n, vec![adapt(&t.definition)]);
            }
        }
    }
    out
}

fn kind_name(c: &ConstraintInfo) -> &'static str {
    match c.kind.as_str() {
        "f" => "foreign",
        "c" => "check",
        _ => "unique",
    }
}

/// Texto de um objeto no banco: o DDL do plano de dump (sem os `GO` do SQL Server).
fn ddl_text(parts: &[String]) -> String {
    parts.iter().flat_map(|p| p.lines().filter(|l| !l.trim().eq_ignore_ascii_case("go")).map(str::to_string).collect::<Vec<_>>()).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// O plano de dump (DDL) de um objeto; `kind` é "table", "view" ou "function".
async fn object_ddl(side: &Side, kind: &str, name: &str) -> Result<(Vec<String>, Vec<String>), DatabaseError> {
    let one = [name.to_string()];
    let none: [String; 0] = [];
    let (tables, views, functions): (&[String], &[String], &[String]) = match kind {
        "table" => (&one, &none, &none),
        "view" => (&none, &one, &none),
        _ => (&none, &none, &one),
    };
    Ok(match &side.pair {
        Backend::Mysql(s) => {
            let p = s.dump_plan(&side.schema, Some(tables), Some(views), Some(functions), false).await?;
            (p.pre_data, p.post_data)
        }
        Backend::Mssql(s) => {
            let p = s.dump_plan(&side.database, Some(&side.schema), Some(tables), Some(views), Some(functions), false).await?;
            (p.pre_data, p.post_data)
        }
        Backend::Sqlite(s) => {
            let p = s.dump_plan(Some(tables), Some(views), false)?;
            (p.pre_data, p.post_data)
        }
    })
}

enum Backend {
    Mysql(Arc<atlas_mysql::MysqlSession>),
    Mssql(Arc<atlas_mssql::MssqlSession>),
    Sqlite(Arc<atlas_sqlite::SqliteSession>),
}

struct Side {
    pair: Backend,
    exec: Exec,
    database: String,
    schema: String,
}

fn backend_for(conn: &str, mysql: &MysqlSessions, mssql: &MssqlSessions, sqlite: &SqliteSessions) -> Result<Backend, DatabaseError> {
    if conn.starts_with("mysql_") {
        Ok(Backend::Mysql(mysql.session(conn)?))
    } else if conn.starts_with("mssql_") {
        Ok(Backend::Mssql(mssql.session(conn)?))
    } else {
        Ok(Backend::Sqlite(sqlite.session(conn)?))
    }
}

/// Os nomes de um tipo de objeto do schema.
async fn names(side: &Side, kind: &str) -> Result<Vec<String>, DatabaseError> {
    let o = side.exec.objects(&side.schema).await?;
    let list = match kind {
        "table" => o.tables,
        "view" => o.views,
        _ => o.functions,
    };
    Ok(list.into_iter().map(|x| x.name.split('(').next().unwrap_or(&x.name).to_string()).collect())
}

fn function_drop(d: Dialect, side: &Side, name: &str, is_proc: bool) -> String {
    let kind = if is_proc { "PROCEDURE" } else { "FUNCTION" };
    match d {
        Dialect::Mssql => format!("DROP {kind} {}", d.target(&side.schema, name)),
        _ => format!("DROP {kind} {}", d.q(name)),
    }
}

/// A criação como o destino a recebe (sem `GO`, com o schema trocado).
fn create_text(d: Dialect, parts: &[String], src_schema: &str, dst_schema: &str) -> Vec<String> {
    parts
        .iter()
        .map(|p| {
            let t = ddl_text(std::slice::from_ref(p));
            let t = if d == Dialect::Mssql { mssql_schema_rewrite(&t, src_schema, dst_schema) } else { t };
            t.trim_end_matches(';').to_string()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

async fn compare(app: &AppHandle, d: Dialect, src: &Side, dst: &Side, id: &str, cancel: &AtomicBool) -> Result<Vec<Diff>, DatabaseError> {
    let progress = |text: &str, f: f64| {
        let _ = app.emit("datasync-progress", ProgressEvent { id, text: text.to_string(), fraction: f });
    };
    let mut out: Vec<Diff> = Vec::new();
    let kinds: [&'static str; 3] = ["table", "view", "function"];
    for (ki, kind) in kinds.iter().enumerate() {
        let s_names = names(src, kind).await?;
        let t_names = names(dst, kind).await?;
        let mut all: Vec<String> = s_names.iter().chain(t_names.iter()).cloned().collect();
        all.sort();
        all.dedup();
        for (n, name) in all.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(canceled());
            }
            progress(name, (ki as f64 + n as f64 / all.len().max(1) as f64) / 3.0);
            let in_s = s_names.contains(name);
            let in_t = t_names.contains(name);
            let target = d.target(&dst.schema, name);
            let (s_pre, s_post) = if in_s { object_ddl(src, kind, name).await? } else { (Vec::new(), Vec::new()) };
            let (t_pre, t_post) = if in_t { object_ddl(dst, kind, name).await? } else { (Vec::new(), Vec::new()) };
            let source_ddl = ddl_text(&[s_pre.clone(), s_post.clone()].concat());
            let target_ddl = ddl_text(&[t_pre.clone(), t_post.clone()].concat());
            let mut diff = Diff { kind, name: name.clone(), op: StructOp::Same, parts: Vec::new(), source_ddl, target_ddl, stmts: Vec::new() };
            match (in_s, in_t) {
                (true, false) => {
                    diff.op = StructOp::Add;
                    let pre = create_text(d, &s_pre, &src.schema, &dst.schema);
                    let post = create_text(d, &s_post, &src.schema, &dst.schema);
                    match *kind {
                        "table" => {
                            for (i, sql) in pre.into_iter().enumerate() {
                                diff.stmts.push(Stmt { phase: 3, order: i as i64, part: None, sql });
                            }
                            for (i, sql) in post.into_iter().enumerate() {
                                let phase = if sql.to_uppercase().contains("FOREIGN KEY") { 6 } else { 5 };
                                diff.stmts.push(Stmt { phase, order: i as i64, part: None, sql });
                            }
                        }
                        _ => {
                            for (i, sql) in pre.into_iter().chain(post).enumerate() {
                                diff.stmts.push(Stmt { phase: 7, order: i as i64, part: None, sql });
                            }
                        }
                    }
                }
                (false, true) => {
                    diff.op = StructOp::Delete;
                    let sql = match *kind {
                        "table" => format!("DROP TABLE {target}"),
                        "view" => format!("DROP VIEW {target}"),
                        _ => {
                            let is_proc = dst.exec.objects(&dst.schema).await?.functions.iter().any(|f| f.name.split('(').next() == Some(name.as_str()) && f.procedure);
                            function_drop(d, dst, name, is_proc)
                        }
                    };
                    diff.stmts.push(Stmt { phase: if *kind == "table" { 2 } else { 1 }, order: 0, part: None, sql });
                }
                (true, true) => match *kind {
                    "table" => {
                        let sd = src.exec.table_detail(&src.schema, name).await?;
                        let td = dst.exec.table_detail(&dst.schema, name).await?;
                        // Recriação do SQLite: tabela nova com a estrutura da origem, dados copiados, troca de nomes e índices de volta.
                        let rebuild = if d == Dialect::Sqlite { sqlite_rebuild(&sd, &td, name, &s_pre, &s_post) } else { Vec::new() };
                        let t = diff_table(d, &sd, &td, &target, name, &src.schema, &dst.schema, &rebuild);
                        if !t.parts.is_empty() {
                            diff.op = StructOp::Modify;
                            diff.parts = t.parts;
                            diff.stmts = t.stmts;
                        }
                    }
                    _ => {
                        let fix = |t: String| if d == Dialect::Mssql { mssql_schema_rewrite(&t, &src.schema, &dst.schema) } else { t };
                        if norm(&fix(ddl_text(&s_pre))) != norm(&ddl_text(&t_pre)) || norm(&fix(ddl_text(&s_post))) != norm(&ddl_text(&t_post)) {
                            diff.op = StructOp::Modify;
                            let create = create_text(d, &[s_pre.clone(), s_post.clone()].concat(), &src.schema, &dst.schema);
                            let drop_first = match (d, *kind) {
                                (Dialect::Mysql, "view") => Vec::new(),
                                (Dialect::Mysql, _) | (Dialect::Sqlite, _) => vec![if *kind == "view" { format!("DROP VIEW {target}") } else { function_drop(d, dst, name, false) }],
                                _ => Vec::new(),
                            };
                            let mut sql = drop_first;
                            for c in create {
                                sql.push(match (d, *kind) {
                                    (Dialect::Mysql, "view") => c.replacen("CREATE VIEW", "CREATE OR REPLACE VIEW", 1),
                                    (Dialect::Mssql, _) => c.replacen("CREATE ", "CREATE OR ALTER ", 1),
                                    _ => c,
                                });
                            }
                            for (i, s) in sql.into_iter().enumerate() {
                                diff.stmts.push(Stmt { phase: 7, order: i as i64, part: None, sql: s });
                            }
                        }
                    }
                },
                (false, false) => continue,
            }
            out.push(diff);
        }
    }
    progress("", 1.0);
    Ok(out)
}

/// `CREATE TABLE nome (...)` com outro nome (o nome pode vir entre aspas ou não).
fn rename_create(create: &str, new_name: &str) -> String {
    let Some(at) = create.to_uppercase().find("CREATE TABLE") else { return create.to_string() };
    let head_end = at + "CREATE TABLE".len();
    let rest = &create[head_end..];
    let trimmed = rest.trim_start();
    let after_if = trimmed.to_uppercase().strip_prefix("IF NOT EXISTS").map_or(0, |_| "IF NOT EXISTS".len());
    let body = trimmed[after_if..].trim_start();
    let name_len = match body.chars().next() {
        Some(q @ ('"' | '`' | '\'')) => body[1..].find(q).map_or(body.len(), |i| i + 2),
        Some('[') => body.find(']').map_or(body.len(), |i| i + 1),
        _ => body.find(|c: char| c.is_whitespace() || c == '(').unwrap_or(body.len()),
    };
    format!("{} {}{}", &create[..head_end], Dialect::Sqlite.q(new_name), &body[name_len..])
}

/// SQLite: recria a tabela com a estrutura da origem, copiando as colunas em comum.
fn sqlite_rebuild(src: &TableDetail, dst: &TableDetail, name: &str, s_pre: &[String], s_post: &[String]) -> Vec<String> {
    let d = Dialect::Sqlite;
    let temp = format!("{name}__atlas_new");
    let create = s_pre.iter().find(|s| s.trim_start().to_uppercase().starts_with("CREATE TABLE")).cloned().unwrap_or_default();
    let create = rename_create(create.trim_end_matches(';'), &temp);
    let common: Vec<String> = src.columns.iter().filter(|c| dst.columns.iter().any(|x| x.name == c.name)).map(|c| d.q(&c.name)).collect();
    let cols = common.join(", ");
    let mut v = vec!["PRAGMA foreign_keys = OFF".to_string(), create];
    if !common.is_empty() {
        v.push(format!("INSERT INTO {} ({cols}) SELECT {cols} FROM {}", d.q(&temp), d.q(name)));
    }
    v.push(format!("DROP TABLE {}", d.q(name)));
    v.push(format!("ALTER TABLE {} RENAME TO {}", d.q(&temp), d.q(name)));
    v.extend(s_post.iter().map(|s| s.trim_end_matches(';').to_string()));
    v.push("PRAGMA foreign_keys = ON".to_string());
    v
}

#[tauri::command]
pub async fn ext_structsync_compare(
    app: AppHandle,
    mysql: State<'_, MysqlSessions>,
    mssql: State<'_, MssqlSessions>,
    sqlite: State<'_, SqliteSessions>,
    comparisons: State<'_, ExtStructComparisons>,
    req: StructCompareRequest,
) -> Result<Vec<ObjectOut>, DatabaseError> {
    comparisons.inner.lock().unwrap().remove(&req.compare_id);
    let (d, s_exec) = exec_for(&req.source_conn, &req.source_database, &req.spec.source_schema, &mysql, &mssql, &sqlite)?;
    let (d2, t_exec) = exec_for(&req.target_conn, &req.target_database, &req.spec.target_schema, &mysql, &mssql, &sqlite)?;
    if d != d2 {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "the source and the target must be connections of the same database product"));
    }
    if req.source_conn == req.target_conn && req.source_database == req.target_database && req.spec.source_schema == req.spec.target_schema {
        return Err(DatabaseError::new(ErrorKind::InvalidInput, "the source and the target are the same"));
    }
    let src = Side { pair: backend_for(&req.source_conn, &mysql, &mssql, &sqlite)?, exec: s_exec, database: req.source_database.clone(), schema: req.spec.source_schema.clone() };
    let dst = Side { pair: backend_for(&req.target_conn, &mysql, &mssql, &sqlite)?, exec: t_exec, database: req.target_database.clone(), schema: req.spec.target_schema.clone() };
    let cancel = Arc::new(AtomicBool::new(false));
    register_cancel(&req.compare_id, &req.source_conn, &cancel, &mysql, &mssql, &sqlite);
    let result = compare(&app, d, &src, &dst, &req.compare_id, &cancel).await;
    unregister_cancel(&req.compare_id, &req.source_conn, &mysql, &mssql, &sqlite);
    let diffs = result?;
    let out = diffs
        .iter()
        .map(|o| ObjectOut { kind: o.kind, name: o.name.clone(), op: o.op, parts: o.parts.clone(), source_ddl: o.source_ddl.clone(), target_ddl: o.target_ddl.clone() })
        .collect();
    comparisons.inner.lock().unwrap().insert(req.compare_id, Arc::new(diffs));
    Ok(out)
}

/// Os comandos do que está marcado, na ordem em que devem rodar.
#[tauri::command]
pub fn ext_structsync_script(comparisons: State<'_, ExtStructComparisons>, compare_id: String, selection: Vec<StructSelection>) -> Result<Vec<String>, DatabaseError> {
    let c = comparisons.inner.lock().unwrap().get(&compare_id).cloned().ok_or_else(|| DatabaseError::new(ErrorKind::InvalidInput, "the comparison is no longer available; compare again"))?;
    let mut chosen: Vec<(u8, usize, i64, usize, &str)> = Vec::new();
    for (oi, o) in c.iter().enumerate() {
        let Some(sel) = selection.iter().find(|s| s.kind == o.kind && s.name == o.name) else { continue };
        for (si, s) in o.stmts.iter().enumerate() {
            let wanted = match (&sel.parts, s.part) {
                (None, _) | (Some(_), None) => true,
                (Some(p), Some(i)) => p.contains(&i),
            };
            if wanted {
                chosen.push((s.phase, oi, s.order, si, s.sql.as_str()));
            }
        }
    }
    chosen.sort_by_key(|(phase, oi, order, si, _)| (*phase, *oi, *order, *si));
    Ok(chosen.into_iter().map(|(.., sql)| format!("{};", sql.trim_end_matches(';'))).collect())
}

#[tauri::command]
pub async fn ext_structsync_deploy(
    app: AppHandle,
    mysql: State<'_, MysqlSessions>,
    mssql: State<'_, MssqlSessions>,
    sqlite: State<'_, SqliteSessions>,
    req: StructDeployRequest,
) -> Result<DeployCounts, DatabaseError> {
    let (d, dst) = exec_for(&req.target_conn, &req.target_database, &req.target_database, &mysql, &mssql, &sqlite)?;
    let cancel = Arc::new(AtomicBool::new(false));
    register_cancel(&req.deploy_id, &req.target_conn, &cancel, &mysql, &mssql, &sqlite);
    let (log_app, log_id) = (app.clone(), req.deploy_id.clone());
    let mut on_log = move |line: String| {
        let _ = log_app.emit("datasync-log", LogEvent { id: &log_id, line });
    };
    let (progress_app, progress_id) = (app.clone(), req.deploy_id.clone());
    let mut on_progress = move |done: u64, total: u64| {
        let _ = progress_app.emit("datasync-deploy-progress", DeployProgress { id: &progress_id, done, total });
    };
    let cancelled = || cancel.load(Ordering::Relaxed);
    // O MySQL liga a checagem de chaves estrangeiras só depois: as tabelas novas trazem as chaves dentro do CREATE.
    let mut statements = req.statements.clone();
    if d == Dialect::Mysql {
        statements.insert(0, "SET FOREIGN_KEY_CHECKS = 0;".into());
        statements.push("SET FOREIGN_KEY_CHECKS = 1;".into());
    }
    let out = deploy(d, &dst, &statements, false, req.continue_on_error, req.include_query, &mut on_log, &mut on_progress, &cancelled).await;
    unregister_cancel(&req.deploy_id, &req.target_conn, &mysql, &mssql, &sqlite);
    out
}

#[tauri::command]
pub fn ext_structsync_forget(comparisons: State<'_, ExtStructComparisons>, compare_id: String) {
    comparisons.inner.lock().unwrap().remove(&compare_id);
}
