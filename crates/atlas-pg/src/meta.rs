//! Leitura de catálogo (`pg_catalog`) com gates por versão do servidor (spec §32).
//! Nomes de schema/tabela sempre como parâmetros, nunca concatenados no SQL.

use crate::connect::PgSession;
use crate::error::map_error;
use atlas_core::{
    ColumnInfo, ConstraintInfo, DatabaseError, DatabaseList, ErrorKind, IndexInfo, ObjectInfo, RoleInfo, RuleInfo,
    SchemaObjects, TableDetail, TriggerInfo, ViewDetail,
};

pub async fn list_databases(s: &PgSession) -> Result<DatabaseList, DatabaseError> {
    let rows = s
        .client
        .query("SELECT datname::text, (datallowconn AND NOT datistemplate) FROM pg_catalog.pg_database ORDER BY datname", &[])
        .await
        .map_err(|e| map_error(&e))?;
    let mut databases = Vec::new();
    let mut hidden = 0;
    for r in rows {
        if r.get::<_, bool>(1) {
            databases.push(r.get(0));
        } else {
            hidden += 1;
        }
    }
    Ok(DatabaseList { databases, hidden })
}

/// Schemas do usuário (sem `pg_*` e `information_schema`, como a referência faz por padrão).
pub async fn list_schemas(s: &PgSession) -> Result<Vec<String>, DatabaseError> {
    let rows = s
        .client
        .query(
            "SELECT nspname::text FROM pg_catalog.pg_namespace \
             WHERE nspname !~ '^pg_' AND nspname <> 'information_schema' ORDER BY nspname",
            &[],
        )
        .await
        .map_err(|e| map_error(&e))?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
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
    let fsql = format!(
        "SELECT p.proname::text, pg_get_function_identity_arguments(p.oid), pg_get_function_result(p.oid), \
                l.lanname::text, pg_get_userbyid(p.proowner)::text, obj_description(p.oid, 'pg_proc') \
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
            ..Default::default()
        })
        .collect();

    Ok(SchemaObjects { tables, views, matviews, functions, sequences })
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
    let csql = format!(
        "SELECT a.attname::text, t.typname::text, format_type(a.atttypid, a.atttypmod), a.atttypmod, a.attnotnull, \
                {pk_expr}, pg_get_expr(d.adbin, d.adrelid), col_description(c.oid, a.attnum), \
                (SELECT cn.nspname::text FROM pg_catalog.pg_collation co JOIN pg_catalog.pg_namespace cn ON cn.oid = co.collnamespace WHERE co.oid = a.attcollation), \
                (SELECT co.collname::text FROM pg_catalog.pg_collation co WHERE co.oid = a.attcollation) \
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

    let tsql = "SELECT t.tgname::text, pg_get_triggerdef(t.oid), t.tgenabled <> 'D' \
                  FROM pg_catalog.pg_trigger t \
                  JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
                  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relname = $2 AND NOT t.tgisinternal \
                 ORDER BY t.tgname";
    let trows = s.client.query(tsql, &[&schema, &table]).await.map_err(|e| map_error(&e))?;
    let triggers = trows.iter().map(|r| TriggerInfo { name: r.get(0), definition: r.get(1), enabled: r.get(2) }).collect();

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
    let row = s
        .client
        .query_opt(
            "SELECT c.oid, pg_get_viewdef(c.oid, true), pg_get_userbyid(c.relowner)::text, \
                    obj_description(c.oid, 'pg_class'), array_to_string(c.reloptions, E'\\n') \
               FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
              WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind = 'v'",
            &[&schema, &view],
        )
        .await
        .map_err(|e| map_error(&e))?;
    let Some(row) = row else {
        let mut err = DatabaseError::new(ErrorKind::Server, format!("view \"{schema}\".\"{view}\" does not exist"));
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

    Ok(ViewDetail {
        definition: definition.trim().trim_end_matches(';').trim_end().to_string(),
        owner: row.get(2),
        comment: row.get(3),
        check_option,
        security_barrier,
        rules,
    })
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
