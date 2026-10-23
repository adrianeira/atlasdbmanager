//! Leitura de catálogo (`pg_catalog`) com gates por versão do servidor (spec §32).
//! Nomes de schema/tabela sempre como parâmetros, nunca concatenados no SQL.

use crate::connect::PgSession;
use crate::error::map_error;
use atlas_core::{
    AvailableExtension, ColumnInfo, ConstraintInfo, DatabaseDetail, DatabaseError, DatabaseList, ErrorKind, ExtensionDetail,
    ExtensionInfo, IndexInfo, ObjectInfo, RoleInfo, RuleInfo,
    FunctionDetail, RoleDetail, SchemaDetail, SchemaInfo, SchemaObjects, SequenceDetail, TableDetail, TypeAttribute, TypeCheck, TypeDetail,
    TriggerInfo, ViewDetail,
};

pub async fn list_databases(s: &PgSession) -> Result<DatabaseList, DatabaseError> {
    let rows = s
        .client
        .query(
            "SELECT datname::text, (datallowconn AND NOT datistemplate), shobj_description(oid, 'pg_database') \
               FROM pg_catalog.pg_database ORDER BY datname",
            &[],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let mut databases = Vec::new();
    let mut comments = std::collections::BTreeMap::new();
    let mut hidden = 0;
    for r in rows {
        if r.get::<_, bool>(1) {
            let name: String = r.get(0);
            if let Some(comment) = r.get::<_, Option<String>>(2).filter(|c| !c.is_empty()) {
                comments.insert(name.clone(), comment);
            }
            databases.push(name);
        } else {
            hidden += 1;
        }
    }
    Ok(DatabaseList { databases, hidden, comments })
}

/// Schemas do usuário (sem `pg_*` e `information_schema`, como a referência faz por padrão), com o comentário de cada um.
pub async fn list_schemas(s: &PgSession) -> Result<Vec<SchemaInfo>, DatabaseError> {
    let rows = s
        .client
        .query(
            "SELECT nspname::text, obj_description(oid, 'pg_namespace') FROM pg_catalog.pg_namespace \
             WHERE nspname !~ '^pg_' AND nspname <> 'information_schema' ORDER BY nspname",
            &[],
        )
        .await
        .map_err(|e| map_error(&e))?;
    Ok(rows
        .iter()
        .map(|r| SchemaInfo { name: r.get(0), comment: r.get::<_, Option<String>>(1).filter(|c| !c.is_empty()) })
        .collect())
}

async fn relations(s: &PgSession, schema: &str, kinds: &str) -> Result<Vec<ObjectInfo>, DatabaseError> {
    // `kinds` é uma constante interna (ex.: "'r','p'"), nunca entrada do usuário.
    let sql = format!(
        "SELECT c.relname::text, CASE WHEN c.reltuples < 0 THEN NULL ELSE c.reltuples::bigint END, \
                obj_description(c.oid, 'pg_class'), pg_get_userbyid(c.relowner)::text \
           FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
          WHERE n.nspname = $1 AND c.relkind IN ({kinds}) ORDER BY c.relname"
    );
    let rows = s.client.query(&sql, &[&schema]).await.map_err(|e| map_error(&e))?;
    Ok(rows
        .iter()
        .map(|r| ObjectInfo { name: r.get(0), rows: r.get(1), comment: r.get(2), owner: r.get(3), ..Default::default() })
        .collect())
}

pub async fn list_objects(s: &PgSession, schema: &str) -> Result<SchemaObjects, DatabaseError> {
    let v = s.server.version_num;
    // Tabelas particionadas (relkind 'p') só existem a partir do PostgreSQL 10.
    let table_kinds = if v >= 100000 { "'r','p'" } else { "'r'" };
    let tables = relations(s, schema, table_kinds).await?;
    let views = relations(s, schema, "'v'").await?;
    let matviews = if v >= 90300 { relations(s, schema, "'m'").await? } else { Vec::new() };
    let mut sequences = relations(s, schema, "'S'").await?;
    for seq in &mut sequences {
        seq.rows = None;
    }

    // pg_proc.prokind existe só a partir do 11; antes, agregações/janelas eram flags.
    let kind_filter = if v >= 110000 { "p.prokind IN ('f','p')" } else { "NOT p.proisagg AND NOT p.proiswindow" };
    let is_procedure = if v >= 110000 { "p.prokind = 'p'" } else { "false" };
    let fsql = format!(
        "SELECT p.proname::text, pg_get_function_identity_arguments(p.oid), pg_get_function_result(p.oid), \
                l.lanname::text, pg_get_userbyid(p.proowner)::text, obj_description(p.oid, 'pg_proc'), {is_procedure} \
           FROM pg_catalog.pg_proc p \
           JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
           JOIN pg_catalog.pg_language l ON l.oid = p.prolang \
          WHERE n.nspname = $1 AND {kind_filter} ORDER BY p.proname, 2"
    );
    let frows = s.client.query(&fsql, &[&schema]).await.map_err(|e| map_error(&e))?;
    let functions = frows
        .iter()
        .map(|r| ObjectInfo {
            name: r.get(0),
            args: r.get(1),
            return_type: r.get(2),
            language: r.get(3),
            owner: r.get(4),
            comment: r.get(5),
            procedure: r.get(6),
            ..Default::default()
        })
        .collect();

    // Tipos do schema: enum (e), domínio (d), faixa (r) e composto avulso (c cuja relação é do tipo 'c', e não de uma tabela).
    let trows = s
        .client
        .query(
            "SELECT t.typname::text, CASE t.typtype WHEN 'e' THEN 'enum' WHEN 'd' THEN 'domain' WHEN 'r' THEN 'range' ELSE 'composite' END, \
                    pg_get_userbyid(t.typowner)::text, obj_description(t.oid, 'pg_type') \
               FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
              WHERE n.nspname = $1 AND (t.typtype IN ('e', 'd', 'r') \
                    OR (t.typtype = 'c' AND EXISTS (SELECT 1 FROM pg_catalog.pg_class c WHERE c.oid = t.typrelid AND c.relkind = 'c'))) \
              ORDER BY t.typname",
            &[&schema],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let types = trows
        .iter()
        .map(|r| ObjectInfo { name: r.get(0), data_type: Some(r.get(1)), owner: r.get(2), comment: r.get(3), ..Default::default() })
        .collect();

    // Extensões são do banco inteiro: a mesma lista aparece em qualquer schema, com o schema de cada uma.
    let erows = s
        .client
        .query(
            "SELECT e.extname::text, e.extversion::text, n.nspname::text, pg_get_userbyid(e.extowner)::text, \
                    obj_description(e.oid, 'pg_extension') \
               FROM pg_catalog.pg_extension e JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace \
              ORDER BY e.extname",
            &[],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let extensions = erows
        .iter()
        .map(|r| ExtensionInfo { name: r.get(0), version: r.get(1), schema: r.get(2), owner: r.get(3), comment: r.get(4) })
        .collect();

    Ok(SchemaObjects { tables, views, matviews, functions, types, sequences, extensions })
}

/// Extensões que o servidor oferece e ainda não foram instaladas neste banco (para o "Nova extensão").
pub async fn available_extensions(s: &PgSession) -> Result<Vec<AvailableExtension>, DatabaseError> {
    let rows = s
        .client
        .query(
            "SELECT a.name::text, a.default_version::text, a.comment, v.schema::text \
               FROM pg_catalog.pg_available_extensions a \
               LEFT JOIN pg_catalog.pg_available_extension_versions v ON v.name = a.name AND v.version = a.default_version \
              WHERE a.installed_version IS NULL ORDER BY a.name",
            &[],
        )
        .await
        .map_err(|e| map_error(&e))?;
    Ok(rows
        .iter()
        .map(|r| AvailableExtension { name: r.get(0), default_version: r.get(1), comment: r.get(2), fixed_schema: r.get(3) })
        .collect())
}

/// Uma extensão instalada, com as versões para as quais ela pode ser atualizada.
pub async fn extension_detail(s: &PgSession, name: &str) -> Result<ExtensionDetail, DatabaseError> {
    let row = s
        .client
        .query_opt(
            "SELECT e.extversion::text, n.nspname::text, pg_get_userbyid(e.extowner)::text, e.extrelocatable, \
                    obj_description(e.oid, 'pg_extension') \
               FROM pg_catalog.pg_extension e JOIN pg_catalog.pg_namespace n ON n.oid = e.extnamespace \
              WHERE e.extname = $1",
            &[&name],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let Some(r) = row else {
        let mut err = DatabaseError::new(ErrorKind::Server, format!("extension \"{name}\" does not exist"));
        err.code = Some("42704".into());
        return Err(err);
    };
    let version: String = r.get(0);
    let paths = s
        .client
        .query(
            "SELECT target::text FROM pg_catalog.pg_extension_update_paths($1::name) \
              WHERE source = $2 AND path IS NOT NULL AND target <> $2 ORDER BY target",
            &[&name, &version],
        )
        .await
        .map_err(|e| map_error(&e))?;
    Ok(ExtensionDetail {
        name: name.to_string(),
        version,
        schema: r.get(1),
        owner: r.get(2),
        relocatable: r.get(3),
        comment: r.get(4),
        update_versions: paths.iter().map(|p| p.get(0)).collect(),
    })
}

/// Tamanho/decimais no formato da grade de campos (deriva de `atttypmod`).
fn length_decimal(type_name: &str, typmod: i32) -> (i32, i32) {
    match type_name {
        "int2" => (16, 0),
        "int4" => (32, 0),
        "int8" => (64, 0),
        "varchar" | "bpchar" | "char" if typmod >= 4 => (typmod - 4, 0),
        "numeric" if typmod >= 4 => (((typmod - 4) >> 16) & 0xffff, (typmod - 4) & 0xffff),
        "timestamp" | "timestamptz" | "time" | "timetz" | "interval" => (if typmod >= 0 { typmod & 0xffff } else { 6 }, 0),
        "bit" | "varbit" if typmod > 0 => (typmod, 0),
        _ => (0, 0),
    }
}

fn fk_action(c: Option<String>) -> Option<String> {
    c.map(|a| {
        match a.as_str() {
            "r" => "RESTRICT",
            "c" => "CASCADE",
            "n" => "SET NULL",
            "d" => "SET DEFAULT",
            _ => "NO ACTION",
        }
        .to_string()
    })
}

pub async fn table_detail(s: &PgSession, schema: &str, table: &str) -> Result<TableDetail, DatabaseError> {
    let v = s.server.version_num;
    // array_position existe a partir do 9.5; antes a posição na PK fica desconhecida.
    let pk_expr = if v >= 90500 {
        "(SELECT array_position(con.conkey, a.attnum) FROM pg_catalog.pg_constraint con \
           WHERE con.conrelid = c.oid AND con.contype = 'p')"
    } else {
        "NULL::int"
    };
    // attidentity/attgenerated existem desde PostgreSQL 10/12. Em versões anteriores viriam vazios
    // ou a coluna nem existe — fazemos NULLIF('') para normalizar e usamos NULL::text se precisar.
    let identity_expr = if v >= 100000 { "NULLIF(a.attidentity::text, '')" } else { "NULL::text" };
    let generated_expr = if v >= 120000 { "NULLIF(a.attgenerated::text, '')" } else { "NULL::text" };
    let csql = format!(
        "SELECT a.attname::text, t.typname::text, format_type(a.atttypid, a.atttypmod), a.atttypmod, a.attnotnull, \
                {pk_expr}, pg_get_expr(d.adbin, d.adrelid), col_description(c.oid, a.attnum), \
                (SELECT cn.nspname::text FROM pg_catalog.pg_collation co JOIN pg_catalog.pg_namespace cn ON cn.oid = co.collnamespace WHERE co.oid = a.attcollation), \
                (SELECT co.collname::text FROM pg_catalog.pg_collation co WHERE co.oid = a.attcollation), \
                {identity_expr}, {generated_expr} \
           FROM pg_catalog.pg_attribute a \
           JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
           JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
           JOIN pg_catalog.pg_type t ON t.oid = a.atttypid \
           LEFT JOIN pg_catalog.pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
          WHERE n.nspname = $1 AND c.relname = $2 AND a.attnum > 0 AND NOT a.attisdropped \
          ORDER BY a.attnum"
    );
    let rows = s.client.query(&csql, &[&schema, &table]).await.map_err(|e| map_error(&e))?;
    let columns = rows
        .iter()
        .map(|r| {
            let type_name: String = r.get(1);
            let typmod: i32 = r.get(3);
            let (length, decimal) = length_decimal(&type_name, typmod);
            let coll_ns: Option<String> = r.get(8);
            let coll: Option<String> = r.get(9);
            ColumnInfo {
                name: r.get(0),
                data_type: type_name,
                formatted_type: r.get(2),
                length,
                decimal,
                not_null: r.get(4),
                key: r.get(5),
                default: r.get(6),
                comment: r.get(7),
                collation: match (coll_ns, coll) {
                    (Some(n), Some(c)) => Some(format!("\"{n}\".\"{c}\"")),
                    _ => None,
                },
                identity: r.get(10),
                generated: r.get(11),
            }
        })
        .collect();

    let isql = "SELECT i.relname::text, pg_get_indexdef(ix.indexrelid), am.amname::text, ix.indisunique, ix.indisprimary, \
                       obj_description(i.oid, 'pg_class'), \
                       array_to_string(ARRAY(SELECT pg_get_indexdef(ix.indexrelid, k + 1, true) \
                                               FROM generate_subscripts(ix.indkey, 1) k ORDER BY k), ', ') \
                  FROM pg_catalog.pg_index ix \
                  JOIN pg_catalog.pg_class i ON i.oid = ix.indexrelid \
                  JOIN pg_catalog.pg_class c ON c.oid = ix.indrelid \
                  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  JOIN pg_catalog.pg_am am ON am.oid = i.relam \
                 WHERE n.nspname = $1 AND c.relname = $2 \
                   AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_constraint con WHERE con.conindid = ix.indexrelid AND con.contype IN ('p','u','x')) \
                 ORDER BY i.relname";
    let irows = s.client.query(isql, &[&schema, &table]).await.map_err(|e| map_error(&e))?;
    let indexes = irows
        .iter()
        .map(|r| IndexInfo {
            name: r.get(0),
            definition: r.get(1),
            method: r.get(2),
            unique: r.get(3),
            primary: r.get(4),
            comment: r.get(5),
            fields: r.get(6),
        })
        .collect();

    let ksql = "SELECT con.conname::text, con.contype::text, pg_get_constraintdef(con.oid), \
                       (SELECT string_agg(a.attname::text, ', ' ORDER BY k.ord) FROM unnest(con.conkey) WITH ORDINALITY k(attnum, ord) \
                          JOIN pg_catalog.pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum), \
                       rn.nspname::text, rc.relname::text, \
                       (SELECT string_agg(a.attname::text, ', ' ORDER BY k.ord) FROM unnest(con.confkey) WITH ORDINALITY k(attnum, ord) \
                          JOIN pg_catalog.pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.attnum), \
                       CASE WHEN con.contype = 'f' THEN con.confdeltype::text END, \
                       CASE WHEN con.contype = 'f' THEN con.confupdtype::text END, \
                       obj_description(con.oid, 'pg_constraint') \
                  FROM pg_catalog.pg_constraint con \
                  JOIN pg_catalog.pg_class c ON c.oid = con.conrelid \
                  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  LEFT JOIN pg_catalog.pg_class rc ON rc.oid = con.confrelid \
                  LEFT JOIN pg_catalog.pg_namespace rn ON rn.oid = rc.relnamespace \
                 WHERE n.nspname = $1 AND c.relname = $2 AND con.contype IN ('f','u','c','x') \
                 ORDER BY con.contype, con.conname";
    let krows = s.client.query(ksql, &[&schema, &table]).await.map_err(|e| map_error(&e))?;
    let constraints = krows
        .iter()
        .map(|r| ConstraintInfo {
            name: r.get(0),
            kind: r.get(1),
            definition: r.get(2),
            fields: r.get::<_, Option<String>>(3).unwrap_or_default(),
            ref_schema: r.get(4),
            ref_table: r.get(5),
            ref_fields: r.get(6),
            on_delete: fk_action(r.get(7)),
            on_update: fk_action(r.get(8)),
            comment: r.get(9),
        })
        .collect();

    // Tabelas de transição (`REFERENCING`) existem a partir do PostgreSQL 10.
    let transition = if s.server.version_num >= 100000 { "(t.tgoldtable IS NOT NULL OR t.tgnewtable IS NOT NULL)" } else { "false" };
    let tsql = format!(
        "SELECT t.tgname::text, pg_get_triggerdef(t.oid), t.tgenabled <> 'D', t.tgtype::int4, \
                (SELECT quote_ident(pn.nspname) || '.' || quote_ident(p.proname) FROM pg_catalog.pg_proc p \
                   JOIN pg_catalog.pg_namespace pn ON pn.oid = p.pronamespace WHERE p.oid = t.tgfoid), \
                t.tgnargs::int4, t.tgargs, \
                ARRAY(SELECT a.attname::text FROM unnest(t.tgattr::int2[]) WITH ORDINALITY u(num, ord) \
                        JOIN pg_catalog.pg_attribute a ON a.attrelid = t.tgrelid AND a.attnum = u.num ORDER BY u.ord), \
                obj_description(t.oid, 'pg_trigger'), (t.tgconstraint <> 0 OR {transition}) \
           FROM pg_catalog.pg_trigger t \
           JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
           JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
          WHERE n.nspname = $1 AND c.relname = $2 AND NOT t.tgisinternal \
          ORDER BY t.tgname"
    );
    let trows = s.client.query(&tsql, &[&schema, &table]).await.map_err(|e| map_error(&e))?;
    let triggers = trows.iter().map(trigger_info).collect();

    let orow = s
        .client
        .query_opt(
            "SELECT pg_get_userbyid(c.relowner)::text, obj_description(c.oid, 'pg_class') \
               FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
              WHERE n.nspname = $1 AND c.relname = $2",
            &[&schema, &table],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let (owner, comment) = orow.map(|r| (r.get(0), r.get(1))).unwrap_or((None, None));

    Ok(TableDetail { columns, indexes, constraints, triggers, owner, comment })
}

/// Definição da view para o designer. Erro 42P01 (como o servidor) se não existir ou não for view.
pub async fn view_detail(s: &PgSession, schema: &str, view: &str) -> Result<ViewDetail, DatabaseError> {
    relation_detail(s, schema, view, false).await
}

/// Definição de uma view materializada (PostgreSQL 9.3+), com o estado "preenchida".
pub async fn matview_detail(s: &PgSession, schema: &str, view: &str) -> Result<ViewDetail, DatabaseError> {
    relation_detail(s, schema, view, true).await
}

async fn relation_detail(s: &PgSession, schema: &str, view: &str, materialized: bool) -> Result<ViewDetail, DatabaseError> {
    let kind = if materialized { "m" } else { "v" };
    let row = s
        .client
        .query_opt(
            "SELECT c.oid, pg_get_viewdef(c.oid, true), pg_get_userbyid(c.relowner)::text, \
                    obj_description(c.oid, 'pg_class'), array_to_string(c.reloptions, E'\\n') \
               FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
              WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind = $3::text::\"char\"",
            &[&schema, &view, &kind],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let Some(row) = row else {
        let mut err = DatabaseError::new(ErrorKind::Server, format!("{} \"{schema}\".\"{view}\" does not exist", if materialized { "materialized view" } else { "view" }));
        err.code = Some("42P01".into());
        return Err(err);
    };
    let oid: u32 = row.get(0);
    let definition: String = row.get(1);
    let options: Option<String> = row.get(4);
    let mut check_option = None;
    let mut security_barrier = false;
    for opt in options.as_deref().unwrap_or("").lines() {
        match opt.split_once('=') {
            Some(("check_option", v)) => check_option = Some(v.to_string()),
            Some(("security_barrier", v)) => security_barrier = matches!(v, "true" | "on" | "1" | "yes"),
            _ => {}
        }
    }

    let rrows = s
        .client
        .query(
            "SELECT rulename::text, pg_get_ruledef(oid, true) FROM pg_catalog.pg_rewrite \
              WHERE ev_class = $1 AND rulename <> '_RETURN' ORDER BY rulename",
            &[&oid],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let rules = rrows.iter().map(|r| RuleInfo { name: r.get(0), definition: r.get(1) }).collect();
    // `relispopulated` só existe a partir do 9.3 (o mesmo que introduziu as views materializadas).
    let populated = if materialized {
        let r = s
            .client
            .query_one("SELECT relispopulated FROM pg_catalog.pg_class WHERE oid = $1", &[&oid])
            .await
            .map_err(|e| map_error(&e))?;
        r.get(0)
    } else {
        true
    };

    Ok(ViewDetail {
        definition: definition.trim().trim_end_matches(';').trim_end().to_string(),
        owner: row.get(2),
        comment: row.get(3),
        check_option,
        security_barrier,
        rules,
        populated,
    })
}

/// Detalhe de um tipo definido pelo usuário (enum, composto, domínio ou faixa).
pub async fn type_detail(s: &PgSession, schema: &str, name: &str) -> Result<TypeDetail, DatabaseError> {
    let head = s
        .client
        .query_opt(
            "SELECT t.oid, t.typtype::text, t.typrelid, pg_get_userbyid(t.typowner)::text, obj_description(t.oid, 'pg_type'), \
                    CASE WHEN t.typtype = 'd' THEN format_type(t.typbasetype, t.typtypmod) END, t.typdefault, t.typnotnull \
               FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace \
              WHERE n.nspname = $1 AND t.typname = $2 AND (t.typtype IN ('e', 'd', 'r') \
                    OR (t.typtype = 'c' AND EXISTS (SELECT 1 FROM pg_catalog.pg_class c WHERE c.oid = t.typrelid AND c.relkind = 'c')))",
            &[&schema, &name],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let Some(head) = head else {
        let mut err = DatabaseError::new(ErrorKind::Server, format!("type \"{schema}\".\"{name}\" does not exist"));
        err.code = Some("42704".into());
        return Err(err);
    };
    let oid: u32 = head.get(0);
    let kind = match head.get::<_, String>(1).as_str() {
        "e" => "enum",
        "d" => "domain",
        "r" => "range",
        _ => "composite",
    };
    let relid: u32 = head.get(2);

    let mut detail = TypeDetail {
        name: name.to_string(),
        kind: kind.to_string(),
        owner: head.get(3),
        comment: head.get(4),
        labels: Vec::new(),
        attributes: Vec::new(),
        base_type: head.get(5),
        default: head.get(6),
        not_null: head.get(7),
        checks: Vec::new(),
        subtype: None,
    };
    match kind {
        "enum" => {
            let rows = s
                .client
                .query("SELECT enumlabel::text FROM pg_catalog.pg_enum WHERE enumtypid = $1 ORDER BY enumsortorder", &[&oid])
                .await
                .map_err(|e| map_error(&e))?;
            detail.labels = rows.iter().map(|r| r.get(0)).collect();
        }
        "composite" => {
            let rows = s
                .client
                .query(
                    "SELECT attname::text, format_type(atttypid, atttypmod) FROM pg_catalog.pg_attribute \
                      WHERE attrelid = $1 AND attnum > 0 AND NOT attisdropped ORDER BY attnum",
                    &[&relid],
                )
                .await
                .map_err(|e| map_error(&e))?;
            detail.attributes = rows.iter().map(|r| TypeAttribute { name: r.get(0), data_type: r.get(1) }).collect();
        }
        "domain" => {
            let rows = s
                .client
                .query("SELECT conname::text, pg_get_constraintdef(oid) FROM pg_catalog.pg_constraint WHERE contypid = $1 AND contype = 'c' ORDER BY conname", &[&oid])
                .await
                .map_err(|e| map_error(&e))?;
            detail.checks = rows.iter().map(|r| TypeCheck { name: r.get(0), definition: r.get(1) }).collect();
        }
        _ => {
            let r = s
                .client
                .query_opt("SELECT format_type(rngsubtype, NULL) FROM pg_catalog.pg_range WHERE rngtypid = $1", &[&oid])
                .await
                .map_err(|e| map_error(&e))?;
            detail.subtype = r.map(|r| r.get(0));
        }
    }
    Ok(detail)
}

/// Parâmetros e posição atual de uma sequência. No PostgreSQL 10+ vêm de `pg_sequences`; antes, da própria
/// relação da sequência (que tinha uma linha com os parâmetros).
pub async fn sequence_detail(s: &PgSession, schema: &str, name: &str) -> Result<SequenceDetail, DatabaseError> {
    let head = s
        .client
        .query_opt(
            "SELECT c.oid, pg_get_userbyid(c.relowner)::text, obj_description(c.oid, 'pg_class'), \
                    quote_ident(n.nspname) || '.' || quote_ident(c.relname) \
               FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
              WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind = 'S'",
            &[&schema, &name],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let Some(head) = head else {
        let mut err = DatabaseError::new(ErrorKind::Server, format!("sequence \"{schema}\".\"{name}\" does not exist"));
        err.code = Some("42P01".into());
        return Err(err);
    };
    let oid: u32 = head.get(0);
    let qualified: String = head.get(3);

    let owned = s
        .client
        .query_opt(
            "SELECT quote_ident(tn.nspname) || '.' || quote_ident(t.relname) || '.' || quote_ident(a.attname) \
               FROM pg_catalog.pg_depend d \
               JOIN pg_catalog.pg_class t ON t.oid = d.refobjid \
               JOIN pg_catalog.pg_namespace tn ON tn.oid = t.relnamespace \
               JOIN pg_catalog.pg_attribute a ON a.attrelid = d.refobjid AND a.attnum = d.refobjsubid \
              WHERE d.classid = 'pg_catalog.pg_class'::regclass AND d.objid = $1 AND d.deptype = 'a' AND d.refobjsubid > 0",
            &[&oid],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let owned_by: Option<String> = owned.map(|r| r.get(0));

    let text = |v: i64| v.to_string();
    let (data_type, start, increment, min, max, cache, cycle, last_value, is_called);
    if s.server.version_num >= 100000 {
        let r = s
            .client
            .query_one(
                "SELECT data_type::text, start_value, increment_by, min_value, max_value, cache_size, cycle, last_value \
                   FROM pg_catalog.pg_sequences WHERE schemaname = $1 AND sequencename = $2",
                &[&schema, &name],
            )
            .await
            .map_err(|e| map_error(&e))?;
        data_type = r.get::<_, String>(0);
        start = text(r.get(1));
        increment = text(r.get(2));
        min = text(r.get(3));
        max = text(r.get(4));
        cache = text(r.get(5));
        cycle = r.get::<_, bool>(6);
        // `last_value` é NULL enquanto a sequência nunca foi usada (a posição vem da relação, abaixo).
        let used: Option<i64> = r.get(7);
        let pos = s
            .client
            .query_one(&format!("SELECT last_value, is_called FROM {qualified}"), &[])
            .await
            .map_err(|e| map_error(&e))?;
        last_value = text(used.unwrap_or_else(|| pos.get::<_, i64>(0)));
        is_called = pos.get::<_, bool>(1);
    } else {
        let r = s
            .client
            .query_one(
                &format!("SELECT start_value, increment_by, min_value, max_value, cache_value, is_cycled, last_value, is_called FROM {qualified}"),
                &[],
            )
            .await
            .map_err(|e| map_error(&e))?;
        data_type = "bigint".to_string();
        start = text(r.get(0));
        increment = text(r.get(1));
        min = text(r.get(2));
        max = text(r.get(3));
        cache = text(r.get(4));
        cycle = r.get::<_, bool>(5);
        last_value = text(r.get(6));
        is_called = r.get::<_, bool>(7);
    }
    Ok(SequenceDetail {
        name: name.to_string(),
        owner: head.get(1),
        comment: head.get(2),
        data_type,
        start,
        increment,
        min,
        max,
        cache,
        cycle,
        last_value,
        is_called,
        owned_by,
    })
}

/// Função/procedimento identificado por schema, nome e argumentos de identidade (`pg_get_function_identity_arguments`).
pub async fn function_detail(s: &PgSession, schema: &str, name: &str, identity: &str) -> Result<FunctionDetail, DatabaseError> {
    // `prokind` (procedimentos) existe a partir do PostgreSQL 11.
    let procedure = if s.server.version_num >= 110000 { "p.prokind = 'p'" } else { "false" };
    let sql = format!(
        "SELECT p.proname::text, l.lanname::text, p.prosrc, pg_get_function_result(p.oid), \
                pg_get_function_arguments(p.oid), COALESCE(p.proargnames, '{{}}'::text[]), \
                pg_get_function_identity_arguments(p.oid), p.provolatile::text, p.prosecdef, p.proisstrict, \
                pg_get_userbyid(p.proowner)::text, obj_description(p.oid, 'pg_proc'), {procedure} \
           FROM pg_catalog.pg_proc p \
           JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
           JOIN pg_catalog.pg_language l ON l.oid = p.prolang \
          WHERE n.nspname = $1 AND p.proname = $2 AND pg_get_function_identity_arguments(p.oid) = $3"
    );
    let row = s.client.query_opt(&sql, &[&schema, &name, &identity]).await.map_err(|e| map_error(&e))?;
    let Some(r) = row else {
        let mut err = DatabaseError::new(ErrorKind::Server, format!("function \"{schema}\".\"{name}\"({identity}) does not exist"));
        err.code = Some("42883".into());
        return Err(err);
    };
    let volatility = match r.get::<_, String>(7).as_str() {
        "i" => "immutable",
        "s" => "stable",
        _ => "volatile",
    };
    let is_procedure: bool = r.get(12);
    Ok(FunctionDetail {
        name: r.get(0),
        language: r.get(1),
        body: r.get(2),
        returns: if is_procedure { None } else { r.get(3) },
        arguments: r.get(4),
        arg_names: r.get(5),
        identity: r.get(6),
        volatility: volatility.to_string(),
        security_definer: r.get(8),
        strict: r.get(9),
        owner: r.get(10),
        comment: r.get(11),
        procedure: is_procedure,
    })
}

/// Atributos e participações de um papel.
/// Atributos de um banco do servidor. `pg_database` vale para o cluster inteiro, então a leitura não
/// exige abrir uma sessão naquele banco.
pub async fn database_detail(s: &PgSession, name: &str) -> Result<DatabaseDetail, DatabaseError> {
    let row = s
        .client
        .query_opt(
            "SELECT pg_get_userbyid(d.datdba)::text, pg_encoding_to_char(d.encoding)::text, d.datcollate::text, d.datctype::text, \
                    t.spcname::text, d.datconnlimit, d.datallowconn, d.datistemplate, shobj_description(d.oid, 'pg_database') \
               FROM pg_catalog.pg_database d JOIN pg_catalog.pg_tablespace t ON t.oid = d.dattablespace \
              WHERE d.datname = $1",
            &[&name],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let Some(r) = row else {
        let mut err = DatabaseError::new(ErrorKind::Server, format!("database \"{name}\" does not exist"));
        err.code = Some("3D000".into());
        return Err(err);
    };
    Ok(DatabaseDetail {
        name: name.to_string(),
        owner: r.get(0),
        encoding: r.get(1),
        collate: r.get(2),
        ctype: r.get(3),
        tablespace: r.get(4),
        connection_limit: r.get(5),
        allow_connections: r.get(6),
        is_template: r.get(7),
        comment: r.get(8),
    })
}

/// Dono e comentário de um schema do banco da sessão.
pub async fn schema_detail(s: &PgSession, name: &str) -> Result<SchemaDetail, DatabaseError> {
    let row = s
        .client
        .query_opt(
            "SELECT pg_get_userbyid(n.nspowner)::text, obj_description(n.oid, 'pg_namespace') \
               FROM pg_catalog.pg_namespace n WHERE n.nspname = $1",
            &[&name],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let Some(r) = row else {
        let mut err = DatabaseError::new(ErrorKind::Server, format!("schema \"{name}\" does not exist"));
        err.code = Some("3F000".into());
        return Err(err);
    };
    Ok(SchemaDetail { name: name.to_string(), owner: r.get(0), comment: r.get(1) })
}

pub async fn role_detail(s: &PgSession, name: &str) -> Result<RoleDetail, DatabaseError> {
    // `rolbypassrls` existe a partir do PostgreSQL 9.5.
    let bypass = if s.server.version_num >= 90500 { "rolbypassrls" } else { "NULL::boolean" };
    let sql = format!(
        "SELECT oid, rolsuper, rolinherit, rolcreaterole, rolcreatedb, rolcanlogin, rolreplication, {bypass}, rolconnlimit, \
                rolvaliduntil::text, shobj_description(oid, 'pg_authid') \
           FROM pg_catalog.pg_roles WHERE rolname = $1"
    );
    let row = s.client.query_opt(&sql, &[&name]).await.map_err(|e| map_error(&e))?;
    let Some(r) = row else {
        let mut err = DatabaseError::new(ErrorKind::Server, format!("role \"{name}\" does not exist"));
        err.code = Some("42704".into());
        return Err(err);
    };
    let oid: u32 = r.get(0);
    let member_of = s
        .client
        .query(
            "SELECT g.rolname::text FROM pg_catalog.pg_auth_members m JOIN pg_catalog.pg_roles g ON g.oid = m.roleid WHERE m.member = $1 ORDER BY 1",
            &[&oid],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let members = s
        .client
        .query(
            "SELECT g.rolname::text FROM pg_catalog.pg_auth_members m JOIN pg_catalog.pg_roles g ON g.oid = m.member WHERE m.roleid = $1 ORDER BY 1",
            &[&oid],
        )
        .await
        .map_err(|e| map_error(&e))?;
    Ok(RoleDetail {
        name: name.to_string(),
        superuser: r.get(1),
        inherit: r.get(2),
        create_role: r.get(3),
        create_db: r.get(4),
        can_login: r.get(5),
        replication: r.get(6),
        bypass_rls: r.get(7),
        connection_limit: r.get(8),
        valid_until: r.get(9),
        comment: r.get(10),
        member_of: member_of.iter().map(|x| x.get(0)).collect(),
        members: members.iter().map(|x| x.get(0)).collect(),
    })
}

/// Monta o gatilho a partir das colunas de `pg_trigger` (`tgtype`: 1 linha, 2 BEFORE, 4 INSERT, 8 DELETE, 16 UPDATE,
/// 32 TRUNCATE, 64 INSTEAD OF) e dos argumentos (`tgargs`: textos separados por byte nulo).
/// Condição do `WHEN (...)` de um `pg_get_triggerdef`, sem os parênteses externos.
fn when_clause(definition: &str) -> Option<String> {
    let start = definition.find(" WHEN (")? + " WHEN (".len();
    let end = start + definition[start..].find(") EXECUTE ")?;
    Some(definition[start..end].to_string())
}

fn trigger_info(r: &tokio_postgres::Row) -> TriggerInfo {
    let definition: String = r.get(1);
    let tgtype: i32 = r.get(3);
    let nargs: i32 = r.get(5);
    let raw: Vec<u8> = r.get(6);
    let args = raw.split(|b| *b == 0).take(nargs.max(0) as usize).map(|a| String::from_utf8_lossy(a).into_owned()).collect();
    let events = [(4, "INSERT"), (16, "UPDATE"), (8, "DELETE"), (32, "TRUNCATE")]
        .iter()
        .filter(|(bit, _)| tgtype & bit != 0)
        .map(|(_, name)| (*name).to_string())
        .collect();
    let when = when_clause(&definition);
    TriggerInfo {
        name: r.get(0),
        definition,
        enabled: r.get(2),
        timing: if tgtype & 64 != 0 { "INSTEAD OF" } else if tgtype & 2 != 0 { "BEFORE" } else { "AFTER" }.to_string(),
        events,
        for_each_row: tgtype & 1 != 0,
        when,
        function: r.get::<_, Option<String>>(4).unwrap_or_default(),
        args,
        update_columns: r.get(7),
        comment: r.get(8),
        read_only: r.get(9),
    }
}

pub async fn list_roles(s: &PgSession) -> Result<Vec<RoleInfo>, DatabaseError> {
    let rows = s
        .client
        .query("SELECT rolname::text, rolcanlogin, rolsuper FROM pg_catalog.pg_roles WHERE rolname !~ '^pg_' ORDER BY rolname", &[])
        .await
        .map_err(|e| map_error(&e))?;
    Ok(rows.iter().map(|r| RoleInfo { name: r.get(0), can_login: r.get(1), superuser: r.get(2) }).collect())
}

#[cfg(test)]
mod tests {
    use super::length_decimal;

    #[test]
    fn typmod_decoding() {
        assert_eq!(length_decimal("varchar", 154), (150, 0));
        // numeric(19,4): typmod = ((19 << 16) | 4) + 4
        assert_eq!(length_decimal("numeric", ((19 << 16) | 4) + 4), (19, 4));
        assert_eq!(length_decimal("numeric", -1), (0, 0));
        assert_eq!(length_decimal("timestamptz", -1), (6, 0));
        assert_eq!(length_decimal("timestamptz", 3), (3, 0));
        assert_eq!(length_decimal("int4", -1), (32, 0));
    }
}
