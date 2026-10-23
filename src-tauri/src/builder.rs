//! Janela do Query Builder: uma janela do sistema à parte (maximiza pela tela, não pela janela principal).
//!
//! O conteúdo é o mesmo front-end; os parâmetros seguem em `window.__ATLAS_QB__` e o SELECT montado volta à janela
//! principal pelo evento `qb-build`. Esta janela usa as mesmas conexões (a sessão é pelo id), então não há estado a copiar.

use crate::WebviewArgs;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryBuilderOpen {
    /// Identifica a aba de consulta: uma aba abre no máximo uma janela.
    pub id: String,
    pub conn_id: String,
    pub database: String,
    pub default_schema: String,
    pub initial_sql: String,
    pub title: String,
}

#[tauri::command]
pub async fn open_query_builder(app: AppHandle, args: State<'_, WebviewArgs>, req: QueryBuilderOpen) -> Result<(), String> {
    // O id vem da interface e vira nome de janela: só caracteres seguros.
    if req.id.is_empty() || !req.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("invalid query builder id".into());
    }
    let label = format!("qb-{}", req.id);
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(());
    }
    let init = format!("window.__ATLAS_QB__ = {};", serde_json::to_string(&req).map_err(|e| e.to_string())?);
    let mut builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::default())
        .title(&req.title)
        .inner_size(1266.0, 793.0)
        .min_inner_size(900.0, 560.0)
        .center()
        .initialization_script(&init)
        .additional_browser_args(&args.0);
    // Janela "filha": fica à frente da principal e some junto com ela, mas a principal continua utilizável.
    if let Some(main) = app.get_webview_window("main") {
        builder = builder.parent(&main).map_err(|e| e.to_string())?;
    }
    builder.build().map_err(|e| e.to_string())?;
    Ok(())
}

/// Fecha a janela do Query Builder de uma aba (a aba de consulta foi fechada).
#[tauri::command]
pub async fn close_query_builder(app: AppHandle, id: String) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(&format!("qb-{id}")) {
        window.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataTransferOpen {
    pub title: String,
    /// Conexões (nome, servidor, versão, se estão abertas) e a origem sugerida; a janela só lê.
    pub params: serde_json::Value,
}

/// Abre (ou traz para a frente) a janela do Data Transfer, também uma janela do sistema à parte.
#[tauri::command]
pub async fn open_data_transfer(app: AppHandle, args: State<'_, WebviewArgs>, req: DataTransferOpen) -> Result<(), String> {
    let label = "dt-main";
    if let Some(existing) = app.get_webview_window(label) {
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(());
    }
    let init = format!("window.__ATLAS_DT__ = {};", serde_json::to_string(&req.params).map_err(|e| e.to_string())?);
    WebviewWindowBuilder::new(&app, label, WebviewUrl::default())
        .title(&req.title)
        .inner_size(1186.0, 793.0)
        .min_inner_size(900.0, 560.0)
        .center()
        .initialization_script(&init)
        .additional_browser_args(&args.0)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}
