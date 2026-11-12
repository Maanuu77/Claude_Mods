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
use cockpit_ingest::git::{FileDiff, GitReader};
use cockpit_ingest::live::{LiveConfig, LiveDelta, LivePipeline, LiveState};
use cockpit_ingest::project_scan::{self, ProjectScan};
use cockpit_ingest::workspace::{FilePreview, read_preview};
use cockpit_store::{DB_FILE_NAME, Diagnostics, FileEventView, Store};
use tauri::ipc::Channel;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WebviewWindow, WindowEvent};

/// Los cambios se agrupan en ventanas de este tamaño antes de publicarse.
const COALESCE: Duration = Duration::from_millis(100);
const MAIN_WINDOW: &str = "main";
/// Tope de una página del histórico de cambios.
const MAX_PAGE: u32 = 500;

type Subscriber = Arc<Mutex<Option<Channel<LiveDelta>>>>;

struct AppState {
    live: LiveState,
    subscriber: Subscriber,
    wake: Sender<()>,
    /// Fichero CSV de latencias, solo si `DEV_COCKPIT_PERF_LOG` está definida (B4).
    perf_log: Option<PathBuf>,
    db_path: PathBuf,
    /// Conexión de solo lectura para las consultas, abierta la primera vez.
    reader: Mutex<Option<Store>>,
    _pipeline: LivePipeline,
}

impl AppState {
    /// Raíz de un proyecto **activo**. Las consultas de Git y las vistas previas
    /// solo se admiten sobre las raíces que la app está vigilando.
    fn active_root(&self, root_id: i64) -> Result<PathBuf, String> {
        self.live
            .workspaces()
            .into_iter()
            .find(|w| w.root_id == root_id)
            .map(|w| PathBuf::from(w.root))
            .ok_or_else(|| "la raíz no está activa".to_owned())
    }
}

/// Histórico de cambios de fichero de una raíz (últimos 30 días, §7.4).
#[tauri::command]
fn query_file_events(
    state: State<'_, AppState>,
    root_id: i64,
    before_id: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<FileEventView>, String> {
    let mut reader = state.reader.lock().unwrap_or_else(|e| e.into_inner());
    if reader.is_none() {
        *reader = Some(Store::open_read_only(&state.db_path).map_err(|e| e.to_string())?);
    }
    let store = reader.as_ref().ok_or("sin base de datos")?;
    store
        .file_events_page(root_id, before_id, limit.unwrap_or(200).min(MAX_PAGE))
        .map_err(|e| e.to_string())
}

/// Project Map de una raíz activa (SPEC §9): estructura (ficheros, bytes y lenguajes
/// por directorio) y componentes de sus manifiestos. Solo lectura y sin contenido:
/// de los ficheros solo se mira el tamaño y solo se leen los manifiestos.
#[tauri::command]
async fn project_map(state: State<'_, AppState>, root_id: i64) -> Result<ProjectScan, String> {
    let root = state.active_root(root_id)?;
    tauri::async_runtime::spawn_blocking(move || project_scan::scan(&root, &GitReader::new()))
        .await
        .map_err(|e| e.to_string())
}

/// Diagnóstico (SPEC §7, vista 5, y avisos de §6.4.2), con los mismos cortes que la
/// compactación, más el tamaño de la base en disco. Solo lectura.
#[derive(serde::Serialize)]
struct DiagnosticsView {
    #[serde(flatten)]
    store: Diagnostics,
    db_bytes: u64,
    wal_bytes: u64,
}

#[tauri::command]
fn diagnostics(state: State<'_, AppState>) -> Result<DiagnosticsView, String> {
    let mut reader = state.reader.lock().unwrap_or_else(|e| e.into_inner());
    if reader.is_none() {
        *reader = Some(Store::open_read_only(&state.db_path).map_err(|e| e.to_string())?);
    }
    let store = reader.as_ref().ok_or("sin base de datos")?;
    let now = cockpit_ingest::live::now_ms();
    let closed = cockpit_core::time::closed_before(now);
    let purge = cockpit_core::time::local_day_minus(now, cockpit_core::PATH_DETAIL_DAYS);
    let size = |suffix: &str| {
        let mut path = state.db_path.clone().into_os_string();
        path.push(suffix);
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    };
    Ok(DiagnosticsView {
        store: store
            .diagnostics(&closed, &purge)
            .map_err(|e| e.to_string())?,
        db_bytes: size(""),
        wal_bytes: size("-wal"),
    })
}

/// Diff de un fichero respecto a `HEAD`, bajo demanda. Solo viaja en la respuesta:
/// no se guarda en disco ni en caché (§6.5).
#[tauri::command]
async fn git_diff(
    state: State<'_, AppState>,
    root_id: i64,
    rel_path: String,
) -> Result<FileDiff, String> {
    let root = state.active_root(root_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        GitReader::new()
            .diff_file(&root, &rel_path)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Primeros 256 KB de un fichero sin seguimiento, en memoria (§7.2).
#[tauri::command]
async fn read_file_preview(
    state: State<'_, AppState>,
    root_id: i64,
    rel_path: String,
) -> Result<FilePreview, String> {
    let root = state.active_root(root_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        read_preview(&root, &rel_path).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
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
    // `file`: un cambio de fichero nuevo (B6). `numstat`: un `+/−` actualizado (B7).
    let kind = match kind.as_str() {
        "tool" | "file" | "numstat" => kind.as_str(),
        _ => "other",
    };
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
    let config = live_config()?;
    let db_path = config.db_path.clone();
    let pipeline = LivePipeline::spawn(config, live.clone())?;
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
        db_path,
        reader: Mutex::new(None),
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
            perf_mark,
            query_file_events,
            git_diff,
            read_file_preview,
            project_map,
            diagnostics
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
