mod backup;
mod builder;
mod commands;
mod datagen;
mod monitor;
mod dictionary;
mod download;
mod datasync;
mod structsync;
mod dump;
mod export;
mod import;
mod local;
mod sessions;
mod mssql;
mod mysql;
mod sqlite;
mod transfer;
mod tunnel;
mod tx;

use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

/// Tempo mínimo da tela de abertura (para não piscar quando o app carrega rápido) e o máximo, depois do qual a
/// janela principal aparece mesmo que a interface nunca avise que está pronta.
const SPLASH_MIN: Duration = Duration::from_millis(3000);
const SPLASH_MAX: Duration = Duration::from_secs(20);
/// Tempo que a barra completa fica visível antes de a splash fechar.
const SPLASH_FINISH: Duration = Duration::from_millis(350);

/// Argumentos do WebView2 comuns a todas as janelas (todas compartilham o mesmo ambiente).
pub struct WebviewArgs(pub String);

/// Quando o app começou a abrir (para respeitar `SPLASH_MIN`).
struct StartedAt(Instant);

/// Fecha a tela de abertura e mostra a janela principal. Só age se a splash ainda existe, então chamadas
/// repetidas (aviso da interface + limite de segurança) não roubam o foco depois.
fn reveal_main(app: &AppHandle) {
    let Some(splash) = app.get_webview_window("splash") else { return };
    let _ = splash.close();
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    }
}

/// A interface carregou o que precisa (perfis de conexão lidos): troca a splash pela janela principal.
#[tauri::command]
fn app_ready(app: AppHandle, started: State<'_, StartedAt>) {
    let wait = SPLASH_MIN.saturating_sub(started.0.elapsed());
    std::thread::spawn(move || {
        std::thread::sleep(wait);
        // Deixa a barra da splash completar antes de fechá-la.
        if let Some(splash) = app.get_webview_window("splash") {
            let _ = splash.eval("window.atlasSplashDone && window.atlasSplashDone()");
            std::thread::sleep(SPLASH_FINISH);
        }
        reveal_main(&app);
    });
}

/// Argumentos padrão que o Tauri usa no WebView2 do Windows.
const DEFAULT_WEBVIEW2_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Área de transferência do sistema (copiar/colar no grid sem o pedido de permissão do WebView).
        .plugin(tauri_plugin_clipboard_manager::init())
        // Janela "Salvar como" nativa para escolher o arquivo da exportação.
        .plugin(tauri_plugin_dialog::init())
        .manage(sessions::Sessions::default())
        .manage(export::Exports::default())
        .manage(backup::Backups::default())
        .manage(tx::Transactions::default())
        .manage(sqlite::SqliteSessions::default())
        .manage(mssql::MssqlSessions::default())
        .manage(mysql::MysqlSessions::default())
        .manage(datasync::Comparisons::default())
        .manage(structsync::StructComparisons::default())
        .manage(dictionary::DictRuns::default())
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
            // Idioma forçado para testar a interface/splash (ex.: en-US); só em build de debug.
            if cfg!(debug_assertions)
                && let Ok(lang) = std::env::var("ATLAS_LANG")
                && lang.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                args.push_str(&format!(" --lang={lang}"));
            }

            app.manage(WebviewArgs(args.clone()));

            // Tela de abertura: pequena, sem bordas, mostrada enquanto o app carrega. Usa os mesmos argumentos do
            // WebView2 da janela principal (as duas compartilham o mesmo ambiente).
            WebviewWindowBuilder::new(app, "splash", WebviewUrl::App("splash.html".into()))
                .title("Atlas DB Manager")
                .inner_size(836.0, 470.0)
                .decorations(false)
                .resizable(false)
                .maximizable(false)
                .minimizable(false)
                .always_on_top(true)
                .center()
                .visible(false)
                .background_color(tauri::window::Color(233, 241, 251, 255))
                .on_page_load(|window, payload| {
                    // Só mostra a splash depois de a página (e a imagem) carregar: evita a janela preta inicial.
                    if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                        let _ = window.show();
                    }
                })
                .additional_browser_args(&args)
                .build()?;

            // A janela principal nasce oculta; `app_ready` (ou o limite de segurança) a mostra.
            WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                .title("Atlas DB Manager")
                .inner_size(1440.0, 900.0)
                .min_inner_size(1024.0, 650.0)
                .center()
                .visible(false)
                // Sem isso o WebView2 do Windows toma os arrastes para si (soltar arquivos) e o arrastar de abas e colunas não funciona.
                .disable_drag_drop_handler()
                .additional_browser_args(&args)
                .build()?;

            app.manage(StartedAt(Instant::now()));
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(SPLASH_MAX);
                reveal_main(&handle);
            });
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
            commands::pg_view_detail,
            commands::pg_matview_detail,
            commands::pg_function_detail,
            commands::pg_sequence_detail,
            commands::pg_type_detail,
            commands::pg_schema_diagram,
            commands::pg_available_extensions,
            commands::pg_extension_detail,
            commands::pg_foreign_wrappers,
            commands::pg_foreign_server_detail,
            commands::pg_tablespace_detail,
            commands::pg_table_profile,
            commands::pg_foreign_table_detail,
            commands::pg_database_detail,
            commands::pg_schema_detail,
            commands::pg_role_detail,
            commands::pg_find,
            commands::pg_find_data,
            commands::pg_list_roles,
            commands::pg_execute,
            commands::pg_cancel,
            commands::pg_table_rows,
            commands::pg_fk_labels,
            commands::pg_table_count,
            commands::pg_data_profile,
            mysql::mysql_test_connection,
            mysql::mysql_open,
            mysql::mysql_close,
            mysql::mysql_close_database,
            mysql::mysql_list_databases,
            mysql::mysql_list_schemas,
            mysql::mysql_list_objects,
            mysql::mysql_table_detail,
            mysql::mysql_list_roles,
            mysql::mysql_execute,
            mysql::mysql_cancel,
            mysql::mysql_table_rows,
            mysql::mysql_table_count,
            mysql::mysql_apply,
            mysql::mysql_fk_labels,
            mysql::mysql_schema_diagram,
            mysql::mysql_view_detail,
            mysql::mysql_export,
            mysql::mysql_export_cancel,
            mysql::mysql_import,
            mysql::mysql_find,
            mysql::mysql_database_detail,
            mysql::mysql_collations,
            mysql::mysql_data_profile,
            mysql::mysql_table_profile,
            mysql::mysql_find_data,
            mysql::mysql_dump_sql,
            mysql::mysql_backup,
            mysql::mysql_backup_list,
            mysql::mysql_restore,
            mysql::mysql_console_run,
            mysql::mysql_routine_definition,
            mysql::mysql_console_prompt,
            mysql::mysql_console_complete,
            mysql::mysql_unsupported,
            mysql::mysql_tx_begin,
            mysql::mysql_tx_apply,
            mysql::mysql_tx_execute,
            mysql::mysql_tx_table_rows,
            mysql::mysql_tx_table_count,
            mysql::mysql_tx_end,
            mssql::mssql_backup_delete,
            mssql::mssql_test_connection,
            mssql::mssql_open,
            mssql::mssql_close,
            mssql::mssql_close_database,
            mssql::mssql_list_databases,
            mssql::mssql_list_schemas,
            mssql::mssql_list_objects,
            mssql::mssql_table_detail,
            mssql::mssql_list_roles,
            mssql::mssql_execute,
            mssql::mssql_cancel,
            mssql::mssql_table_rows,
            mssql::mssql_table_count,
            mssql::mssql_apply,
            mssql::mssql_fk_labels,
            mssql::mssql_schema_diagram,
            mssql::mssql_export,
            mssql::mssql_import,
            mssql::mssql_view_detail,
            mssql::mssql_data_profile,
            mssql::mssql_table_profile,
            mssql::mssql_console_run,
            mssql::mssql_console_prompt,
            mssql::mssql_console_complete,
            mssql::mssql_console_is_complete,
            mssql::mssql_routine_definition,
            mssql::mssql_dump_sql,
            mssql::mssql_backup,
            mssql::mssql_backup_list,
            mssql::mssql_restore,
            mssql::mssql_find,
            mssql::mssql_find_data,
            mssql::mssql_export_cancel,
            mssql::mssql_unsupported,
            mssql::mssql_tx_begin,
            mssql::mssql_tx_apply,
            mssql::mssql_tx_execute,
            mssql::mssql_tx_table_rows,
            mssql::mssql_tx_table_count,
            mssql::mssql_tx_end,
            sqlite::sqlite_test_connection,
            sqlite::sqlite_open,
            sqlite::sqlite_close,
            sqlite::sqlite_close_database,
            sqlite::sqlite_list_databases,
            sqlite::sqlite_list_schemas,
            sqlite::sqlite_list_objects,
            sqlite::sqlite_table_detail,
            sqlite::sqlite_list_roles,
            sqlite::sqlite_execute,
            sqlite::sqlite_cancel,
            sqlite::sqlite_table_rows,
            sqlite::sqlite_table_count,
            sqlite::sqlite_apply,
            sqlite::sqlite_fk_labels,
            sqlite::sqlite_data_profile,
            sqlite::sqlite_table_profile,
            sqlite::sqlite_schema_diagram,
            sqlite::sqlite_dump_sql,
            sqlite::sqlite_console_run,
            sqlite::sqlite_console_prompt,
            sqlite::sqlite_console_complete,
            sqlite::sqlite_console_is_complete,
            sqlite::sqlite_view_detail,
            sqlite::sqlite_create_file,
            sqlite::sqlite_backup,
            sqlite::sqlite_backup_list,
            sqlite::sqlite_restore,
            sqlite::sqlite_export,
            sqlite::sqlite_import,
            sqlite::sqlite_find,
            sqlite::sqlite_find_data,
            sqlite::sqlite_export_cancel,
            sqlite::sqlite_unsupported,
            sqlite::sqlite_tx_begin,
            sqlite::sqlite_tx_apply,
            sqlite::sqlite_tx_execute,
            sqlite::sqlite_tx_table_rows,
            sqlite::sqlite_tx_table_count,
            sqlite::sqlite_tx_end,
            commands::pg_apply,
            tx::pg_tx_begin,
            tx::pg_tx_execute,
            tx::pg_tx_apply,
            tx::pg_tx_table_rows,
            tx::pg_tx_table_count,
            tx::pg_tx_end,
            app_ready,
            builder::open_query_builder,
            builder::close_query_builder,
            builder::open_data_transfer,
            transfer::pg_transfer,
            builder::open_data_sync,
            builder::open_struct_sync,
            builder::open_data_gen,
            builder::open_data_dictionary,
            builder::open_tab_window,
            dictionary::dict_generate,
            monitor::monitor_snapshot,
            monitor::monitor_end_process,
            monitor::sqllog_read,
            monitor::sqllog_clear,
            monitor::sqllog_set_paused,
            monitor::console_run,
            monitor::console_prompt,
            monitor::console_is_complete,
            monitor::console_complete,
            dictionary::dict_cancel,
            dictionary::dict_defaults,
            dictionary::dict_preview,
            dictionary::dict_open,
            datagen::datagen_describe,
            datagen::datagen_preview,
            datagen::datagen_run,
            structsync::structsync_compare,
            structsync::structsync_script,
            structsync::structsync_deploy,
            structsync::structsync_forget,
            datasync::datasync_compare,
            datasync::datasync_rows,
            datasync::datasync_script,
            datasync::datasync_script_full,
            datasync::datasync_deploy,
            datasync::datasync_forget,
            export::pg_export,
            dump::pg_dump_sql,
            backup::pg_tools_locate,
            backup::pg_backup,
            backup::pg_backup_list,
            backup::pg_backup_delete,
            backup::pg_restore,
            backup::pg_backup_cancel,
            export::pg_export_cancel,
            import::import_list_tables,
            import::import_preview,
            import::pg_import,
            local::local_status,
            local::profiles_list,
            local::profile_save,
            local::profile_delete,
            local::ssh_password_save,
            local::open_external,
            download::download_to_temp,
            local::file_read_hex,
            local::file_write_hex,
            local::connections_export,
            local::connections_import,
            local::history_add,
            local::history_list,
            local::history_favorite,
            local::history_delete,
            local::history_clear,
            local::snippets_list,
            local::snippet_save,
            local::snippet_delete,
            local::pref_get,
            local::pref_set,
            local::tab_state_save,
            local::tab_state_take,
        ])
        .run(tauri::generate_context!())
        .expect("falha ao iniciar o Atlas DB Manager");
}
