//! Shell Tauri de DEV COCKPIT (SPEC §6.5, §6.6).
//!
//! Rust es la única fuente de estado: el WebView se suscribe con un `Channel`,
//! recibe una instantánea y después solo los cambios.

mod procs;

use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cockpit_core::privacy::PrivacyPolicy;
use cockpit_ingest::live::{LiveConfig, LiveDelta, LivePipeline, LiveState};
use cockpit_store::DB_FILE_NAME;
use tauri::ipc::Channel;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WebviewWindow, WindowEvent};

/// Los cambios se agrupan en ventanas de este tamaño antes de publicarse.
const COALESCE: Duration = Duration::from_millis(100);
const MAIN_WINDOW: &str = "main";

type Subscriber = Arc<Mutex<Option<Channel<LiveDelta>>>>;

struct AppState {
    live: LiveState,
    subscriber: Subscriber,
    wake: Sender<()>,
    /// Fichero CSV de latencias, solo si `DEV_COCKPIT_PERF_LOG` está definida (B4).
    perf_log: Option<PathBuf>,
    _pipeline: LivePipeline,
}

/// Conecta el stream de la vista En vivo. Envía primero la instantánea completa.
#[tauri::command]
fn subscribe(state: State<'_, AppState>, on_delta: Channel<LiveDelta>) -> Result<(), String> {
    // Con el cerrojo tomado, el publicador no puede colar un cambio antes de la instantánea.
    let mut subscriber = state.subscriber.lock().unwrap_or_else(|e| e.into_inner());
    on_delta
        .send(state.live.snapshot())
        .map_err(|e| e.to_string())?;
    *subscriber = Some(on_delta);
    Ok(())
}

/// PIDs del grupo WebView2 propio de la app (C2, SPEC §11.4).
#[tauri::command]
async fn webview_processes(window: WebviewWindow) -> Result<Vec<procs::ProcessInfo>, String> {
    procs::webview_processes(&window).await
}

/// `true` si esta ejecución está midiendo latencias (B4).
#[tauri::command]
fn perf_enabled(state: State<'_, AppState>) -> bool {
    state.perf_log.is_some()
}

/// Registra una latencia transcript → pintado, solo durante las mediciones de B4.
#[tauri::command]
fn perf_mark(state: State<'_, AppState>, kind: String, event_ts_ms: i64, painted_ts_ms: i64) {
    let Some(path) = &state.perf_log else { return };
    // `tool`: una herramienta nueva (B3). `other`: cualquier otro cambio (B4).
    let kind = if kind == "tool" { "tool" } else { "other" };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(
            file,
            "{event_ts_ms},{painted_ts_ms},{},{kind}",
            painted_ts_ms - event_ts_ms
        );
    }
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Rutas de la canalización. Las variables `DEV_COCKPIT_*` solo existen para las
/// pruebas, que no deben tocar `~/.claude` ni la base de datos real.
fn live_config() -> Result<LiveConfig, String> {
    let claude = home_dir().map(|h| h.join(".claude"));
    let projects_root = env_path("DEV_COCKPIT_PROJECTS_DIR")
        .or_else(|| claude.as_ref().map(|c| c.join("projects")))
        .ok_or("no se encuentra la carpeta del usuario")?;
    let sessions_dir =
        env_path("DEV_COCKPIT_SESSIONS_DIR").or_else(|| claude.map(|c| c.join("sessions")));
    let db_path = match env_path("DEV_COCKPIT_DB") {
        Some(path) => path,
        None => PathBuf::from(env::var_os("APPDATA").ok_or("no se encuentra %APPDATA%")?)
            .join("dev-cockpit")
            .join(DB_FILE_NAME),
    };
    // El mod busca `endpoint.json` junto a la base de datos real. Con una base de
    // pruebas, el fichero queda en la carpeta de pruebas y el mod real no lo ve.
    let endpoint_path = db_path.parent().map(|dir| dir.join("endpoint.json"));
    Ok(LiveConfig {
        projects_root,
        sessions_dir,
        db_path,
        endpoint_path,
        policy: PrivacyPolicy::APP_DEFAULT,
    })
}

fn window_is_showing(app: &AppHandle) -> bool {
    app.get_webview_window(MAIN_WINDOW)
        .is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false))
}

/// Publica los cambios coalescidos. Con la ventana oculta no publica nada: los
/// cambios se acumulan en `LiveState` y salen juntos cuando vuelve a mostrarse.
fn run_publisher(app: AppHandle, live: LiveState, subscriber: Subscriber, wake: Receiver<()>) {
    while wake.recv().is_ok() {
        thread::sleep(COALESCE);
        while wake.try_recv().is_ok() {}
        if !window_is_showing(&app) {
            continue;
        }
        let mut subscriber = subscriber.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(channel) = subscriber.as_ref()
            && let Some(update) = live.take_update()
            && channel.send(update).is_err()
        {
            // El WebView se ha recargado: la nueva página volverá a suscribirse.
            *subscriber = None;
        }
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let (wake_tx, wake_rx) = mpsc::channel();
    let notifier = wake_tx.clone();
    let live = LiveState::new(move || {
        let _ = notifier.send(());
    });
    let pipeline = LivePipeline::spawn(live_config()?, live.clone())?;
    let subscriber = Subscriber::default();

    let handle = app.handle().clone();
    let (publisher_live, publisher_subscriber) = (live.clone(), subscriber.clone());
    thread::Builder::new()
        .name("cockpit-publisher".to_owned())
        .spawn(move || run_publisher(handle, publisher_live, publisher_subscriber, wake_rx))?;

    app.manage(AppState {
        live,
        subscriber,
        wake: wake_tx,
        perf_log: env_path("DEV_COCKPIT_PERF_LOG"),
        _pipeline: pipeline,
    });

    // Mediciones de memoria (B8, B9): publica cada segundo los PIDs del grupo propio.
    if let Some(path) = env_path("DEV_COCKPIT_PROCS_LOG")
        && let Some(window) = app.get_webview_window(MAIN_WINDOW)
    {
        thread::Builder::new()
            .name("cockpit-procs".to_owned())
            .spawn(move || {
                loop {
                    thread::sleep(Duration::from_secs(1));
                    let report =
                        match tauri::async_runtime::block_on(procs::webview_processes(&window)) {
                            Ok(processes) => {
                                serde_json::json!({ "ok": true, "processes": processes })
                            }
                            Err(error) => serde_json::json!({ "ok": false, "error": error }),
                        };
                    let _ = std::fs::write(&path, report.to_string());
                }
            })?;
    }

    let show = MenuItem::with_id(app, "show", "Mostrar DEV COCKPIT", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Salir", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("DEV COCKPIT")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(setup)
        .on_window_event(|window, event| match event {
            // Cerrar la ventana la oculta en la bandeja; se sale desde el menú.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            // Al volver a primer plano se publica lo acumulado mientras estaba oculta.
            WindowEvent::Focused(true) => {
                if let Some(state) = window.try_state::<AppState>() {
                    let _ = state.wake.send(());
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            subscribe,
            webview_processes,
            perf_enabled,
            perf_mark
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
