//! Comandos IPC chamados pela interface (`invoke("pg_...")`).
//! Erros voltam tipados (`DatabaseError`); nenhum comando registra parâmetros em log.

use crate::local::Local;
use crate::sessions::Sessions;
use atlas_core::{
    AvailableExtension, ConnectionParams, DatabaseDetail, DatabaseError, DatabaseList, ExtensionDetail, FunctionDetail, OrderBy, RoleDetail, SchemaDetail, SequenceDetail,
    TypeDetail, QueryOutcome, RoleInfo, SchemaDiagram, SchemaInfo, SchemaObjects, ServerInfo, TableDetail, TableFilter, ViewDetail,
};
use serde::Serialize;
use tauri::State;

/// Limite de linhas por result set no editor (o resto é descartado e sinalizado como truncado).
const MAX_ROWS_CAP: usize = 100_000;

/// Teto de linhas ao abrir uma tabela, inclusive com "Limitar registros" desmarcado.
pub(crate) const TABLE_ROWS_CAP: usize = 1_000_000;

/// `profile_id`: usa a senha salva no cofre quando `params.password` vier vazia.
#[tauri::command]
pub async fn pg_test_connection(
    local: State<'_, Local>,
    params: ConnectionParams,
    profile_id: Option<String>,
) -> Result<ServerInfo, DatabaseError> {
    let params = local.resolve_password(params, profile_id.as_deref())?;
    let session = atlas_pg::connect(&params, None).await?;
    Ok(session.server.clone())
}

#[tauri::command]
pub async fn pg_open(
    sessions: State<'_, Sessions>,
    local: State<'_, Local>,
    session_id: String,
    params: ConnectionParams,
    profile_id: Option<String>,
) -> Result<ServerInfo, DatabaseError> {
    let params = local.resolve_password(params, profile_id.as_deref())?;
    let s = sessions.open(&session_id, params).await?;
    Ok(s.server.clone())
}

#[tauri::command]
pub async fn pg_close(sessions: State<'_, Sessions>, session_id: String) -> Result<(), DatabaseError> {
    sessions.close(&session_id).await;
    Ok(())
}

#[tauri::command]
pub async fn pg_close_database(sessions: State<'_, Sessions>, session_id: String, database: String) -> Result<(), DatabaseError> {
    sessions.close_database(&session_id, &database).await;
    Ok(())
}

#[tauri::command]
pub async fn pg_list_databases(sessions: State<'_, Sessions>, session_id: String) -> Result<DatabaseList, DatabaseError> {
    let s = sessions.get(&session_id, None).await?;
    atlas_pg::list_databases(&s).await
}

#[tauri::command]
pub async fn pg_list_schemas(sessions: State<'_, Sessions>, session_id: String, database: String) -> Result<Vec<SchemaInfo>, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::list_schemas(&s).await
}

#[tauri::command]
pub async fn pg_list_objects(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    schema: String,
) -> Result<SchemaObjects, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::list_objects(&s, &schema).await
}

#[tauri::command]
pub async fn pg_table_detail(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    schema: String,
    table: String,
) -> Result<TableDetail, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::table_detail(&s, &schema, &table).await
}

#[tauri::command]
pub async fn pg_view_detail(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    schema: String,
    view: String,
) -> Result<ViewDetail, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::view_detail(&s, &schema, &view).await
}

/// "Localizar no banco/schema": nomes, colunas, definições e comentários.
#[tauri::command]
pub async fn pg_find(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    options: atlas_pg::FindOptions,
) -> Result<atlas_pg::FindResult, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::find(&s, &options).await
}

/// Tabelas (com colunas) e chaves estrangeiras de um schema, para o diagrama ER.
#[tauri::command]
pub async fn pg_schema_diagram(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    schema: String,
) -> Result<SchemaDiagram, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::schema_diagram(&s, &schema).await
}

/// Extensões que o servidor oferece e ainda não estão instaladas no banco.
#[tauri::command]
pub async fn pg_available_extensions(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
) -> Result<Vec<AvailableExtension>, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::available_extensions(&s).await
}

#[tauri::command]
pub async fn pg_extension_detail(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    name: String,
) -> Result<ExtensionDetail, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::extension_detail(&s, &name).await
}

/// Atributos de um banco (leitura do catálogo do cluster; usa a sessão do banco inicial).
#[tauri::command]
pub async fn pg_database_detail(sessions: State<'_, Sessions>, session_id: String, name: String) -> Result<DatabaseDetail, DatabaseError> {
    let s = sessions.get(&session_id, None).await?;
    atlas_pg::database_detail(&s, &name).await
}

#[tauri::command]
pub async fn pg_schema_detail(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    name: String,
) -> Result<SchemaDetail, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::schema_detail(&s, &name).await
}

#[tauri::command]
pub async fn pg_role_detail(sessions: State<'_, Sessions>, session_id: String, name: String) -> Result<RoleDetail, DatabaseError> {
    let s = sessions.get(&session_id, None).await?;
    atlas_pg::role_detail(&s, &name).await
}

#[tauri::command]
pub async fn pg_type_detail(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    schema: String,
    name: String,
) -> Result<TypeDetail, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::type_detail(&s, &schema, &name).await
}

#[tauri::command]
pub async fn pg_sequence_detail(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    schema: String,
    name: String,
) -> Result<SequenceDetail, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::sequence_detail(&s, &schema, &name).await
}

#[tauri::command]
pub async fn pg_function_detail(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    schema: String,
    name: String,
    identity: String,
) -> Result<FunctionDetail, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::function_detail(&s, &schema, &name, &identity).await
}

#[tauri::command]
pub async fn pg_matview_detail(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    schema: String,
    view: String,
) -> Result<ViewDetail, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::matview_detail(&s, &schema, &view).await
}

#[tauri::command]
pub async fn pg_list_roles(sessions: State<'_, Sessions>, session_id: String) -> Result<Vec<RoleInfo>, DatabaseError> {
    let s = sessions.get(&session_id, None).await?;
    atlas_pg::list_roles(&s).await
}

#[tauri::command]
pub async fn pg_execute(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    sql: String,
    max_rows: usize,
    // Schema escolhido no editor: nomes sem schema na consulta passam a valer nele.
    schema: Option<String>,
) -> Result<QueryOutcome, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::execute_in_schema(&s, &sql, max_rows.clamp(1, MAX_ROWS_CAP), schema.as_deref()).await
}

#[tauri::command]
pub async fn pg_cancel(sessions: State<'_, Sessions>, session_id: String, database: String) -> Result<(), DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    s.cancel().await
}

/// Aplica alterações (grid/designer) em transação numa conexão dedicada.
#[tauri::command]
pub async fn pg_apply(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    statements: Vec<atlas_pg::PlannedStatement>,
) -> Result<Vec<u64>, DatabaseError> {
    let conn = sessions.dedicated(&session_id, &database).await?;
    atlas_pg::apply_in_transaction(conn, &statements).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableRows {
    pub sql: String,
    pub outcome: QueryOutcome,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn pg_table_rows(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    schema: String,
    table: String,
    order_by: Vec<OrderBy>,
    filter: Option<TableFilter>,
    limit: Option<u32>,
    offset: u64,
) -> Result<TableRows, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    let limit = limit.map(|l| l.clamp(1, TABLE_ROWS_CAP as u32));
    let (sql, outcome) = atlas_pg::table_rows(&s, &schema, &table, &order_by, filter.as_ref(), limit, offset, TABLE_ROWS_CAP).await?;
    Ok(TableRows { sql, outcome })
}

#[tauri::command]
pub async fn pg_table_count(
    sessions: State<'_, Sessions>,
    session_id: String,
    database: String,
    schema: String,
    table: String,
    filter: Option<TableFilter>,
) -> Result<u64, DatabaseError> {
    let s = sessions.get(&session_id, Some(&database)).await?;
    atlas_pg::table_count(&s, &schema, &table, filter.as_ref()).await
}
