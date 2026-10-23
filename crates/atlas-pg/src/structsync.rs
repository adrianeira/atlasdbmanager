//! Structure Synchronization: compara a estrutura de dois schemas (tabelas com colunas, restrições, índices e gatilhos;
//! views; views materializadas; sequências; funções; tipos) e gera o DDL que deixa o destino igual à origem.
//!
//! Cada lado vira um retrato do catálogo (com `search_path = pg_catalog`, para os nomes virem qualificados). O schema da
//! origem é reescrito para o do destino nas definições, de modo que as duas pontas se comparam como texto. O resultado
//! guarda, por objeto, o DDL dos dois lados (para a tela mostrar) e os comandos de cada mudança, marcados com a fase em
//! que devem rodar e a parte (coluna, restrição...) a que pertencem, para que a tela escolha o que implantar.

use crate::connect::PgSession;
use crate::datasync::CompareHooks;
use crate::dump::in_extension;
use crate::error::map_error;
use crate::exec::quote_ident;
use crate::meta::{sequence_detail, type_detail};
use crate::transfer::{adapt_for_target, rewrite_schema};
use atlas_core::{DatabaseError, ErrorKind, SequenceDetail, TypeDetail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructSpec {
    pub source_schema: String,
    pub target_schema: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
    Add,
    Delete,
    Modify,
    Same,
}

/// Parte de uma tabela (coluna, restrição, índice, gatilho ou as opções da tabela).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    /// "column" | "constraint" | "index" | "trigger" | "options" | "primary" | "foreign" | "unique" | "check".
    pub kind: String,
    pub name: String,
    pub op: Op,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    phase: u8,
    /// Ordem dentro da fase.
    order: i64,
    /// Parte a que o comando pertence; `None` vale para o objeto todo.
    part: Option<usize>,
    sql: String,
}

#[derive(Debug, Clone)]
pub struct ObjectDiff {
    /// "table" | "view" | "matview" | "sequence" | "function" | "type".
    pub kind: &'static str,
    pub name: String,
    pub op: Op,
    pub parts: Vec<Part>,
    pub source_ddl: String,
    pub target_ddl: String,
    stmts: Vec<Stmt>,
}

#[derive(Debug, Default)]
pub struct StructComparison {
    pub objects: Vec<ObjectDiff>,
}

/// O que o usuário marcou de um objeto: as partes (`None` = todas).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StructSelection {
    pub kind: String,
    pub name: String,
    pub parts: Option<Vec<usize>>,
}

fn sql_err(e: tokio_postgres::Error) -> DatabaseError {
    map_error(&e)
}

fn canceled() -> DatabaseError {
    DatabaseError::new(ErrorKind::Canceled, "synchronization canceled")
}

fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

// ---------------------------------------------------------------- retrato do catálogo

#[derive(Clone)]
struct Col {
    name: String,
    data_type: String,
    not_null: bool,
    default: Option<String>,
    identity: String,
    generated: String,
    collation: Option<String>,
    comment: Option<String>,
}

#[derive(Clone)]
struct Named {
    name: String,
    /// Restrições: o tipo (`p`, `u`, `c`, `x`, `f`).
    kind: String,
    def: String,
}

#[derive(Clone)]
struct TableSnap {
    name: String,
    unlogged: bool,
    /// `reloptions` (`fillfactor=70`...), em ordem.
    options: Vec<String>,
    partition_key: Option<String>,
    partition_bound: Option<String>,
    is_partition: bool,
    parents: Vec<String>,
    owner: String,
    comment: Option<String>,
    columns: Vec<Col>,
    constraints: Vec<Named>,
    indexes: Vec<Named>,
    triggers: Vec<Named>,
}

#[derive(Clone)]
struct ViewSnap {
    name: String,
    materialized: bool,
    def: String,
    owner: String,
    comment: Option<String>,
    /// Índices da view materializada.
    indexes: Vec<Named>,
}

#[derive(Clone)]
struct SeqSnap {
    name: String,
    detail: SequenceDetail,
}

#[derive(Clone)]
struct FuncSnap {
    /// `nome(argumentos)`.
    key: String,
    procedure: bool,
    def: String,
    comment: Option<String>,
}

#[derive(Clone)]
struct TypeSnap {
    name: String,
    detail: TypeDetail,
}

#[derive(Default, Clone)]
struct Snapshot {
    tables: Vec<TableSnap>,
    views: Vec<ViewSnap>,
    seqs: Vec<SeqSnap>,
    funcs: Vec<FuncSnap>,
    types: Vec<TypeSnap>,
}

/// Fração do trabalho para a barra: cada retrato ocupa metade.
async fn load(s: &PgSession, schema: &str, hooks: &mut CompareHooks<'_>, side: &str, base: f64) -> Result<Snapshot, DatabaseError> {
    let v = s.server.version_num;
    if v < 90300 {
        return Err(DatabaseError::new(ErrorKind::UnsupportedFeature, "Structure Synchronization requires PostgreSQL 9.3 or later"));
    }
    s.client.batch_execute("SET search_path TO pg_catalog").await.map_err(sql_err)?;
    let schemas = vec![schema.to_string()];
    let mut snap = Snapshot::default();
    let step = |text: &str, n: f64, hooks: &mut CompareHooks<'_>| -> Result<(), DatabaseError> {
        if (hooks.cancelled)() {
            return Err(canceled());
        }
        (hooks.progress)(&format!("{text} ({side})..."), base + n * 0.5);
        Ok(())
    };

    // ---- Tabelas.
    step("Getting table info", 0.0, hooks)?;
    let part_key = if v >= 100000 { "CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END" } else { "NULL::text" };
    let part_bound = if v >= 100000 { "CASE WHEN c.relispartition THEN pg_get_expr(c.relpartbound, c.oid) END" } else { "NULL::text" };
    let is_part = if v >= 100000 { "c.relispartition" } else { "false" };
    let kinds = if v >= 100000 { "'r','p'" } else { "'r'" };
    let rows = s
        .client
        .query(
            &format!(
                "SELECT c.oid, c.relname::text, c.relpersistence = 'u', {part_key}, {part_bound}, {is_part}, \
                        ARRAY(SELECT quote_ident(pn.nspname) || '.' || quote_ident(pc.relname) FROM pg_catalog.pg_inherits i \
                                JOIN pg_catalog.pg_class pc ON pc.oid = i.inhparent JOIN pg_catalog.pg_namespace pn ON pn.oid = pc.relnamespace \
                               WHERE i.inhrelid = c.oid ORDER BY i.inhseqno), \
                        pg_get_userbyid(c.relowner)::text, obj_description(c.oid, 'pg_class'), \
                        ARRAY(SELECT o FROM unnest(COALESCE(c.reloptions, ARRAY[]::text[])) o ORDER BY o) \
                   FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  WHERE n.nspname = ANY($1) AND c.relkind IN ({kinds}) AND {} ORDER BY c.relname",
                in_extension("pg_class", "c.oid")
            ),
            &[&schemas],
        )
        .await
        .map_err(sql_err)?;
    let mut oids: Vec<u32> = Vec::new();
    let mut by_oid: HashMap<u32, usize> = HashMap::new();
    for r in rows {
        by_oid.insert(r.get(0), snap.tables.len());
        oids.push(r.get(0));
        snap.tables.push(TableSnap {
            name: r.get(1),
            unlogged: r.get(2),
            options: r.get(9),
            partition_key: r.get(3),
            partition_bound: r.get(4),
            is_partition: r.get(5),
            parents: r.get(6),
            owner: r.get(7),
            comment: r.get(8),
            columns: Vec::new(),
            constraints: Vec::new(),
            indexes: Vec::new(),
            triggers: Vec::new(),
        });
    }

    step("Getting table fields", 0.2, hooks)?;
    let identity = if v >= 100000 { "a.attidentity::text" } else { "''::text" };
    let generated = if v >= 120000 { "a.attgenerated::text" } else { "''::text" };
    let rows = s
        .client
        .query(
            &format!(
                "SELECT a.attrelid, a.attname::text, format_type(a.atttypid, a.atttypmod), a.attnotnull, pg_get_expr(d.adbin, d.adrelid), {identity}, {generated}, \
                        (SELECT quote_ident(cn.nspname) || '.' || quote_ident(co.collname) FROM pg_catalog.pg_collation co \
                           JOIN pg_catalog.pg_namespace cn ON cn.oid = co.collnamespace \
                          WHERE co.oid = a.attcollation AND a.attcollation <> t.typcollation), \
                        col_description(a.attrelid, a.attnum) \
                   FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_type t ON t.oid = a.atttypid \
                   LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
                  WHERE a.attrelid = ANY($1) AND a.attnum > 0 AND NOT a.attisdropped AND a.attislocal ORDER BY a.attrelid, a.attnum"
            ),
            &[&oids],
        )
        .await
        .map_err(sql_err)?;
    for r in rows {
        if let Some(&i) = by_oid.get(&r.get::<_, u32>(0)) {
            snap.tables[i].columns.push(Col {
                name: r.get(1),
                data_type: r.get(2),
                not_null: r.get(3),
                default: r.get(4),
                identity: r.get(5),
                generated: r.get(6),
                collation: r.get(7),
                comment: r.get(8),
            });
        }
    }

    step("Getting table constraints", 0.4, hooks)?;
    let rows = s
        .client
        .query(
            "SELECT con.conrelid, con.conname::text, con.contype::text, pg_get_constraintdef(con.oid) \
               FROM pg_catalog.pg_constraint con \
              WHERE con.conrelid = ANY($1) AND con.contype IN ('p','u','c','x','f') AND con.conislocal ORDER BY con.conname",
            &[&oids],
        )
        .await
        .map_err(sql_err)?;
    for r in rows {
        if let Some(&i) = by_oid.get(&r.get::<_, u32>(0)) {
            snap.tables[i].constraints.push(Named { name: r.get(1), kind: r.get(2), def: r.get(3) });
        }
    }
    let ispart = if v >= 110000 { "AND NOT ic.relispartition" } else { "" };
    let rows = s
        .client
        .query(
            &format!(
                "SELECT i.indrelid, ic.relname::text, pg_get_indexdef(i.indexrelid) FROM pg_catalog.pg_index i \
                   JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid \
                  WHERE i.indrelid = ANY($1) AND NOT i.indisprimary {ispart} \
                    AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_constraint c WHERE c.conindid = i.indexrelid AND c.contype IN ('p','u','x')) \
                  ORDER BY ic.relname"
            ),
            &[&oids],
        )
        .await
        .map_err(sql_err)?;
    for r in rows {
        if let Some(&i) = by_oid.get(&r.get::<_, u32>(0)) {
            snap.tables[i].indexes.push(Named { name: r.get(1), kind: String::new(), def: r.get(2) });
        }
    }
    let trg_filter = if v >= 130000 { "AND t.tgparentid = 0" } else { "" };
    let rows = s
        .client
        .query(
            &format!(
                "SELECT t.tgrelid, t.tgname::text, pg_get_triggerdef(t.oid, true) FROM pg_catalog.pg_trigger t \
                  WHERE t.tgrelid = ANY($1) AND NOT t.tgisinternal {trg_filter} ORDER BY t.tgname"
            ),
            &[&oids],
        )
        .await
        .map_err(sql_err)?;
    for r in rows {
        if let Some(&i) = by_oid.get(&r.get::<_, u32>(0)) {
            snap.tables[i].triggers.push(Named { name: r.get(1), kind: String::new(), def: r.get(2) });
        }
    }

    // ---- Views e views materializadas.
    step("Getting view info", 0.6, hooks)?;
    let rows = s
        .client
        .query(
            &format!(
                "SELECT c.oid, c.relname::text, c.relkind = 'm', pg_get_viewdef(c.oid, true), pg_get_userbyid(c.relowner)::text, obj_description(c.oid, 'pg_class') \
                   FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  WHERE c.relkind IN ('v','m') AND n.nspname = ANY($1) AND {} ORDER BY c.oid",
                in_extension("pg_class", "c.oid")
            ),
            &[&schemas],
        )
        .await
        .map_err(sql_err)?;
    let mut view_oids: HashMap<u32, usize> = HashMap::new();
    for r in rows {
        view_oids.insert(r.get(0), snap.views.len());
        snap.views.push(ViewSnap { name: r.get(1), materialized: r.get(2), def: r.get(3), owner: r.get(4), comment: r.get(5), indexes: Vec::new() });
    }
    let ids: Vec<u32> = view_oids.keys().copied().collect();
    let rows = s
        .client
        .query(
            "SELECT i.indrelid, ic.relname::text, pg_get_indexdef(i.indexrelid) FROM pg_catalog.pg_index i \
               JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid WHERE i.indrelid = ANY($1) ORDER BY ic.relname",
            &[&ids],
        )
        .await
        .map_err(sql_err)?;
    for r in rows {
        if let Some(&i) = view_oids.get(&r.get::<_, u32>(0)) {
            snap.views[i].indexes.push(Named { name: r.get(1), kind: String::new(), def: r.get(2) });
        }
    }

    // ---- Sequências (as de identidade pertencem à coluna).
    step("Getting sequence info", 0.7, hooks)?;
    let rows = s
        .client
        .query(
            &format!(
                "SELECT c.relname::text FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  WHERE c.relkind = 'S' AND n.nspname = ANY($1) AND {} \
                    AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_depend d WHERE d.classid = 'pg_catalog.pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'i') \
                  ORDER BY c.relname",
                in_extension("pg_class", "c.oid")
            ),
            &[&schemas],
        )
        .await
        .map_err(sql_err)?;
    for r in rows {
        let name: String = r.get(0);
        let detail = sequence_detail(s, schema, &name).await?;
        snap.seqs.push(SeqSnap { name, detail });
    }

    // ---- Funções e procedimentos.
    step("Getting function info", 0.8, hooks)?;
    let fn_filter = if v >= 110000 { "p.prokind IN ('f','p')" } else { "NOT p.proisagg AND NOT p.proiswindow" };
    let is_proc = if v >= 110000 { "p.prokind = 'p'" } else { "false" };
    let rows = s
        .client
        .query(
            &format!(
                "SELECT p.proname::text || '(' || pg_get_function_identity_arguments(p.oid) || ')', {is_proc}, pg_get_functiondef(p.oid), obj_description(p.oid, 'pg_proc') \
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
        snap.funcs.push(FuncSnap { key: r.get(0), procedure: r.get(1), def: r.get(2), comment: r.get(3) });
    }

    // ---- Tipos.
    step("Getting type info", 0.9, hooks)?;
    let rows = s
        .client
        .query(
            &format!(
                "SELECT t.typname::text FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
                  WHERE n.nspname = ANY($1) AND (t.typtype IN ('e','d','r') \
                        OR (t.typtype = 'c' AND EXISTS (SELECT 1 FROM pg_catalog.pg_class tc WHERE tc.oid = t.typrelid AND tc.relkind = 'c'))) \
                    AND {} ORDER BY t.typname",
                in_extension("pg_type", "t.oid")
            ),
            &[&schemas],
        )
        .await
        .map_err(sql_err)?;
    for r in rows {
        let name: String = r.get(0);
        let detail = type_detail(s, schema, &name).await?;
        snap.types.push(TypeSnap { name, detail });
    }
    Ok(snap)
}

/// `rewrite_schema` sempre põe aspas no schema novo, mas o catálogo só as usa quando o nome precisa. Tira as aspas de um
/// nome simples para que o texto reescrito fique igual ao que o destino devolve.
fn retarget_text(sql: &str, from: &str, to: &str) -> String {
    let out = rewrite_schema(sql, from, to);
    let simple = to.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == '_') && to.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '$'));
    if simple { out.replace(&format!("\"{to}\"."), &format!("{to}.")) } else { out }
}

/// Troca o schema da origem pelo do destino em tudo o que o catálogo devolveu como texto de definição.
fn retarget(snap: &mut Snapshot, from: &str, to: &str) {
    if from == to {
        return;
    }
    let rw = |s: &mut String| *s = retarget_text(s, from, to);
    let rw_opt = |s: &mut Option<String>| {
        if let Some(x) = s {
            *x = retarget_text(x, from, to);
        }
    };
    for t in &mut snap.tables {
        for c in &mut t.columns {
            rw(&mut c.data_type);
            rw_opt(&mut c.default);
            rw_opt(&mut c.collation);
        }
        for n in t.constraints.iter_mut().chain(&mut t.indexes).chain(&mut t.triggers) {
            rw(&mut n.def);
        }
        t.parents.iter_mut().for_each(rw);
        rw_opt(&mut t.partition_key);
        rw_opt(&mut t.partition_bound);
    }
    for v in &mut snap.views {
        rw(&mut v.def);
        v.indexes.iter_mut().for_each(|n| rw(&mut n.def));
    }
    for q in &mut snap.seqs {
        rw_opt(&mut q.detail.owned_by);
    }
    for f in &mut snap.funcs {
        rw(&mut f.def);
    }
    for t in &mut snap.types {
        rw_opt(&mut t.detail.base_type);
        rw_opt(&mut t.detail.default);
        rw_opt(&mut t.detail.subtype);
        t.detail.attributes.iter_mut().for_each(|a| rw(&mut a.data_type));
        t.detail.checks.iter_mut().for_each(|c| rw(&mut c.definition));
    }
}

// ---------------------------------------------------------------- DDL

struct Ctx {
    schema: String,
    target_version: i32,
    /// Os servidores são de versões principais diferentes: o mesmo objeto sai escrito de outro jeito (`SELECT id` ou
    /// `SELECT clientes.id`), então as definições se comparam sem espaços, parênteses e qualificadores.
    loose: bool,
    /// Um dos lados é anterior ao PostgreSQL 10: a sequência é sempre `bigint`, então os limites não dizem nada.
    legacy_sequences: bool,
}

/// Forma usada para comparar definições de servidores diferentes.
fn loosen(sql: &str) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() || c == '(' || c == ')' {
            i += 1;
        } else if c == '"' {
            // `"Nome".` (qualificador) some; o que vem depois fica.
            let end = chars[i + 1..].iter().position(|x| *x == '"').map(|e| i + 1 + e + 1).unwrap_or(chars.len());
            if chars.get(end) == Some(&'.') {
                i = end + 1;
            } else {
                out.extend(&chars[i..end]);
                i = end;
            }
        } else if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || matches!(chars[j], '_' | '$')) {
                j += 1;
            }
            // `nome.` seguido de outro nome é qualificador.
            if chars.get(j) == Some(&'.') && chars.get(j + 1).is_some_and(|x| x.is_alphabetic() || matches!(x, '_' | '"')) {
                i = j + 1;
            } else {
                out.extend(&chars[i..j]);
                i = j;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    // Antes do PostgreSQL 11 o gatilho diz `EXECUTE PROCEDURE`; depois, `EXECUTE FUNCTION`.
    out.replace("EXECUTEPROCEDURE", "EXECUTEFUNCTION")
}

impl Ctx {
    fn same(&self, a: &str, b: &str) -> bool {
        a == b || (self.loose && loosen(a) == loosen(b))
    }

    fn same_opt(&self, a: &Option<String>, b: &Option<String>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => self.same(a, b),
            (None, None) => true,
            _ => false,
        }
    }

    fn q(&self, name: &str) -> String {
        format!("{}.{}", quote_ident(&self.schema), quote_ident(name))
    }
}

fn column_line(c: &Col) -> String {
    let mut l = format!("{} {}", quote_ident(&c.name), c.data_type);
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
}

fn create_table(cx: &Ctx, t: &TableSnap) -> String {
    let q = cx.q(&t.name);
    let persistence = if t.unlogged { "UNLOGGED " } else { "" };
    let tail = t.partition_key.as_ref().map(|k| format!(" PARTITION BY {k}")).unwrap_or_default();
    if t.is_partition {
        let parent = t.parents.first().cloned().unwrap_or_default();
        return format!("CREATE {persistence}TABLE {q} PARTITION OF {parent} {}{tail}{}\n;", t.partition_bound.clone().unwrap_or_default(), with_options(t));
    }
    let mut lines: Vec<String> = t.columns.iter().map(|c| format!("  {}", column_line(c))).collect();
    lines.extend(t.constraints.iter().filter(|c| c.kind != "f").map(|c| format!("  CONSTRAINT {} {}", quote_ident(&c.name), c.def)));
    let inherits = if t.parents.is_empty() { String::new() } else { format!("\nINHERITS ({})", t.parents.join(", ")) };
    let body = if lines.is_empty() { "()".to_string() } else { format!("(\n{}\n)", lines.join(",\n")) };
    format!("CREATE {persistence}TABLE {q} {body}{inherits}{tail}{}\n;", with_options(t))
}

/// ` WITH (fillfactor=70)` ou nada.
fn with_options(t: &TableSnap) -> String {
    if t.options.is_empty() { String::new() } else { format!(" WITH ({})", t.options.join(", ")) }
}

fn add_column(q: &str, c: &Col) -> String {
    format!("ALTER TABLE {q} ADD COLUMN {};", column_line(c))
}

fn comment_on(kind: &str, target: &str, comment: &Option<String>) -> String {
    format!("COMMENT ON {kind} {target} IS {};", comment.as_ref().map(|c| literal(c)).unwrap_or_else(|| "NULL".into()))
}

fn table_ddl(cx: &Ctx, t: &TableSnap) -> String {
    let q = cx.q(&t.name);
    let mut out = vec![create_table(cx, t)];
    out.push(format!("ALTER TABLE {q} OWNER TO {};", quote_ident(&t.owner)));
    for c in &t.columns {
        if c.comment.is_some() {
            out.push(comment_on("COLUMN", &format!("{q}.{}", quote_ident(&c.name)), &c.comment));
        }
    }
    if t.comment.is_some() {
        out.push(comment_on("TABLE", &q, &t.comment));
    }
    out.extend(t.indexes.iter().map(|i| format!("{};", i.def)));
    out.extend(t.triggers.iter().map(|g| format!("{};", g.def)));
    out.extend(t.constraints.iter().filter(|c| c.kind == "f").map(|c| format!("ALTER TABLE {q} ADD CONSTRAINT {} {};", quote_ident(&c.name), c.def)));
    out.join("\n\n")
}

fn view_create(cx: &Ctx, v: &ViewSnap, replace: bool) -> String {
    let q = cx.q(&v.name);
    let body = v.def.trim_end().trim_end_matches(';');
    if v.materialized {
        format!("CREATE MATERIALIZED VIEW {q} AS\n{body};")
    } else {
        format!("CREATE {}VIEW {q} AS\n{body};", if replace { "OR REPLACE " } else { "" })
    }
}

fn view_ddl(cx: &Ctx, v: &ViewSnap) -> String {
    let kw = if v.materialized { "MATERIALIZED VIEW" } else { "VIEW" };
    let q = cx.q(&v.name);
    let mut out = vec![view_create(cx, v, false), format!("ALTER {kw} {q} OWNER TO {};", quote_ident(&v.owner))];
    if v.comment.is_some() {
        out.push(comment_on(kw, &q, &v.comment));
    }
    out.extend(v.indexes.iter().map(|i| format!("{};", i.def)));
    out.join("\n\n")
}

fn sequence_create(cx: &Ctx, d: &SequenceDetail, with_type: bool) -> String {
    let q = cx.q(&d.name);
    let as_clause = if with_type && d.data_type != "bigint" { format!("    AS {}\n", d.data_type) } else { String::new() };
    format!(
        "CREATE SEQUENCE {q}\n{as_clause}    START WITH {}\n    INCREMENT BY {}\n    MINVALUE {}\n    MAXVALUE {}\n    CACHE {}{};",
        d.start,
        d.increment,
        d.min,
        d.max,
        d.cache,
        if d.cycle { "\n    CYCLE" } else { "" }
    )
}

fn sequence_alter(cx: &Ctx, d: &SequenceDetail) -> String {
    format!(
        "ALTER SEQUENCE {} INCREMENT BY {} MINVALUE {} MAXVALUE {} START WITH {} CACHE {} {};",
        cx.q(&d.name),
        d.increment,
        d.min,
        d.max,
        d.start,
        d.cache,
        if d.cycle { "CYCLE" } else { "NO CYCLE" }
    )
}

fn sequence_same(cx: &Ctx, a: &SequenceDetail, b: &SequenceDetail) -> bool {
    (&a.start, &a.increment, &a.cache, a.cycle) == (&b.start, &b.increment, &b.cache, b.cycle) && (cx.legacy_sequences || (&a.min, &a.max) == (&b.min, &b.max))
}

fn type_create(cx: &Ctx, t: &TypeSnap) -> String {
    let q = cx.q(&t.name);
    let d = &t.detail;
    match d.kind.as_str() {
        "enum" => format!("CREATE TYPE {q} AS ENUM ({});", d.labels.iter().map(|l| literal(l)).collect::<Vec<_>>().join(", ")),
        "composite" => format!(
            "CREATE TYPE {q} AS (\n    {}\n);",
            d.attributes.iter().map(|a| format!("{} {}", quote_ident(&a.name), a.data_type)).collect::<Vec<_>>().join(",\n    ")
        ),
        "domain" => {
            let mut parts = vec![format!("CREATE DOMAIN {q} AS {}", d.base_type.clone().unwrap_or_default())];
            if let Some(x) = &d.default {
                parts.push(format!("DEFAULT {x}"));
            }
            if d.not_null {
                parts.push("NOT NULL".into());
            }
            for c in &d.checks {
                parts.push(format!("CONSTRAINT {} {}", quote_ident(&c.name), c.definition));
            }
            format!("{};", parts.join("\n    "))
        }
        _ => format!("CREATE TYPE {q} AS RANGE (subtype = {});", d.subtype.clone().unwrap_or_default()),
    }
}

fn type_ddl(cx: &Ctx, t: &TypeSnap) -> String {
    let mut out = vec![type_create(cx, t)];
    if t.detail.comment.is_some() {
        let kw = if t.detail.kind == "domain" { "DOMAIN" } else { "TYPE" };
        out.push(comment_on(kw, &cx.q(&t.name), &t.detail.comment));
    }
    out.join("\n\n")
}

fn func_ddl(f: &FuncSnap) -> String {
    format!("{};", f.def.trim_end().trim_end_matches(';'))
}

// ---------------------------------------------------------------- comparação

// Fases do script: o que precisa existir (ou deixar de existir) antes do próximo passo.
const DROP_DEPENDENTS: u8 = 0;
const CREATE_BASE: u8 = 1;
const TABLES: u8 = 2;
const TABLE_PARTS: u8 = 3;
const FOREIGN_KEYS: u8 = 4;
const VIEWS: u8 = 5;
const DROP_OBJECTS: u8 = 6;

/// Ajusta um comando de um servidor novo a um destino antigo: `AS tipo` nas sequências (antes do 10) e `EXECUTE FUNCTION`
/// nos gatilhos (antes do 11).
fn adapt(sql: &str, target_version: i32) -> String {
    let sql = adapt_for_target(sql, target_version);
    if target_version < 110000 && (sql.starts_with("CREATE TRIGGER ") || sql.starts_with("CREATE CONSTRAINT TRIGGER ")) {
        return sql.replace(" EXECUTE FUNCTION ", " EXECUTE PROCEDURE ");
    }
    sql
}

struct Out<'a> {
    cx: &'a Ctx,
    stmts: Vec<Stmt>,
    parts: Vec<Part>,
}

impl Out<'_> {
    fn push(&mut self, phase: u8, order: i64, part: Option<usize>, sql: String) {
        self.stmts.push(Stmt { phase, order, part, sql: adapt(&sql, self.cx.target_version) });
    }
}

fn part_kind<'a>(kind: &'a str, constraint: &str) -> &'a str {
    match (kind, constraint) {
        ("constraint", "p") => "primary",
        ("constraint", "u") => "unique",
        ("constraint", "f") => "foreign",
        ("constraint", "c") => "check",
        (k, _) => k,
    }
}

fn diff_named(o: &mut Out, kind: &str, src: &[Named], dst: &[Named], order: i64, drop_sql: &dyn Fn(&Named) -> String, add_sql: &dyn Fn(&Named) -> (u8, String)) {
    let dst_by: HashMap<&str, &Named> = dst.iter().map(|n| (n.name.as_str(), n)).collect();
    for s in src {
        let part_kind = part_kind(kind, &s.kind);
        let idx = o.parts.len();
        match dst_by.get(s.name.as_str()) {
            None => {
                o.parts.push(Part { kind: part_kind.into(), name: s.name.clone(), op: Op::Add });
                let (phase, sql) = add_sql(s);
                o.push(phase, order, Some(idx), sql);
            }
            Some(d) if !o.cx.same(&d.def, &s.def) => {
                o.parts.push(Part { kind: part_kind.into(), name: s.name.clone(), op: Op::Modify });
                o.push(DROP_DEPENDENTS, order, Some(idx), drop_sql(d));
                let (phase, sql) = add_sql(s);
                o.push(phase, order, Some(idx), sql);
            }
            Some(_) => o.parts.push(Part { kind: part_kind.into(), name: s.name.clone(), op: Op::Same }),
        }
    }
    let src_names: std::collections::HashSet<&str> = src.iter().map(|n| n.name.as_str()).collect();
    for d in dst.iter().filter(|d| !src_names.contains(d.name.as_str())) {
        let part_kind = part_kind(kind, &d.kind);
        let idx = o.parts.len();
        o.parts.push(Part { kind: part_kind.into(), name: d.name.clone(), op: Op::Delete });
        o.push(DROP_DEPENDENTS, order, Some(idx), drop_sql(d));
    }
}

fn col_same(cx: &Ctx, a: &Col, b: &Col) -> bool {
    a.data_type == b.data_type
        && a.not_null == b.not_null
        && cx.same_opt(&a.default, &b.default)
        && (&a.identity, &a.generated, &a.collation, &a.comment) == (&b.identity, &b.generated, &b.collation, &b.comment)
}

fn alter_column(o: &mut Out, cx: &Ctx, q: &str, order: i64, idx: usize, s: &Col, d: &Col) {
    let col = quote_ident(&s.name);
    let alter = |what: String| format!("ALTER TABLE {q} ALTER COLUMN {col} {what};");
    if s.data_type != d.data_type || s.collation != d.collation {
        let collate = s.collation.as_ref().map(|c| format!(" COLLATE {c}")).unwrap_or_default();
        o.push(TABLES, order, Some(idx), alter(format!("TYPE {}{collate}", s.data_type)));
    }
    if s.generated != d.generated || (s.generated == "s" && !cx.same_opt(&s.default, &d.default)) {
        o.push(TABLES, order, Some(idx), format!("-- column {col}: the generated expression differs and cannot be altered automatically"));
    } else if s.generated.is_empty() && s.identity.is_empty() && !cx.same_opt(&s.default, &d.default) {
        match &s.default {
            Some(x) => o.push(TABLES, order, Some(idx), alter(format!("SET DEFAULT {x}"))),
            None => o.push(TABLES, order, Some(idx), alter("DROP DEFAULT".into())),
        }
    }
    if s.identity != d.identity {
        let kind = if s.identity == "a" { "ALWAYS" } else { "BY DEFAULT" };
        match (s.identity.as_str(), d.identity.as_str()) {
            ("", _) => o.push(TABLES, order, Some(idx), alter("DROP IDENTITY".into())),
            (_, "") => o.push(TABLES, order, Some(idx), alter(format!("ADD GENERATED {kind} AS IDENTITY"))),
            _ => o.push(TABLES, order, Some(idx), alter(format!("SET GENERATED {kind}"))),
        }
    }
    if s.not_null != d.not_null {
        o.push(TABLES, order, Some(idx), alter(if s.not_null { "SET NOT NULL" } else { "DROP NOT NULL" }.into()));
    }
    if s.comment != d.comment {
        o.push(TABLES, order, Some(idx), comment_on("COLUMN", &format!("{q}.{col}"), &s.comment));
    }
}

/// Comandos que igualam as opções da tabela: comentário, `reloptions` e `LOGGED`/`UNLOGGED`.
fn table_option_changes(q: &str, s: &TableSnap, d: &TableSnap) -> Vec<String> {
    let mut out = Vec::new();
    if s.comment != d.comment {
        out.push(comment_on("TABLE", q, &s.comment));
    }
    if s.unlogged != d.unlogged {
        out.push(format!("ALTER TABLE {q} SET {};", if s.unlogged { "UNLOGGED" } else { "LOGGED" }));
    }
    let set: Vec<&String> = s.options.iter().filter(|o| !d.options.contains(o)).collect();
    if !set.is_empty() {
        out.push(format!("ALTER TABLE {q} SET ({});", set.iter().map(|o| o.as_str()).collect::<Vec<_>>().join(", ")));
    }
    let key = |o: &str| o.split('=').next().unwrap_or(o).to_string();
    let kept: Vec<String> = s.options.iter().map(|o| key(o)).collect();
    let reset: Vec<String> = d.options.iter().map(|o| key(o)).filter(|k| !kept.contains(k)).collect();
    if !reset.is_empty() {
        out.push(format!("ALTER TABLE {q} RESET ({});", reset.join(", ")));
    }
    out
}

fn diff_table(cx: &Ctx, order: i64, src: Option<&TableSnap>, dst: Option<&TableSnap>) -> ObjectDiff {
    let name = src.or(dst).map(|t| t.name.clone()).unwrap_or_default();
    let q = cx.q(&name);
    let mut o = Out { cx, stmts: Vec::new(), parts: Vec::new() };
    let source_ddl = src.map(|t| table_ddl(cx, t)).unwrap_or_default();
    let target_ddl = dst.map(|t| table_ddl(cx, t)).unwrap_or_default();
    let op = match (src, dst) {
        (Some(t), None) => {
            o.push(TABLES, order, None, create_table(cx, t));
            for c in &t.columns {
                if c.comment.is_some() {
                    o.push(TABLE_PARTS, order, None, comment_on("COLUMN", &format!("{q}.{}", quote_ident(&c.name)), &c.comment));
                }
            }
            if t.comment.is_some() {
                o.push(TABLE_PARTS, order, None, comment_on("TABLE", &q, &t.comment));
            }
            for i in &t.indexes {
                o.push(TABLE_PARTS, order, None, format!("{};", i.def));
            }
            for g in &t.triggers {
                o.push(TABLE_PARTS, order, None, format!("{};", g.def));
            }
            for c in t.constraints.iter().filter(|c| c.kind == "f") {
                o.push(FOREIGN_KEYS, order, None, format!("ALTER TABLE {q} ADD CONSTRAINT {} {};", quote_ident(&c.name), c.def));
            }
            Op::Add
        }
        (None, Some(_)) => {
            o.push(DROP_OBJECTS, order, None, format!("DROP TABLE {q};"));
            Op::Delete
        }
        (Some(s), Some(d)) => {
            let dst_cols: HashMap<&str, &Col> = d.columns.iter().map(|c| (c.name.as_str(), c)).collect();
            for c in &s.columns {
                let idx = o.parts.len();
                match dst_cols.get(c.name.as_str()) {
                    None => {
                        o.parts.push(Part { kind: "column".into(), name: c.name.clone(), op: Op::Add });
                        o.push(TABLES, order, Some(idx), add_column(&q, c));
                        if c.comment.is_some() {
                            o.push(TABLES, order, Some(idx), comment_on("COLUMN", &format!("{q}.{}", quote_ident(&c.name)), &c.comment));
                        }
                    }
                    Some(t) if !col_same(cx, c, t) => {
                        o.parts.push(Part { kind: "column".into(), name: c.name.clone(), op: Op::Modify });
                        alter_column(&mut o, cx, &q, order, idx, c, t);
                    }
                    Some(_) => o.parts.push(Part { kind: "column".into(), name: c.name.clone(), op: Op::Same }),
                }
            }
            let src_cols: std::collections::HashSet<&str> = s.columns.iter().map(|c| c.name.as_str()).collect();
            for c in d.columns.iter().filter(|c| !src_cols.contains(c.name.as_str())) {
                let idx = o.parts.len();
                o.parts.push(Part { kind: "column".into(), name: c.name.clone(), op: Op::Delete });
                o.push(TABLES, order, Some(idx), format!("ALTER TABLE {q} DROP COLUMN {};", quote_ident(&c.name)));
            }
            let (qc, qi, qt) = (q.clone(), cx.schema.clone(), q.clone());
            diff_named(
                &mut o,
                "constraint",
                &s.constraints,
                &d.constraints,
                order,
                &|n| format!("ALTER TABLE {qc} DROP CONSTRAINT {};", quote_ident(&n.name)),
                &|n| (if n.kind == "f" { FOREIGN_KEYS } else { TABLE_PARTS }, format!("ALTER TABLE {qc} ADD CONSTRAINT {} {};", quote_ident(&n.name), n.def)),
            );
            diff_named(&mut o, "index", &s.indexes, &d.indexes, order, &|n| format!("DROP INDEX {}.{};", quote_ident(&qi), quote_ident(&n.name)), &|n| (TABLE_PARTS, format!("{};", n.def)));
            diff_named(&mut o, "trigger", &s.triggers, &d.triggers, order, &|n| format!("DROP TRIGGER {} ON {qt};", quote_ident(&n.name)), &|n| (TABLE_PARTS, format!("{};", n.def)));
            let idx = o.parts.len();
            let changes = table_option_changes(&q, s, d);
            o.parts.push(Part { kind: "options".into(), name: "(Table Options)".into(), op: if changes.is_empty() { Op::Same } else { Op::Modify } });
            for sql in changes {
                o.push(TABLES, order, Some(idx), sql);
            }
            if o.parts.iter().all(|p| p.op == Op::Same) { Op::Same } else { Op::Modify }
        }
        (None, None) => Op::Same,
    };
    ObjectDiff { kind: "table", name, op, parts: o.parts, source_ddl, target_ddl, stmts: o.stmts }
}

fn diff_view(cx: &Ctx, order: i64, src: Option<&ViewSnap>, dst: Option<&ViewSnap>) -> ObjectDiff {
    let v = src.or(dst).expect("view");
    let (kind, kw) = if v.materialized { ("matview", "MATERIALIZED VIEW") } else { ("view", "VIEW") };
    let q = cx.q(&v.name);
    let mut o = Out { cx, stmts: Vec::new(), parts: Vec::new() };
    let source_ddl = src.map(|v| view_ddl(cx, v)).unwrap_or_default();
    let target_ddl = dst.map(|v| view_ddl(cx, v)).unwrap_or_default();
    let create = |o: &mut Out, v: &ViewSnap, replace: bool| {
        o.push(VIEWS, order, None, view_create(cx, v, replace));
        if v.comment.is_some() {
            o.push(VIEWS, order, None, comment_on(kw, &q, &v.comment));
        }
        for i in &v.indexes {
            o.push(VIEWS, order, None, format!("{};", i.def));
        }
    };
    let op = match (src, dst) {
        (Some(s), None) => {
            create(&mut o, s, false);
            Op::Add
        }
        (None, Some(_)) => {
            o.push(DROP_DEPENDENTS, -order, None, format!("DROP {kw} {q};"));
            Op::Delete
        }
        (Some(s), Some(d)) => {
            let same = cx.same(s.def.trim_end().trim_end_matches(';'), d.def.trim_end().trim_end_matches(';'))
                && s.comment == d.comment
                && s.indexes.iter().map(|i| (&i.name, &i.def)).zip(d.indexes.iter().map(|i| (&i.name, &i.def))).all(|(a, b)| a.0 == b.0 && cx.same(a.1, b.1))
                && s.indexes.len() == d.indexes.len();
            if same {
                Op::Same
            } else if s.materialized {
                // Uma view materializada não troca de definição: sai e volta.
                o.push(DROP_DEPENDENTS, -order, None, format!("DROP {kw} {q};"));
                create(&mut o, s, false);
                Op::Modify
            } else {
                create(&mut o, s, true);
                Op::Modify
            }
        }
        (None, None) => Op::Same,
    };
    ObjectDiff { kind, name: v.name.clone(), op, parts: Vec::new(), source_ddl, target_ddl, stmts: o.stmts }
}

fn diff_sequence(cx: &Ctx, order: i64, src: Option<&SeqSnap>, dst: Option<&SeqSnap>) -> ObjectDiff {
    let name = src.or(dst).map(|s| s.name.clone()).unwrap_or_default();
    let mut o = Out { cx, stmts: Vec::new(), parts: Vec::new() };
    let with_type = |d: &SequenceDetail| cx.target_version >= 100000 && d.data_type != "bigint";
    let ddl = |s: &SeqSnap| sequence_create(cx, &s.detail, with_type(&s.detail));
    let op = match (src, dst) {
        (Some(s), None) => {
            o.push(CREATE_BASE, order, None, sequence_create(cx, &s.detail, with_type(&s.detail)));
            if let Some(owner) = &s.detail.owned_by {
                o.push(TABLE_PARTS, order, None, format!("ALTER SEQUENCE {} OWNED BY {owner};", cx.q(&s.name)));
            }
            Op::Add
        }
        (None, Some(_)) => {
            o.push(DROP_OBJECTS, 1_000_000 + order, None, format!("DROP SEQUENCE {};", cx.q(&name)));
            Op::Delete
        }
        (Some(s), Some(d)) => {
            if sequence_same(cx, &s.detail, &d.detail) {
                Op::Same
            } else {
                o.push(CREATE_BASE, order, None, sequence_alter(cx, &s.detail));
                Op::Modify
            }
        }
        (None, None) => Op::Same,
    };
    ObjectDiff { kind: "sequence", name, op, parts: Vec::new(), source_ddl: src.map(ddl).unwrap_or_default(), target_ddl: dst.map(ddl).unwrap_or_default(), stmts: o.stmts }
}

fn diff_function(cx: &Ctx, order: i64, src: Option<&FuncSnap>, dst: Option<&FuncSnap>) -> ObjectDiff {
    let f = src.or(dst).expect("function");
    let kw = if f.procedure { "PROCEDURE" } else { "FUNCTION" };
    let sig = {
        let (n, rest) = f.key.split_once('(').unwrap_or((&f.key, ")"));
        format!("{}.{}({rest}", quote_ident(&cx.schema), quote_ident(n))
    };
    let mut o = Out { cx, stmts: Vec::new(), parts: Vec::new() };
    let create = |o: &mut Out, f: &FuncSnap| {
        o.push(CREATE_BASE, 1_000_000 + order, None, func_ddl(f));
        if f.comment.is_some() {
            o.push(CREATE_BASE, 1_000_000 + order, None, comment_on(kw, &sig, &f.comment));
        }
    };
    let ddl = |f: &FuncSnap| {
        let mut t = func_ddl(f);
        if f.comment.is_some() {
            t.push_str(&format!("\n\n{}", comment_on(kw, &sig, &f.comment)));
        }
        t
    };
    let op = match (src, dst) {
        (Some(s), None) => {
            create(&mut o, s);
            Op::Add
        }
        (None, Some(_)) => {
            o.push(DROP_OBJECTS, 2_000_000 + order, None, format!("DROP {kw} {sig};"));
            Op::Delete
        }
        (Some(s), Some(d)) => {
            if cx.same(&func_ddl(s), &func_ddl(d)) && s.comment == d.comment {
                Op::Same
            } else {
                create(&mut o, s);
                Op::Modify
            }
        }
        (None, None) => Op::Same,
    };
    ObjectDiff { kind: "function", name: f.key.clone(), op, parts: Vec::new(), source_ddl: src.map(ddl).unwrap_or_default(), target_ddl: dst.map(ddl).unwrap_or_default(), stmts: o.stmts }
}

/// Comandos que igualam um tipo ao da origem: rótulos novos de enum, atributos de tipo composto, padrão, `NOT NULL` e
/// restrições de domínio e o comentário. O que não tem `ALTER` (tipo base do domínio, subtipo da faixa, mudança de
/// espécie) vira um comentário no script.
fn type_changes(cx: &Ctx, q: &str, s: &TypeSnap, d: &TypeSnap) -> Vec<String> {
    let (sd, dd) = (&s.detail, &d.detail);
    let mut out = Vec::new();
    let kw = if sd.kind == "domain" { "DOMAIN" } else { "TYPE" };
    if sd.kind != dd.kind {
        out.push(format!("-- type {q} changed from {} to {} and cannot be altered automatically", dd.kind, sd.kind));
        return out;
    }
    match sd.kind.as_str() {
        "enum" => {
            for l in sd.labels.iter().filter(|l| !dd.labels.contains(l)) {
                out.push(format!("ALTER TYPE {q} ADD VALUE {};", literal(l)));
            }
        }
        "composite" => {
            for a in &sd.attributes {
                match dd.attributes.iter().find(|x| x.name == a.name) {
                    None => out.push(format!("ALTER TYPE {q} ADD ATTRIBUTE {} {};", quote_ident(&a.name), a.data_type)),
                    Some(x) if x.data_type != a.data_type => out.push(format!("ALTER TYPE {q} ALTER ATTRIBUTE {} TYPE {};", quote_ident(&a.name), a.data_type)),
                    Some(_) => {}
                }
            }
            for a in dd.attributes.iter().filter(|a| !sd.attributes.iter().any(|x| x.name == a.name)) {
                out.push(format!("ALTER TYPE {q} DROP ATTRIBUTE {};", quote_ident(&a.name)));
            }
        }
        "domain" => {
            if !cx.same_opt(&sd.base_type, &dd.base_type) {
                out.push(format!("-- domain {q}: the base type differs and cannot be altered automatically"));
            }
            if !cx.same_opt(&sd.default, &dd.default) {
                out.push(match &sd.default {
                    Some(x) => format!("ALTER DOMAIN {q} SET DEFAULT {x};"),
                    None => format!("ALTER DOMAIN {q} DROP DEFAULT;"),
                });
            }
            if sd.not_null != dd.not_null {
                out.push(format!("ALTER DOMAIN {q} {} NOT NULL;", if sd.not_null { "SET" } else { "DROP" }));
            }
            for c in &sd.checks {
                match dd.checks.iter().find(|x| x.name == c.name) {
                    Some(x) if cx.same(&x.definition, &c.definition) => {}
                    found => {
                        if found.is_some() {
                            out.push(format!("ALTER DOMAIN {q} DROP CONSTRAINT {};", quote_ident(&c.name)));
                        }
                        out.push(format!("ALTER DOMAIN {q} ADD CONSTRAINT {} {};", quote_ident(&c.name), c.definition));
                    }
                }
            }
            for c in dd.checks.iter().filter(|c| !sd.checks.iter().any(|x| x.name == c.name)) {
                out.push(format!("ALTER DOMAIN {q} DROP CONSTRAINT {};", quote_ident(&c.name)));
            }
        }
        _ => {
            if !cx.same_opt(&sd.subtype, &dd.subtype) {
                out.push(format!("-- type {q}: the range subtype differs and cannot be altered automatically"));
            }
        }
    }
    if sd.comment != dd.comment {
        out.push(comment_on(kw, q, &sd.comment));
    }
    out
}

fn diff_type(cx: &Ctx, order: i64, src: Option<&TypeSnap>, dst: Option<&TypeSnap>) -> ObjectDiff {
    let name = src.or(dst).map(|t| t.name.clone()).unwrap_or_default();
    let q = cx.q(&name);
    let mut o = Out { cx, stmts: Vec::new(), parts: Vec::new() };
    let op = match (src, dst) {
        (Some(s), None) => {
            o.push(CREATE_BASE, order, None, type_create(cx, s));
            if s.detail.comment.is_some() {
                let kw = if s.detail.kind == "domain" { "DOMAIN" } else { "TYPE" };
                o.push(CREATE_BASE, order, None, comment_on(kw, &q, &s.detail.comment));
            }
            Op::Add
        }
        (None, Some(t)) => {
            let kw = if t.detail.kind == "domain" { "DOMAIN" } else { "TYPE" };
            o.push(DROP_OBJECTS, 3_000_000 + order, None, format!("DROP {kw} {q};"));
            Op::Delete
        }
        (Some(s), Some(d)) => {
            let changes = type_changes(cx, &q, s, d);
            for sql in &changes {
                o.push(CREATE_BASE, order, None, sql.clone());
            }
            if changes.is_empty() { Op::Same } else { Op::Modify }
        }
        (None, None) => Op::Same,
    };
    ObjectDiff { kind: "type", name, op, parts: Vec::new(), source_ddl: src.map(|t| type_ddl(cx, t)).unwrap_or_default(), target_ddl: dst.map(|t| type_ddl(cx, t)).unwrap_or_default(), stmts: o.stmts }
}

/// Junta os dois lados pelo nome, na ordem da origem e depois o que só existe no destino.
fn pair<'a, T>(src: &'a [T], dst: &'a [T], key: impl Fn(&T) -> &str) -> Vec<(Option<&'a T>, Option<&'a T>)> {
    let dst_by: HashMap<&str, &T> = dst.iter().map(|d| (key(d), d)).collect();
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<(Option<&T>, Option<&T>)> = src
        .iter()
        .map(|s| {
            seen.insert(key(s));
            (Some(s), dst_by.get(key(s)).copied())
        })
        .collect();
    out.extend(dst.iter().filter(|d| !seen.contains(key(d))).map(|d| (None, Some(d))));
    out
}

pub async fn compare(src: &PgSession, dst: &PgSession, spec: &StructSpec, mut hooks: CompareHooks<'_>) -> Result<StructComparison, DatabaseError> {
    let mut a = load(src, &spec.source_schema, &mut hooks, "Source", 0.0).await?;
    let b = load(dst, &spec.target_schema, &mut hooks, "Target", 0.5).await?;
    retarget(&mut a, &spec.source_schema, &spec.target_schema);
    (hooks.progress)("Comparing...", 0.98);
    let major = |v: i32| if v >= 100000 { v / 10000 } else { v / 100 };
    let cx = Ctx { schema: spec.target_schema.clone(), target_version: dst.server.version_num, loose: major(src.server.version_num) != major(dst.server.version_num),
        legacy_sequences: src.server.version_num.min(dst.server.version_num) < 100000,
    };
    let mut objects = Vec::new();
    let mut order = 0i64;
    let mut next = || {
        order += 1;
        order
    };
    for (s, d) in pair(&a.types, &b.types, |t| t.name.as_str()) {
        objects.push(diff_type(&cx, next(), s, d));
    }
    for (s, d) in pair(&a.seqs, &b.seqs, |t| t.name.as_str()) {
        objects.push(diff_sequence(&cx, next(), s, d));
    }
    for (s, d) in pair(&a.funcs, &b.funcs, |t| t.key.as_str()) {
        objects.push(diff_function(&cx, next(), s, d));
    }
    for (s, d) in pair(&a.tables, &b.tables, |t| t.name.as_str()) {
        objects.push(diff_table(&cx, next(), s, d));
    }
    // Views e views materializadas dividem o espaço de nomes.
    for (s, d) in pair(&a.views, &b.views, |t| t.name.as_str()) {
        objects.push(diff_view(&cx, next(), s, d));
    }
    Ok(StructComparison { objects })
}

/// O script do que está marcado: fase a fase, e dentro da fase na ordem dos objetos.
pub fn script(c: &StructComparison, selection: &[StructSelection]) -> Vec<String> {
    let mut picked: Vec<&Stmt> = Vec::new();
    for sel in selection {
        let Some(obj) = c.objects.iter().find(|o| o.kind == sel.kind && o.name == sel.name) else { continue };
        picked.extend(obj.stmts.iter().filter(|s| match (s.part, &sel.parts) {
            (Some(p), Some(chosen)) => chosen.contains(&p),
            _ => true,
        }));
    }
    picked.sort_by_key(|s| (s.phase, s.order));
    picked.into_iter().map(|s| s.sql.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str) -> Col {
        Col { name: name.into(), data_type: ty.into(), not_null: false, default: None, identity: String::new(), generated: String::new(), collation: None, comment: None }
    }

    fn table(name: &str, cols: Vec<Col>) -> TableSnap {
        TableSnap {
            name: name.into(),
            unlogged: false,
            options: vec![],
            partition_key: None,
            partition_bound: None,
            is_partition: false,
            parents: vec![],
            owner: "o".into(),
            comment: None,
            columns: cols,
            constraints: vec![],
            indexes: vec![],
            triggers: vec![],
        }
    }

    #[test]
    fn a_new_column_becomes_an_alter_and_the_table_is_modified() {
        let cx = Ctx { schema: "public".into(), target_version: 180000, loose: false, legacy_sequences: false };
        let s = table("t", vec![col("id", "integer"), col("status", "boolean")]);
        let d = table("t", vec![col("id", "integer")]);
        let diff = diff_table(&cx, 1, Some(&s), Some(&d));
        assert_eq!(diff.op, Op::Modify);
        let c = StructComparison { objects: vec![diff] };
        let sel = [StructSelection { kind: "table".into(), name: "t".into(), parts: None }];
        assert_eq!(script(&c, &sel), vec![r#"ALTER TABLE "public"."t" ADD COLUMN "status" boolean;"#.to_string()]);
        // Sem a coluna marcada, nada roda.
        let none = [StructSelection { kind: "table".into(), name: "t".into(), parts: Some(vec![]) }];
        assert!(script(&c, &none).is_empty());
    }

    #[test]
    fn loose_comparison_ignores_qualifiers_spacing_and_parentheses() {
        let old = "SELECT clients.id,
    clients.name
   FROM public.clients;";
        let new = "SELECT id,
    name
   FROM public.clients;";
        assert_eq!(loosen(old), loosen(new));
        assert_ne!(loosen("SELECT a FROM t"), loosen("SELECT b FROM t"));
        assert_eq!(loosen("SELECT 1.5"), "SELECT1.5");
    }

    #[test]
    fn triggers_use_execute_procedure_for_old_targets() {
        let stmt = "CREATE TRIGGER t AFTER INSERT ON s.a FOR EACH ROW EXECUTE FUNCTION s.f();";
        assert!(adapt(stmt, 90619).contains("EXECUTE PROCEDURE s.f()"));
        assert_eq!(adapt(stmt, 180006), stmt);
        assert_eq!(loosen("EXECUTE PROCEDURE s.f()"), loosen("EXECUTE FUNCTION s.f()"));
    }

    #[test]
    fn storage_options_are_set_and_reset() {
        let cx = Ctx { schema: "public".into(), target_version: 180000, loose: false, legacy_sequences: false };
        let mut s = table("t", vec![col("id", "integer")]);
        let mut d = s.clone();
        s.options = vec!["fillfactor=70".into()];
        d.options = vec!["autovacuum_enabled=false".into()];
        s.unlogged = true;
        let diff = diff_table(&cx, 1, Some(&s), Some(&d));
        let c = StructComparison { objects: vec![diff] };
        let sel = [StructSelection { kind: "table".into(), name: "t".into(), parts: None }];
        assert_eq!(
            script(&c, &sel),
            vec![
                r#"ALTER TABLE "public"."t" SET UNLOGGED;"#.to_string(),
                r#"ALTER TABLE "public"."t" SET (fillfactor=70);"#.to_string(),
                r#"ALTER TABLE "public"."t" RESET (autovacuum_enabled);"#.to_string(),
            ]
        );
        assert!(create_table(&cx, &s).contains("WITH (fillfactor=70)"));
    }

    fn type_snap(kind: &str, f: impl FnOnce(&mut TypeDetail)) -> TypeSnap {
        let mut detail = TypeDetail {
            name: "t".into(),
            kind: kind.into(),
            owner: None,
            comment: None,
            labels: vec![],
            attributes: vec![],
            base_type: None,
            default: None,
            not_null: false,
            checks: vec![],
            subtype: None,
        };
        f(&mut detail);
        TypeSnap { name: "t".into(), detail }
    }

    #[test]
    fn composite_types_and_domains_are_altered_in_place() {
        use atlas_core::{TypeAttribute, TypeCheck};
        let cx = Ctx { schema: "public".into(), target_version: 180000, loose: false, legacy_sequences: false };
        let attr = |n: &str, t: &str| TypeAttribute { name: n.into(), data_type: t.into() };
        let s = type_snap("composite", |d| d.attributes = vec![attr("a", "integer"), attr("b", "text"), attr("c", "date")]);
        let d = type_snap("composite", |d| d.attributes = vec![attr("a", "bigint"), attr("b", "text"), attr("old", "text")]);
        let sql = type_changes(&cx, "\"public\".\"t\"", &s, &d);
        assert_eq!(
            sql,
            vec![
                r#"ALTER TYPE "public"."t" ALTER ATTRIBUTE "a" TYPE integer;"#,
                r#"ALTER TYPE "public"."t" ADD ATTRIBUTE "c" date;"#,
                r#"ALTER TYPE "public"."t" DROP ATTRIBUTE "old";"#,
            ]
        );
        let check = |n: &str, def: &str| TypeCheck { name: n.into(), definition: def.into() };
        let s = type_snap("domain", |d| {
            d.base_type = Some("integer".into());
            d.default = Some("1".into());
            d.not_null = true;
            d.checks = vec![check("pos", "CHECK ((VALUE > 0))")];
        });
        let d = type_snap("domain", |d| {
            d.base_type = Some("integer".into());
            d.checks = vec![check("pos", "CHECK ((VALUE >= 0))"), check("gone", "CHECK ((VALUE < 9))")];
        });
        let sql = type_changes(&cx, "\"public\".\"t\"", &s, &d);
        assert_eq!(
            sql,
            vec![
                r#"ALTER DOMAIN "public"."t" SET DEFAULT 1;"#,
                r#"ALTER DOMAIN "public"."t" SET NOT NULL;"#,
                r#"ALTER DOMAIN "public"."t" DROP CONSTRAINT "pos";"#,
                r#"ALTER DOMAIN "public"."t" ADD CONSTRAINT "pos" CHECK ((VALUE > 0));"#,
                r#"ALTER DOMAIN "public"."t" DROP CONSTRAINT "gone";"#,
            ]
        );
        assert!(type_changes(&cx, "q", &s, &s.clone()).is_empty());
    }

    #[test]
    fn identical_tables_are_the_same() {
        let cx = Ctx { schema: "public".into(), target_version: 180000, loose: false, legacy_sequences: false };
        let t = table("t", vec![col("id", "integer")]);
        assert_eq!(diff_table(&cx, 1, Some(&t), Some(&t.clone())).op, Op::Same);
    }

    #[test]
    fn a_changed_constraint_is_dropped_before_the_new_one_is_added() {
        let cx = Ctx { schema: "public".into(), target_version: 180000, loose: false, legacy_sequences: false };
        let mut s = table("t", vec![col("a", "integer")]);
        let mut d = s.clone();
        s.constraints.push(Named { name: "c".into(), kind: "c".into(), def: "CHECK ((a > 1))".into() });
        d.constraints.push(Named { name: "c".into(), kind: "c".into(), def: "CHECK ((a > 0))".into() });
        let diff = diff_table(&cx, 1, Some(&s), Some(&d));
        let c = StructComparison { objects: vec![diff] };
        let sel = [StructSelection { kind: "table".into(), name: "t".into(), parts: None }];
        let sql = script(&c, &sel);
        assert!(sql[0].contains("DROP CONSTRAINT"));
        assert!(sql[1].contains("ADD CONSTRAINT") && sql[1].contains("a > 1"));
    }
}
