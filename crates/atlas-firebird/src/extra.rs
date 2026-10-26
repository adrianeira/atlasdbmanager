//! Data Profiling, Table Profile, diagrama ER e rótulos de chave estrangeira do Firebird, no mesmo formato do PostgreSQL.

use super::{Conn, execute_on, quote_ident, select_sql};
use atlas_core::{DatabaseError, DiagramColumn, DiagramRelation, DiagramTable, OrderBy, QueryOutcome, SchemaDiagram, TableFilter, TableProfile};
use rsfbclient::SqlType;
use serde::Serialize;

/// Quantos valores mais frequentes cada coluna devolve (para o gráfico de distribuição).
const TOP_VALUES: usize = 100;
/// Máximo de chaves por chamada de rótulos (a grade mostra uma página por vez).
const MAX_FK_KEYS: usize = 5_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueCount {
    /// `None` = NULL.
    pub value: Option<String>,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnProfile {
    pub name: String,
    pub rows: u64,
    pub nulls: u64,
    pub distinct: Option<u64>,
    pub unique: Option<u64>,
    pub min: Option<String>,
    pub max: Option<String>,
    pub top: Vec<ValueCount>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataProfile {
    pub rows: u64,
    pub columns: Vec<ColumnProfile>,
}

fn cell(out: &QueryOutcome, row: usize, col: usize) -> Option<String> {
    out.statements.first()?.result.as_ref()?.rows.get(row)?.get(col)?.clone()
}

fn num(out: &QueryOutcome, col: usize) -> Option<u64> {
    cell(out, 0, col)?.parse().ok()
}

/// Perfil das `columns` dos registros que o filtro, a ordenação e o limite escolhem (sem limite = todos).
/// Cada medida é uma consulta própria: uma que falhar (`DISTINCT` num blob, por exemplo) só deixa aquela medida vazia.
pub(crate) fn data_profile_on(
    conn: &mut Conn,
    table: &str,
    columns: &[String],
    order_by: &[OrderBy],
    filter: Option<&TableFilter>,
    limit: Option<u64>,
) -> Result<DataProfile, DatabaseError> {
    // O Firebird só aceita `ORDER BY` numa tabela derivada quando há `FIRST`: sem limite a ordem não muda nenhuma estatística.
    let source = match limit {
        Some(n) => select_sql(table, order_by, filter).replacen("SELECT *", &format!("SELECT FIRST {n} *"), 1),
        None => select_sql(table, &[], filter),
    };
    let mut out = Vec::with_capacity(columns.len());
    let mut total = 0;
    for name in columns {
        let q = quote_ident(name);
        let counts = execute_on(conn, &format!("SELECT COUNT(*), COUNT({q}) FROM ({source}) SRC"), 1)?;
        let rows = num(&counts, 0).unwrap_or(0);
        let not_null = num(&counts, 1).unwrap_or(0);
        total = rows;

        let distinct_sql = format!(
            "SELECT COUNT(DISTINCT {q}), (SELECT COUNT(*) FROM (SELECT {q} FROM ({source}) S2 WHERE {q} IS NOT NULL GROUP BY {q} HAVING COUNT(*) = 1) U) FROM ({source}) SRC"
        );
        let (distinct, unique) = match execute_on(conn, &distinct_sql, 1) {
            Ok(o) => (num(&o, 0), num(&o, 1)),
            Err(_) => (None, None),
        };
        let (min, max) = match execute_on(conn, &format!("SELECT MIN({q}), MAX({q}) FROM ({source}) SRC"), 1) {
            Ok(o) => (cell(&o, 0, 0), cell(&o, 0, 1)),
            Err(_) => (None, None),
        };
        let top_sql = format!("SELECT FIRST {TOP_VALUES} {q}, COUNT(*) FROM ({source}) SRC GROUP BY {q} ORDER BY 2 DESC, 1");
        let top = match execute_on(conn, &top_sql, TOP_VALUES) {
            Ok(o) => o
                .statements
                .first()
                .and_then(|st| st.result.as_ref())
                .map(|rs| rs.rows.iter().map(|r| ValueCount { value: r.first().cloned().flatten(), count: r.get(1).and_then(|c| c.as_deref()).and_then(|c| c.parse().ok()).unwrap_or(0) }).collect())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        out.push(ColumnProfile { name: name.clone(), rows, nulls: rows - not_null.min(rows), distinct, unique, min, max, top });
    }
    Ok(DataProfile { rows: total, columns: out })
}

/// Perfil da tabela: contagens da estrutura e total de linhas (o Firebird não informa o tamanho por tabela sem as tabelas `MON$`).
pub(crate) fn table_profile_on(conn: &mut Conn, table: &str) -> Result<TableProfile, DatabaseError> {
    let detail = super::meta::table_detail_on(conn, table)?;
    let rows = super::count_on(conn, table, None)? as i64;
    let count_kind = |k: &str| detail.constraints.iter().filter(|c| c.kind == k).count() as i64;
    Ok(TableProfile {
        owner: detail.owner.clone().unwrap_or_default(),
        kind: "r".into(),
        persistence: "permanent".into(),
        tablespace: String::new(),
        estimated_rows: rows,
        table_bytes: 0,
        index_bytes: 0,
        total_bytes: 0,
        columns: detail.columns.len() as i64,
        indexes: detail.indexes.len() as i64,
        primary_keys: i64::from(detail.columns.iter().any(|c| c.key.is_some())),
        foreign_keys: count_kind("f"),
        uniques: count_kind("u"),
        checks: count_kind("c"),
        triggers: detail.triggers.len() as i64,
        row_security: false,
        replica_identity: "d".into(),
        live_tuples: Some(rows),
        dead_tuples: None,
        seq_scans: None,
        index_scans: None,
        last_vacuum: None,
        last_autovacuum: None,
        last_analyze: None,
        last_autoanalyze: None,
        comment: detail.comment.clone(),
    })
}

/// Diagrama ER: as tabelas (com colunas, chave primária e chave estrangeira) e as chaves estrangeiras entre elas.
pub(crate) fn schema_diagram_on(conn: &mut Conn) -> Result<SchemaDiagram, DatabaseError> {
    let names: Vec<String> = super::meta::list_objects_on(conn)?.tables.into_iter().map(|t| t.name).collect();
    let mut tables = Vec::with_capacity(names.len());
    let mut relations = Vec::new();
    for table in &names {
        let d = super::meta::table_detail_on(conn, table)?;
        let fks: Vec<&atlas_core::ConstraintInfo> = d.constraints.iter().filter(|c| c.kind == "f").collect();
        let fk_cols: Vec<String> = fks.iter().flat_map(|f| f.fields.split(", ").map(str::to_string)).collect();
        let columns = d
            .columns
            .iter()
            .map(|c| DiagramColumn { name: c.name.clone(), data_type: c.formatted_type.clone(), length: c.length, decimal: c.decimal, is_pk: c.key.is_some(), is_fk: fk_cols.contains(&c.name) })
            .collect();
        tables.push(DiagramTable { name: table.clone(), columns });
        for f in fks {
            let from: Vec<String> = f.fields.split(", ").map(str::to_string).collect();
            relations.push(DiagramRelation {
                name: f.name.clone(),
                table: table.clone(),
                required: from.iter().all(|c| d.columns.iter().any(|x| &x.name == c && x.not_null)),
                columns: from,
                ref_schema: "main".into(),
                ref_table: f.ref_table.clone().unwrap_or_default(),
                ref_columns: f.ref_fields.clone().unwrap_or_default().split(", ").map(str::to_string).collect(),
            });
        }
    }
    Ok(SchemaDiagram { tables, relations })
}

/// Rótulos de chaves estrangeiras: para cada chave pedida, o texto da coluna de descrição da tabela referenciada.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FkLabels {
    /// Coluna usada como descrição; `None` se a tabela não tem uma que sirva (a tela mostra a chave mesmo).
    pub label_column: Option<String>,
    /// Pares `[chave, rótulo]` (só das chaves que existem na tabela).
    pub labels: Vec<[String; 2]>,
}

/// Escolhe a coluna de descrição: nomes típicos primeiro (`nome`, `name`, `descricao`...), depois a primeira coluna de texto
/// que não seja a própria chave nem pareça outra chave (`..._id`, `id_...`, `codigo`).
fn pick_label_column(columns: &[(String, String)], key: &str) -> Option<String> {
    let textual = |t: &str| {
        let t = t.to_ascii_lowercase();
        t.contains("char") || t.contains("text")
    };
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

pub(crate) fn fk_labels_on(conn: &mut Conn, table: &str, key_column: &str, keys: &[String]) -> Result<FkLabels, DatabaseError> {
    let none = FkLabels { label_column: None, labels: Vec::new() };
    if keys.is_empty() {
        return Ok(none);
    }
    let detail = super::meta::table_detail_on(conn, table)?;
    let columns: Vec<(String, String)> = detail.columns.iter().map(|c| (c.name.clone(), c.data_type.clone())).collect();
    let Some(label) = pick_label_column(&columns, key_column) else { return Ok(none) };
    let mut labels = Vec::new();
    for chunk in keys.iter().take(MAX_FK_KEYS).collect::<Vec<_>>().chunks(500) {
        let marks = vec!["?"; chunk.len()].join(", ");
        let sql = format!(
            "SELECT CAST({k} AS VARCHAR(255)), CAST({l} AS VARCHAR(255)) FROM {t} WHERE CAST({k} AS VARCHAR(255)) IN ({marks}) AND {l} IS NOT NULL",
            k = quote_ident(key_column),
            l = quote_ident(&label),
            t = quote_ident(table),
        );
        let params: Vec<SqlType> = chunk.iter().map(|k| SqlType::Text((*k).clone())).collect();
        for r in super::meta::query(conn, &sql, params)? {
            labels.push([r[0].clone().unwrap_or_default(), r[1].clone().unwrap_or_default()]);
        }
    }
    Ok(FkLabels { label_column: Some(label), labels })
}
