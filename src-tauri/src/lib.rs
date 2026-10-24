mod backup;
mod builder;
mod commands;
mod datagen;
mod monitor;
mod dictionary;
mod datasync;
mod structsync;
mod dump;
mod export;
mod import;
mod local;
mod sessions;
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
        ])
        .run(tauri::generate_context!())
        .expect("falha ao iniciar o Atlas DB Manager");
}
