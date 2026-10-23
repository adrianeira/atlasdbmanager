//! Dump SQL (estrutura e dados) gerado a partir do catálogo, sem `pg_dump`.
//!
//! Tudo roda numa transação `REPEATABLE READ` somente leitura (um snapshot só, como o `pg_dump`) e com
//! `search_path = pg_catalog`, de modo que as funções `pg_get_*` devolvam nomes sempre qualificados. O resultado é um
//! plano em partes (antes dos dados, tabelas com dados, depois dos dados); quem grava o arquivo costura as partes.
//! Não entram: privilégios (`GRANT`), opções de armazenamento (`reloptions`), tabelas estrangeiras, agregações e
//! objetos que pertencem a extensões (a extensão é recriada com `CREATE EXTENSION`).

use crate::connect::PgSession;
use crate::error::map_error;
use crate::exec::quote_ident;
use crate::meta::{sequence_detail, type_detail};
use atlas_core::{DatabaseError, ErrorKind};
use futures_util::StreamExt;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

/// O que despejar.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DumpScope {
    /// Todos os schemas do usuário.
    Database,
    Schema { schema: String },
    /// Só estas tabelas (com seus índices, restrições, gatilhos, comentários e as sequências que são delas).
    Tables { schema: String, tables: Vec<String> },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpOptions {
    pub scope: DumpScope,
    pub structure: bool,
    pub data: bool,
    /// `DROP ... IF EXISTS ... CASCADE` antes de cada `CREATE`.
    pub clean: bool,
    /// `ALTER ... OWNER TO`.
    pub include_owner: bool,
}

/// Tabela cujos dados entram no dump.
#[derive(Debug, Clone)]
pub struct DumpTable {
    /// `"schema"."tabela"`.
    pub quoted: String,
    pub name: String,
    /// Colunas copiadas (sem as geradas, que o servidor recalcula).
    pub columns: Vec<String>,
}

#[derive(Debug, Default)]
pub struct DumpPlan {
    /// Extensões, schemas, tipos, sequências, funções e tabelas.
    pub pre_data: Vec<String>,
    pub data_tables: Vec<DumpTable>,
    /// Restrições, índices, views, chaves estrangeiras, gatilhos, valores das sequências e comentários.
    pub post_data: Vec<String>,
    /// Objetos despejados (tabelas, views, sequências, funções, tipos).
    pub objects: usize,
}

fn sql_err(e: tokio_postgres::Error) -> DatabaseError {
    map_error(&e)
}

fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Abre o snapshot: sessão só de leitura, nomes sempre qualificados.
pub async fn begin_snapshot(s: &PgSession) -> Result<(), DatabaseError> {
    s.client
        .batch_execute("SET search_path TO pg_catalog; BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .map_err(sql_err)
}

pub async fn end_snapshot(s: &PgSession) {
    let _ = s.client.batch_execute("ROLLBACK").await;
}

/// Cabeçalho do arquivo: ajustes de sessão que o restore precisa (como o do `pg_dump`).
pub fn header(server_version: &str, database: &str) -> String {
    format!(
        "-- Atlas DB Manager: SQL dump\n-- Servidor: PostgreSQL {server_version}; banco: {database}\n\n\
         SET statement_timeout = 0;\nSET lock_timeout = 0;\nSET client_encoding = 'UTF8';\nSET standard_conforming_strings = on;\n\
         SET check_function_bodies = false;\nSET client_min_messages = warning;\nSELECT pg_catalog.set_config('search_path', '', false);\n"
    )
}

/// Está ligado a uma extensão (`pg_depend` com `deptype = 'e'`)? `class` é o catálogo e `oid` a expressão do objeto.
fn in_extension(class: &str, oid: &str) -> String {
    format!(
        "NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend ext WHERE ext.classid = 'pg_catalog.{class}'::regclass AND ext.objid = {oid} AND ext.deptype = 'e')"
    )
}

struct Column {
    name: String,
    data_type: String,
    not_null: bool,
    default: Option<String>,
    identity: String,
    generated: String,
    collation: Option<String>,
}

struct TableInfo {
    oid: u32,
    name: String,
    quoted: String,
    relkind: String,
    unlogged: bool,
    partition_key: Option<String>,
    partition_bound: Option<String>,
    is_partition: bool,
    parents: Vec<String>,
    owner: String,
    comment: Option<String>,
    columns: Vec<Column>,
}

pub async fn plan(s: &PgSession, o: &DumpOptions) -> Result<DumpPlan, DatabaseError> {
    let v = s.server.version_num;
    if v < 90300 {
        return Err(DatabaseError::new(ErrorKind::UnsupportedFeature, "SQL dump requires PostgreSQL 9.3 or later"));
    }
    let mut plan = DumpPlan::default();

    // Schemas do escopo.
    let (schemas, only_tables): (Vec<String>, Option<HashSet<String>>) = match &o.scope {
        DumpScope::Database => {
            let rows = s
                .client
                .query("SELECT nspname::text FROM pg_catalog.pg_namespace WHERE nspname !~ '^pg_' AND nspname <> 'information_schema' ORDER BY nspname", &[])
                .await
                .map_err(sql_err)?;
            (rows.iter().map(|r| r.get(0)).collect(), None)
        }
        DumpScope::Schema { schema } => (vec![schema.clone()], None),
        DumpScope::Tables { schema, tables } => (vec![schema.clone()], Some(tables.iter().cloned().collect())),
    };
    let table_scope = only_tables.is_some();

    // ---- Tabelas e colunas (precisa vir antes: o resto depende delas).
    let part_key = if v >= 100000 { "CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END" } else { "NULL::text" };
    let part_bound = if v >= 100000 { "CASE WHEN c.relispartition THEN pg_get_expr(c.relpartbound, c.oid) END" } else { "NULL::text" };
    let is_part = if v >= 100000 { "c.relispartition" } else { "false" };
    let kinds = if v >= 100000 { "'r','p'" } else { "'r'" };
    let sql = format!(
        "SELECT c.oid, n.nspname::text, c.relname::text, quote_ident(n.nspname) || '.' || quote_ident(c.relname), c.relkind::text, \
                c.relpersistence = 'u', {part_key}, {part_bound}, {is_part}, \
                ARRAY(SELECT quote_ident(pn.nspname) || '.' || quote_ident(pc.relname) FROM pg_catalog.pg_inherits i \
                        JOIN pg_catalog.pg_class pc ON pc.oid = i.inhparent JOIN pg_catalog.pg_namespace pn ON pn.oid = pc.relnamespace \
                       WHERE i.inhrelid = c.oid ORDER BY i.inhseqno), \
                pg_get_userbyid(c.relowner)::text, obj_description(c.oid, 'pg_class') \
           FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
          WHERE n.nspname = ANY($1) AND c.relkind IN ({kinds}) AND {} ORDER BY c.oid",
        in_extension("pg_class", "c.oid")
    );
    let mut tables: Vec<TableInfo> = Vec::new();
    for r in s.client.query(&sql, &[&schemas]).await.map_err(sql_err)? {
        let name: String = r.get(2);
        if only_tables.as_ref().is_some_and(|set| !set.contains(&name)) {
            continue;
        }
        tables.push(TableInfo {
            oid: r.get(0),
            name,
            quoted: r.get(3),
            relkind: r.get(4),
            unlogged: r.get(5),
            partition_key: r.get(6),
            partition_bound: r.get(7),
            is_partition: r.get(8),
            parents: r.get(9),
            owner: r.get(10),
            comment: r.get(11),
            columns: Vec::new(),
        });
    }
    let table_oids: Vec<u32> = tables.iter().map(|t| t.oid).collect();
    let identity = if v >= 100000 { "a.attidentity::text" } else { "''::text" };
    let generated = if v >= 120000 { "a.attgenerated::text" } else { "''::text" };
    let colsql = format!(
        "SELECT a.attrelid, a.attname::text, format_type(a.atttypid, a.atttypmod), a.attnotnull, pg_get_expr(d.adbin, d.adrelid), {identity}, {generated}, \
                (SELECT quote_ident(cn.nspname) || '.' || quote_ident(co.collname) FROM pg_catalog.pg_collation co \
                   JOIN pg_catalog.pg_namespace cn ON cn.oid = co.collnamespace \
                  WHERE co.oid = a.attcollation AND a.attcollation <> t.typcollation) \
           FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_type t ON t.oid = a.atttypid \
           LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
          WHERE a.attrelid = ANY($1) AND a.attnum > 0 AND NOT a.attisdropped AND a.attislocal ORDER BY a.attrelid, a.attnum"
    );
    let mut by_oid: HashMap<u32, usize> = HashMap::new();
    for (i, t) in tables.iter().enumerate() {
        by_oid.insert(t.oid, i);
    }
    for r in s.client.query(&colsql, &[&table_oids]).await.map_err(sql_err)? {
        let oid: u32 = r.get(0);
        if let Some(&i) = by_oid.get(&oid) {
            tables[i].columns.push(Column {
                name: r.get(1),
                data_type: r.get(2),
                not_null: r.get(3),
                default: r.get(4),
                identity: r.get(5),
                generated: r.get(6),
                collation: r.get(7),
            });
        }
    }

    let mut pre = Vec::new();
    let mut post = Vec::new();
    let owner_of = |kind: &str, name: &str, owner: &str| format!("ALTER {kind} {name} OWNER TO {};", quote_ident(owner));

    // ---- Extensões, schemas.
    if matches!(o.scope, DumpScope::Database) && o.structure {
        let rows = s
            .client
            .query(
                "SELECT quote_ident(e.extname), quote_ident(n.nspname) FROM pg_catalog.pg_extension e \
                   JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace WHERE e.extname <> 'plpgsql' ORDER BY e.oid",
                &[],
            )
            .await
            .map_err(sql_err)?;
        for r in rows {
            pre.push(format!("CREATE EXTENSION IF NOT EXISTS {} WITH SCHEMA {};", r.get::<_, String>(0), r.get::<_, String>(1)));
        }
    }
    if !table_scope && o.structure {
        let rows = s
            .client
            .query(
                "SELECT quote_ident(n.nspname), n.nspname::text, pg_get_userbyid(n.nspowner)::text, obj_description(n.oid, 'pg_namespace') \
                   FROM pg_catalog.pg_namespace n WHERE n.nspname = ANY($1) AND n.nspname <> 'public' ORDER BY n.oid",
                &[&schemas],
            )
            .await
            .map_err(sql_err)?;
        for r in rows {
            let (q, owner, comment): (String, String, Option<String>) = (r.get(0), r.get(2), r.get(3));
            pre.push(format!("CREATE SCHEMA IF NOT EXISTS {q};"));
            if o.include_owner {
                pre.push(owner_of("SCHEMA", &q, &owner));
            }
            if let Some(c) = comment {
                post.push(format!("COMMENT ON SCHEMA {q} IS {};", literal(&c)));
            }
        }
    }

    // ---- Tipos (fora do escopo de tabelas).
    let mut drops: Vec<String> = Vec::new();
    if o.structure && !table_scope {
        let rows = s
            .client
            .query(
                &format!(
                    "SELECT t.oid, n.nspname::text, t.typname::text, quote_ident(n.nspname) || '.' || quote_ident(t.typname), t.typtype::text, \
                            CASE WHEN t.typtype = 'd' THEN format_type(t.typbasetype, t.typtypmod) END, t.typdefault, t.typnotnull, \
                            pg_get_userbyid(t.typowner)::text, obj_description(t.oid, 'pg_type') \
                       FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
                      WHERE n.nspname = ANY($1) AND (t.typtype IN ('e','d','r') \
                            OR (t.typtype = 'c' AND EXISTS (SELECT 1 FROM pg_catalog.pg_class tc WHERE tc.oid = t.typrelid AND tc.relkind = 'c'))) \
                        AND {} ORDER BY t.oid",
                    in_extension("pg_type", "t.oid")
                ),
                &[&schemas],
            )
            .await
            .map_err(sql_err)?;
        for r in rows {
            let (schema, name, q, kind): (String, String, String, String) = (r.get(1), r.get(2), r.get(3), r.get(4));
            let base: Option<String> = r.get(5);
            let (default, not_null): (Option<String>, bool) = (r.get(6), r.get(7));
            let (owner, comment): (String, Option<String>) = (r.get(8), r.get(9));
            let kw = if kind == "d" { "DOMAIN" } else { "TYPE" };
            drops.push(format!("DROP {kw} IF EXISTS {q} CASCADE;"));
            let detail = type_detail(s, &schema, &name).await?;
            let stmt = match kind.as_str() {
                "e" => format!("CREATE TYPE {q} AS ENUM ({});", detail.labels.iter().map(|l| literal(l)).collect::<Vec<_>>().join(", ")),
                "c" => format!(
                    "CREATE TYPE {q} AS (\n    {}\n);",
                    detail.attributes.iter().map(|a| format!("{} {}", quote_ident(&a.name), a.data_type)).collect::<Vec<_>>().join(",\n    ")
                ),
                "d" => {
                    let mut parts = vec![format!("CREATE DOMAIN {q} AS {}", base.unwrap_or_default())];
                    if let Some(d) = default {
                        parts.push(format!("DEFAULT {d}"));
                    }
                    if not_null {
                        parts.push("NOT NULL".into());
                    }
                    for c in &detail.checks {
                        parts.push(format!("CONSTRAINT {} {}", quote_ident(&c.name), c.definition));
                    }
                    format!("{};", parts.join("\n    "))
                }
                _ => format!("CREATE TYPE {q} AS RANGE (subtype = {});", detail.subtype.unwrap_or_default()),
            };
            pre.push(stmt);
            if o.include_owner {
                pre.push(owner_of(kw, &q, &owner));
            }
            if let Some(c) = comment {
                post.push(format!("COMMENT ON {kw} {q} IS {};", literal(&c)));
            }
            plan.objects += 1;
        }
    }

    // ---- Sequências (as que são de identidade vêm da própria coluna).
    struct Seq {
        quoted: String,
        last_value: String,
        is_called: bool,
        owned_by: Option<String>,
    }
    let mut seqs: Vec<Seq> = Vec::new();
    {
        let rows = s
            .client
            .query(
                &format!(
                    "SELECT n.nspname::text, c.relname::text, quote_ident(n.nspname) || '.' || quote_ident(c.relname), pg_get_userbyid(c.relowner)::text, \
                            obj_description(c.oid, 'pg_class'), \
                            (SELECT quote_ident(tn.nspname) || '.' || quote_ident(t.relname) || '.' || quote_ident(a.attname) FROM pg_catalog.pg_depend d \
                               JOIN pg_catalog.pg_class t ON t.oid = d.refobjid JOIN pg_catalog.pg_namespace tn ON tn.oid = t.relnamespace \
                               JOIN pg_catalog.pg_attribute a ON a.attrelid = d.refobjid AND a.attnum = d.refobjsubid \
                              WHERE d.classid = 'pg_catalog.pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'a' AND d.refobjsubid > 0 LIMIT 1), \
                            (SELECT quote_ident(tn.nspname) || '.' || quote_ident(t.relname) FROM pg_catalog.pg_depend d \
                               JOIN pg_catalog.pg_class t ON t.oid = d.refobjid JOIN pg_catalog.pg_namespace tn ON tn.oid = t.relnamespace \
                              WHERE d.classid = 'pg_catalog.pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'a' AND d.refobjsubid > 0 LIMIT 1) \
                       FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                      WHERE c.relkind = 'S' AND n.nspname = ANY($1) AND {} \
                        AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend d WHERE d.classid = 'pg_catalog.pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'i') \
                      ORDER BY c.oid",
                    in_extension("pg_class", "c.oid")
                ),
                &[&schemas],
            )
            .await
            .map_err(sql_err)?;
        let included_quoted: HashSet<&str> = tables.iter().map(|t| t.quoted.as_str()).collect();
        for r in rows {
            let (schema, name, q, owner): (String, String, String, String) = (r.get(0), r.get(1), r.get(2), r.get(3));
            let (comment, owned_by, owner_table): (Option<String>, Option<String>, Option<String>) = (r.get(4), r.get(5), r.get(6));
            // Escopo de tabelas: só as sequências que pertencem às tabelas escolhidas.
            let owner_dumped = owner_table.as_deref().is_some_and(|t| included_quoted.contains(t));
            if table_scope && !owner_dumped {
                continue;
            }
            let d = sequence_detail(s, &schema, &name).await?;
            drops.push(format!("DROP SEQUENCE IF EXISTS {q} CASCADE;"));
            if o.structure {
                let as_clause = if v >= 100000 && d.data_type != "bigint" { format!("    AS {}\n", d.data_type) } else { String::new() };
                pre.push(format!(
                    "CREATE SEQUENCE {q}\n{as_clause}    START WITH {}\n    INCREMENT BY {}\n    MINVALUE {}\n    MAXVALUE {}\n    CACHE {}{};",
                    d.start, d.increment, d.min, d.max, d.cache, if d.cycle { "\n    CYCLE" } else { "" }
                ));
                if o.include_owner {
                    pre.push(owner_of("SEQUENCE", &q, &owner));
                }
                if let Some(c) = comment {
                    post.push(format!("COMMENT ON SEQUENCE {q} IS {};", literal(&c)));
                }
                plan.objects += 1;
            }
            // `OWNED BY` só se a tabela dona está no dump (senão o restore falharia).
            seqs.push(Seq { quoted: q, last_value: d.last_value, is_called: d.is_called, owned_by: owned_by.filter(|_| owner_dumped) });
        }
    }

    // ---- Funções e procedimentos (fora do escopo de tabelas).
    if o.structure && !table_scope {
        let fn_filter = if v >= 110000 { "p.prokind IN ('f','p')" } else { "NOT p.proisagg AND NOT p.proiswindow" };
        let kind = if v >= 110000 { "CASE p.prokind WHEN 'p' THEN 'PROCEDURE' ELSE 'FUNCTION' END" } else { "'FUNCTION'::text" };
        let rows = s
            .client
            .query(
                &format!(
                    "SELECT quote_ident(n.nspname) || '.' || quote_ident(p.proname) || '(' || pg_get_function_identity_arguments(p.oid) || ')', \
                            pg_get_functiondef(p.oid), {kind}, pg_get_userbyid(p.proowner)::text, obj_description(p.oid, 'pg_proc') \
                       FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
                      WHERE n.nspname = ANY($1) AND {fn_filter} \
                        AND p.prolang NOT IN (SELECT oid FROM pg_catalog.pg_language WHERE lanname IN ('internal', 'c')) AND {} ORDER BY p.oid",
                    in_extension("pg_proc", "p.oid")
                ),
                &[&schemas],
            )
            .await
            .map_err(sql_err)?;
        for r in rows {
            let (sig, def, kw, owner, comment): (String, String, String, String, Option<String>) = (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4));
            drops.push(format!("DROP {kw} IF EXISTS {sig};"));
            pre.push(format!("{};", def.trim_end().trim_end_matches(';')));
            if o.include_owner {
                pre.push(owner_of(&kw, &sig, &owner));
            }
            if let Some(c) = comment {
                post.push(format!("COMMENT ON {kw} {sig} IS {};", literal(&c)));
            }
            plan.objects += 1;
        }
    }

    // ---- Tabelas: CREATE TABLE e a lista dos dados.
    for t in &tables {
        drops.push(format!("DROP TABLE IF EXISTS {} CASCADE;", t.quoted));
        if o.structure {
            let persistence = if t.unlogged { "UNLOGGED " } else { "" };
            let tail = t.partition_key.as_ref().map(|k| format!(" PARTITION BY {k}")).unwrap_or_default();
            if t.is_partition {
                let parent = t.parents.first().cloned().unwrap_or_default();
                pre.push(format!("CREATE {persistence}TABLE {} PARTITION OF {parent} {}{tail};", t.quoted, t.partition_bound.clone().unwrap_or_default()));
            } else {
                let lines: Vec<String> = t
                    .columns
                    .iter()
                    .map(|c| {
                        let mut l = format!("    {} {}", quote_ident(&c.name), c.data_type);
                        if let Some(co) = &c.collation {
                            l.push_str(&format!(" COLLATE {co}"));
                        }
                        match (c.identity.as_str(), c.generated.as_str(), &c.default) {
                            ("a", _, _) => l.push_str(" GENERATED ALWAYS AS IDENTITY"),
                            ("d", _, _) => l.push_str(" GENERATED BY DEFAULT AS IDENTITY"),
                            (_, "s", Some(d)) => l.push_str(&format!(" GENERATED ALWAYS AS ({d}) STORED")),
                            (_, _, Some(d)) => l.push_str(&format!(" DEFAULT {d}")),
                            _ => {}
                        }
                        if c.not_null {
                            l.push_str(" NOT NULL");
                        }
                        l
                    })
                    .collect();
                let inherits = if t.parents.is_empty() { String::new() } else { format!("\nINHERITS ({})", t.parents.join(", ")) };
                let body = if lines.is_empty() { "()".to_string() } else { format!("(\n{}\n)", lines.join(",\n")) };
                pre.push(format!("CREATE {persistence}TABLE {} {body}{inherits}{tail};", t.quoted));
            }
            if o.include_owner {
                pre.push(owner_of("TABLE", &t.quoted, &t.owner));
            }
            if let Some(c) = &t.comment {
                post.push(format!("COMMENT ON TABLE {} IS {};", t.quoted, literal(c)));
            }
            plan.objects += 1;
        }
        // Os dados vão por tabela "folha": a tabela particionada em si não guarda linhas.
        if o.data && t.relkind == "r" {
            let columns: Vec<String> = t.columns.iter().filter(|c| c.generated != "s").map(|c| c.name.clone()).collect();
            // Colunas que o servidor herda não aparecem em `attislocal`: lê a lista completa do que é copiável.
            let all = copy_columns(s, t.oid).await?;
            plan.data_tables.push(DumpTable { quoted: t.quoted.clone(), name: t.name.clone(), columns: if all.is_empty() { columns } else { all } });
        }
    }

    // ---- Views e views materializadas, em ordem de dependência.
    let mut view_rows = Vec::new();
    if o.structure && !table_scope {
        let rows = s
            .client
            .query(
                &format!(
                    "SELECT c.oid, quote_ident(n.nspname) || '.' || quote_ident(c.relname), c.relkind::text, pg_get_viewdef(c.oid, true), \
                            pg_get_userbyid(c.relowner)::text, obj_description(c.oid, 'pg_class') \
                       FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                      WHERE c.relkind IN ('v','m') AND n.nspname = ANY($1) AND {} ORDER BY c.oid",
                    in_extension("pg_class", "c.oid")
                ),
                &[&schemas],
            )
            .await
            .map_err(sql_err)?;
        let edges = s
            .client
            .query(
                "SELECT DISTINCT dep.oid, src.oid FROM pg_catalog.pg_depend d \
                   JOIN pg_catalog.pg_rewrite r ON r.oid = d.objid \
                   JOIN pg_catalog.pg_class dep ON dep.oid = r.ev_class \
                   JOIN pg_catalog.pg_class src ON src.oid = d.refobjid \
                  WHERE d.classid = 'pg_catalog.pg_rewrite'::regclass AND dep.oid <> src.oid AND src.relkind IN ('v','m') AND dep.relkind IN ('v','m')",
                &[],
            )
            .await
            .map_err(sql_err)?;
        let mut needs: HashMap<u32, Vec<u32>> = HashMap::new();
        for e in edges {
            needs.entry(e.get(0)).or_default().push(e.get(1));
        }
        let all: Vec<(u32, String, String, String, String, Option<String>)> =
            rows.iter().map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5))).collect();
        let mut placed: HashSet<u32> = HashSet::new();
        let known: HashSet<u32> = all.iter().map(|v| v.0).collect();
        // Cada rodada coloca as views cujas dependências já foram colocadas (ou não estão no escopo).
        while view_rows.len() < all.len() {
            let before = view_rows.len();
            for v in &all {
                if placed.contains(&v.0) {
                    continue;
                }
                if needs.get(&v.0).is_none_or(|n| n.iter().all(|d| placed.contains(d) || !known.contains(d))) {
                    placed.insert(v.0);
                    view_rows.push(v.clone());
                }
            }
            if view_rows.len() == before {
                // Dependência circular não deveria existir: despeja o que sobrou na ordem de criação.
                for v in &all {
                    if placed.insert(v.0) {
                        view_rows.push(v.clone());
                    }
                }
            }
        }
    }

    // ---- Depois dos dados: restrições, índices, views, chaves estrangeiras, gatilhos.
    let included: HashSet<u32> = table_oids.iter().copied().collect();
    let kinds_by_oid: HashMap<u32, &str> = tables.iter().map(|t| (t.oid, t.relkind.as_str())).collect();
    if o.structure && !tables.is_empty() {
        let rows = s
            .client
            .query(
                "SELECT con.conrelid, quote_ident(n.nspname) || '.' || quote_ident(c.relname), quote_ident(con.conname), con.contype::text, \
                        pg_get_constraintdef(con.oid), con.confrelid \
                   FROM pg_catalog.pg_constraint con JOIN pg_catalog.pg_class c ON c.oid = con.conrelid \
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  WHERE con.conrelid = ANY($1) AND con.contype IN ('p','u','c','x','f') AND con.conislocal \
                  ORDER BY (con.contype = 'f'), n.nspname, c.relname, con.conname",
                &[&table_oids],
            )
            .await
            .map_err(sql_err)?;
        let (mut plain, mut fks) = (Vec::new(), Vec::new());
        for r in rows {
            let (rel, table, name, kind, def, refrel): (u32, String, String, String, String, u32) = (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5));
            let only = if kinds_by_oid.get(&rel) == Some(&"p") { "" } else { "ONLY " };
            let stmt = format!("ALTER TABLE {only}{table} ADD CONSTRAINT {name} {def};");
            if kind == "f" {
                if included.contains(&refrel) || !table_scope {
                    fks.push(stmt);
                } else {
                    fks.push(format!("-- chave estrangeira {name} de {table} não incluída: a tabela referenciada está fora do dump"));
                }
            } else {
                plain.push(stmt);
            }
        }
        post.splice(0..0, plain);

        let ispart = if v >= 110000 { "AND NOT ic.relispartition" } else { "" };
        let idx = s
            .client
            .query(
                &format!(
                    "SELECT pg_get_indexdef(i.indexrelid) FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid \
                      WHERE i.indrelid = ANY($1) AND NOT i.indisprimary {ispart} \
                        AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_constraint c WHERE c.conindid = i.indexrelid AND c.contype IN ('p','u','x')) \
                      ORDER BY ic.relname"
                ),
                &[&table_oids],
            )
            .await
            .map_err(sql_err)?;
        let index_stmts: Vec<String> = idx.iter().map(|r| format!("{};", r.get::<_, String>(0))).collect();
        post.extend(index_stmts);

        for (_, q, kind, def, owner, comment) in &view_rows {
            let (kw, stmt) = if kind == "m" {
                ("MATERIALIZED VIEW", format!("CREATE MATERIALIZED VIEW {q} AS\n{}\nWITH NO DATA;", def.trim_end().trim_end_matches(';')))
            } else {
                ("VIEW", format!("CREATE VIEW {q} AS\n{}", def.trim_end()))
            };
            drops.push(format!("DROP {kw} IF EXISTS {q} CASCADE;"));
            post.push(stmt);
            if o.include_owner {
                post.push(owner_of(kw, q, owner));
            }
            if let Some(c) = comment {
                post.push(format!("COMMENT ON {kw} {q} IS {};", literal(c)));
            }
            plan.objects += 1;
        }
        // Índices das views materializadas (a consulta acima só cobriu tabelas).
        let mat_oids: Vec<u32> = view_rows.iter().filter(|v| v.2 == "m").map(|v| v.0).collect();
        if !mat_oids.is_empty() {
            let rows = s
                .client
                .query(
                    "SELECT pg_get_indexdef(i.indexrelid) FROM pg_catalog.pg_index i JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid \
                      WHERE i.indrelid = ANY($1) ORDER BY ic.relname",
                    &[&mat_oids],
                )
                .await
                .map_err(sql_err)?;
            post.extend(rows.iter().map(|r| format!("{};", r.get::<_, String>(0))));
            if o.data {
                for (_, q, kind, ..) in &view_rows {
                    if kind == "m" {
                        post.push(format!("REFRESH MATERIALIZED VIEW {q};"));
                    }
                }
            }
        }
        post.extend(fks);

        let trg_filter = if v >= 130000 { "AND t.tgparentid = 0" } else { "" };
        let trows = s
            .client
            .query(
                &format!(
                    "SELECT pg_get_triggerdef(t.oid, true), t.tgenabled::text, quote_ident(n.nspname) || '.' || quote_ident(c.relname), quote_ident(t.tgname) \
                       FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                      WHERE t.tgrelid = ANY($1) AND NOT t.tgisinternal {trg_filter} ORDER BY n.nspname, c.relname, t.tgname"
                ),
                &[&table_oids],
            )
            .await
            .map_err(sql_err)?;
        for r in trows {
            let (def, enabled, table, name): (String, String, String, String) = (r.get(0), r.get(1), r.get(2), r.get(3));
            post.push(format!("{def};"));
            if enabled == "D" {
                post.push(format!("ALTER TABLE {table} DISABLE TRIGGER {name};"));
            }
        }
    } else if o.structure {
        for (_, q, kind, def, _, _) in &view_rows {
            let (kw, stmt) = if kind == "m" {
                ("MATERIALIZED VIEW", format!("CREATE MATERIALIZED VIEW {q} AS\n{}\nWITH NO DATA;", def.trim_end().trim_end_matches(';')))
            } else {
                ("VIEW", format!("CREATE VIEW {q} AS\n{}", def.trim_end()))
            };
            drops.push(format!("DROP {kw} IF EXISTS {q} CASCADE;"));
            post.push(stmt);
            plan.objects += 1;
        }
    }

    // ---- Dono das sequências, valores atuais e comentários de coluna.
    if o.structure {
        for q in &seqs {
            if let Some(by) = &q.owned_by {
                post.push(format!("ALTER SEQUENCE {} OWNED BY {by};", q.quoted));
            }
        }
        let mut with_comments: Vec<u32> = table_oids.clone();
        with_comments.extend(view_rows.iter().map(|v| v.0));
        let crows = s
            .client
            .query(
                "SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname), quote_ident(a.attname), col_description(c.oid, a.attnum) \
                   FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid = a.attrelid JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  WHERE a.attrelid = ANY($1) AND a.attnum > 0 AND NOT a.attisdropped AND col_description(c.oid, a.attnum) IS NOT NULL \
                  ORDER BY n.nspname, c.relname, a.attnum",
                &[&with_comments],
            )
            .await
            .map_err(sql_err)?;
        for r in crows {
            post.push(format!("COMMENT ON COLUMN {}.{} IS {};", r.get::<_, String>(0), r.get::<_, String>(1), literal(&r.get::<_, String>(2))));
        }
    }
    if o.data {
        for q in &seqs {
            post.push(format!("SELECT pg_catalog.setval({}, {}, {});", literal(&q.quoted), q.last_value, q.is_called));
        }
    }

    plan.pre_data = if o.clean && o.structure {
        // Views primeiro (dependem de tudo), tabelas, sequências, funções e tipos: o CASCADE cobre o resto.
        let mut v: Vec<String> = drops.iter().filter(|d| d.starts_with("DROP VIEW") || d.starts_with("DROP MATERIALIZED")).cloned().collect();
        v.extend(drops.iter().filter(|d| d.starts_with("DROP TABLE")).cloned());
        v.extend(drops.iter().filter(|d| d.starts_with("DROP SEQUENCE")).cloned());
        v.extend(drops.iter().filter(|d| d.starts_with("DROP FUNCTION") || d.starts_with("DROP PROCEDURE")).cloned());
        v.extend(drops.iter().filter(|d| d.starts_with("DROP TYPE") || d.starts_with("DROP DOMAIN")).cloned());
        v.extend(pre);
        v
    } else {
        pre
    };
    plan.post_data = post;
    Ok(plan)
}

/// Colunas copiáveis de uma tabela (todas, inclusive as herdadas; sem as geradas).
async fn copy_columns(s: &PgSession, oid: u32) -> Result<Vec<String>, DatabaseError> {
    let no_generated = if s.server.version_num >= 120000 { "AND a.attgenerated = ''" } else { "" };
    let rows = s
        .client
        .query(
            &format!("SELECT a.attname::text FROM pg_catalog.pg_attribute a WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped {no_generated} ORDER BY a.attnum"),
            &[&oid],
        )
        .await
        .map_err(sql_err)?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// `COPY ... FROM stdin;` (linha de abertura do bloco de dados).
pub fn copy_header(t: &DumpTable) -> String {
    format!("COPY {} ({}) FROM stdin;\n", t.quoted, t.columns.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", "))
}

/// Consulta que lê as linhas de uma tabela (só dela, sem as filhas) para o `COPY` ou para os `INSERT`.
pub fn data_select(t: &DumpTable) -> String {
    format!("SELECT {} FROM ONLY {}", t.columns.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", "), t.quoted)
}

/// Transmite os dados da tabela em formato `COPY` (texto). `sink` recebe cada pedaço e devolve `false` para parar.
/// Devolve as linhas copiadas.
pub async fn copy_table(s: &PgSession, t: &DumpTable, mut sink: impl FnMut(&[u8], u64) -> Result<bool, DatabaseError>) -> Result<u64, DatabaseError> {
    let sql = format!("COPY ({}) TO STDOUT", data_select(t));
    let stream = s.client.copy_out(sql.as_str()).await.map_err(sql_err)?;
    futures_util::pin_mut!(stream);
    let mut rows = 0u64;
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(sql_err)?;
        // Dentro dos dados a quebra de linha vem escapada (`\n`): cada byte 0x0A é o fim de uma linha.
        let lines = bytes.iter().filter(|b| **b == b'\n').count() as u64;
        rows += lines;
        if !sink(&bytes, rows)? {
            break;
        }
    }
    Ok(rows)
}
