use std::collections::HashSet;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use tauri::webview::NewWindowResponse;
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tokio::sync::oneshot;

struct DesktopState {
    shutdown: Mutex<Option<oneshot::Sender<()>>>,
    server_stopped: AtomicBool,
}

fn request_shutdown(app: &tauri::AppHandle) {
    let state = app.state::<DesktopState>();
    if let Some(tx) = state.shutdown.lock().unwrap().take() {
        let _ = tx.send(());
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let state = app.state::<DesktopState>();
            if !state.server_stopped.load(Ordering::Acquire) {
                tracing::error!("desktop shutdown timed out; forcing exit");
                app.exit(1);
            }
        });
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let log_dir = app.path().app_data_dir()?.join("logs");
            std::fs::create_dir_all(&log_dir)?;

            let mut runtime = rustytune_server::config::RuntimeConfig::desktop();
            runtime.server.open_browser = false;
            runtime.server.port = 0;
            runtime.logging.directory = log_dir;

            let listener = tauri::async_runtime::block_on(tokio::net::TcpListener::bind((
                runtime.server.bind,
                runtime.server.port,
            )))?;
            let addr = listener.local_addr()?;
            runtime.server.port = addr.port();

            let symbols = HashSet::new();
            let def = ts_ini::parse_with_symbols(rustytune_server::EMBEDDED_INI, &symbols)
                .map_err(|err| format!("failed to parse embedded ECU definition: {err}"))?;
            for warning in &def.warnings {
                tracing::warn!("ini: {warning}");
            }

            let state = rustytune_server::build_state_with_runtime(def, Vec::new(), runtime);
            rustytune_server::spawn_auto_connect(state.clone());

            let origin = format!("http://{addr}");
            let window_url = origin.parse()?;
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            app.manage(DesktopState {
                shutdown: Mutex::new(Some(shutdown_tx)),
                server_stopped: AtomicBool::new(false),
            });

            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let result = rustytune_server::serve_with_shutdown(listener, state, async move {
                    let _ = shutdown_rx.await;
                })
                .await;
                match &result {
                    Ok(()) => tracing::info!("desktop server stopped"),
                    Err(err) => tracing::error!("desktop server failed: {err}"),
                }
                app_handle
                    .state::<DesktopState>()
                    .server_stopped
                    .store(true, Ordering::Release);
                app_handle.exit(if result.is_ok() { 0 } else { 1 });
            });

            let allowed_origin = origin.clone();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(window_url))
                .title("RustyTune")
                .inner_size(1200.0, 800.0)
                .min_inner_size(800.0, 600.0)
                .resizable(true)
                .on_navigation(move |url| {
                    let allowed = url.as_str().starts_with(&format!("{allowed_origin}/"));
                    if !allowed
                        && url.scheme() == "https"
                        && let Err(err) = open::that(url.as_str())
                    {
                        tracing::warn!("could not open external link: {err}");
                    }
                    allowed
                })
                .on_new_window(|url, _features| {
                    if url.scheme() == "https"
                        && let Err(err) = open::that(url.as_str())
                    {
                        tracing::warn!("could not open external link: {err}");
                    }
                    NewWindowResponse::Deny
                })
                .build()?;

            tracing::info!("desktop UI listening on {origin}");
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle().clone();
                request_shutdown(&app);
                let _ = window.destroy();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build RustyTune desktop application");

    app.run(|app, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            let state = app.state::<DesktopState>();
            if !state.server_stopped.load(Ordering::Acquire) {
                api.prevent_exit();
                request_shutdown(app);
            }
        }
    });
}
