mod commands;
mod local;
mod sessions;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

/// Argumentos padrão que o Tauri usa no WebView2 do Windows.
const DEFAULT_WEBVIEW2_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(sessions::Sessions::default())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(tauri_plugin_log::Builder::default().level(log::LevelFilter::Info).build())?;
            }

            // Pasta de dados do app no perfil do usuário (sem admin): %LocalAppData%\dev.atlas.dbmanager
            // no Windows. Não usa %LocalAppData%\Atlas\DBManager porque essa pasta pertence ao
            // protótipo .NET anterior (schema diferente) e não deve ser tocada.
            let data_dir = app.path().app_local_data_dir()?;
            app.manage(local::Local::open(data_dir.join("atlas.db")));

            let mut args = DEFAULT_WEBVIEW2_ARGS.to_string();
            // Porta de depuração para testes automatizados da UI: só em build de debug
            // e só quando ATLAS_DEVTOOLS_PORT é definida. Nunca existe no release.
            if cfg!(debug_assertions)
                && let Ok(port) = std::env::var("ATLAS_DEVTOOLS_PORT")
                && port.parse::<u16>().is_ok()
            {
                args.push_str(&format!(" --remote-debugging-port={port}"));
            }

            WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("Atlas DB Manager")
                .inner_size(1440.0, 900.0)
                .min_inner_size(1024.0, 650.0)
                .center()
                .additional_browser_args(&args)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::pg_test_connection,
            commands::pg_open,
            commands::pg_close,
            commands::pg_close_database,
            commands::pg_list_databases,
            commands::pg_list_schemas,
            commands::pg_list_objects,
            commands::pg_table_detail,
            commands::pg_list_roles,
            commands::pg_execute,
            commands::pg_cancel,
            commands::pg_table_rows,
            commands::pg_apply,
            local::local_status,
            local::profiles_list,
            local::profile_save,
            local::profile_delete,
            local::pref_get,
            local::pref_set,
        ])
        .run(tauri::generate_context!())
        .expect("falha ao iniciar o Atlas DB Manager");
}
