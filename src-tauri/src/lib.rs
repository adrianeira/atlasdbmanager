mod commands;
mod export;
mod local;
mod sessions;

use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

/// Tempo mínimo da tela de abertura (para não piscar quando o app carrega rápido) e o máximo, depois do qual a
/// janela principal aparece mesmo que a interface nunca avise que está pronta.
const SPLASH_MIN: Duration = Duration::from_millis(1500);
const SPLASH_MAX: Duration = Duration::from_secs(20);

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
            commands::pg_list_roles,
            commands::pg_execute,
            commands::pg_cancel,
            commands::pg_table_rows,
            commands::pg_table_count,
            commands::pg_apply,
            app_ready,
            export::pg_export,
            export::pg_export_cancel,
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
