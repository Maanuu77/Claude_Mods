//! Canalización en vivo (SPEC §4): vigila los transcripts y el registro de
//! sesiones, escribe en SQLite desde **un solo hilo** y mantiene `LiveState`.
//!
//! Sondeos (SPEC §11.3): `notify` es la fuente principal. Quedan tres respaldos:
//! `stat` cada 2 s solo de los JSONL con actividad reciente, comprobación de PIDs
//! cada 30 s y reescaneo completo cada 60 s.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cockpit_core::model::TokenUsage;
use cockpit_core::privacy::PrivacyPolicy;
use cockpit_store::{SessionSummary, Store};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;

use crate::registry::{Liveness, PidProbe, RegistryEntry, read_registry};
use crate::tailer::{
    IngestFailure, IngestStats, TranscriptFile, ingest_file, ingest_tree, scan_transcripts,
};

/// Agrupa las ráfagas de eventos del sistema de ficheros.
const DEBOUNCE: Duration = Duration::from_millis(150);
/// Respaldo de `notify` para los JSONL con actividad reciente.
const ACTIVE_POLL: Duration = Duration::from_secs(2);
const PID_CHECK: Duration = Duration::from_secs(30);
const RESCAN: Duration = Duration::from_secs(60);
/// Un JSONL deja de sondearse tras este tiempo sin cambios.
const ACTIVE_FILE_WINDOW: Duration = Duration::from_secs(600);
/// Una sesión sin proceso vivo sale de la vista tras este tiempo sin actividad.
const RECENT_SESSION_MS: i64 = 30 * 60 * 1000;
/// Una herramienta en `running` desde hace más de esto no se muestra en curso.
const RUNNING_TOOL_MS: i64 = 60 * 60 * 1000;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Una sesión tal como la pinta la vista En vivo.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionView {
    #[serde(flatten)]
    pub summary: SessionSummary,
    /// `Some` si el registro dice que su proceso está vivo.
    pub live: Option<Liveness>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct BackfillState {
    pub running: bool,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// Mensaje del stream ordenado hacia la interfaz (SPEC §6.5).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveDelta {
    Snapshot {
        sessions: Vec<SessionView>,
        backfill: BackfillState,
        generated_at: i64,
    },
    Update {
        upserts: Vec<SessionView>,
        removed: Vec<String>,
        backfill: Option<BackfillState>,
        generated_at: i64,
    },
}

#[derive(Default)]
struct Inner {
    sessions: BTreeMap<String, SessionView>,
    dirty: BTreeSet<String>,
    removed: BTreeSet<String>,
    backfill: BackfillState,
    backfill_dirty: bool,
}

/// Estado en vivo compartido entre la canalización y la interfaz.
///
/// Es una caché de lo que hay en SQLite más el liveness del registro. La interfaz
/// pide un `snapshot` al conectarse y después solo recibe los cambios.
#[derive(Clone)]
pub struct LiveState {
    inner: Arc<Mutex<Inner>>,
    on_change: Arc<dyn Fn() + Send + Sync>,
}

impl LiveState {
    /// `on_change` se llama cada vez que hay algo nuevo que publicar.
    pub fn new(on_change: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            inner: Arc::default(),
            on_change: Arc::new(on_change),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn snapshot(&self) -> LiveDelta {
        let mut inner = self.lock();
        inner.dirty.clear();
        inner.removed.clear();
        inner.backfill_dirty = false;
        LiveDelta::Snapshot {
            sessions: inner.sessions.values().cloned().collect(),
            backfill: inner.backfill.clone(),
            generated_at: now_ms(),
        }
    }

    /// Cambios desde la última llamada, o `None` si no hay ninguno.
    pub fn take_update(&self) -> Option<LiveDelta> {
        let mut inner = self.lock();
        if inner.dirty.is_empty() && inner.removed.is_empty() && !inner.backfill_dirty {
            return None;
        }
        let dirty = std::mem::take(&mut inner.dirty);
        let upserts = dirty
            .iter()
            .filter_map(|id| inner.sessions.get(id).cloned())
            .collect();
        let removed = std::mem::take(&mut inner.removed).into_iter().collect();
        let backfill = std::mem::take(&mut inner.backfill_dirty).then(|| inner.backfill.clone());
        Some(LiveDelta::Update {
            upserts,
            removed,
            backfill,
            generated_at: now_ms(),
        })
    }

    fn upsert(&self, view: SessionView) {
        {
            let mut inner = self.lock();
            if inner.sessions.get(&view.summary.id) == Some(&view) {
                return;
            }
            inner.removed.remove(&view.summary.id);
            inner.dirty.insert(view.summary.id.clone());
            inner.sessions.insert(view.summary.id.clone(), view);
        }
        (self.on_change)();
    }

    fn remove(&self, id: &str) {
        {
            let mut inner = self.lock();
            if inner.sessions.remove(id).is_none() {
                return;
            }
            inner.dirty.remove(id);
            inner.removed.insert(id.to_owned());
        }
        (self.on_change)();
    }

    fn set_backfill(&self, backfill: BackfillState) {
        {
            let mut inner = self.lock();
            if inner.backfill == backfill {
                return;
            }
            inner.backfill = backfill;
            inner.backfill_dirty = true;
        }
        (self.on_change)();
    }

    fn session_ids(&self) -> Vec<String> {
        self.lock().sessions.keys().cloned().collect()
    }
}

#[derive(Debug, Clone)]
pub struct LiveConfig {
    /// `~/.claude/projects`. Solo lectura.
    pub projects_root: PathBuf,
    /// `~/.claude/sessions`. Solo lectura. `None` desactiva la señal de liveness.
    pub sessions_dir: Option<PathBuf>,
    /// `%APPDATA%\dev-cockpit\cockpit.db`.
    pub db_path: PathBuf,
    pub policy: PrivacyPolicy,
}

enum Msg {
    Fs(PathBuf),
    Rescan,
    Stop,
}

/// Canalización en marcha. Al soltarla se detiene el hilo.
pub struct LivePipeline {
    tx: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
}

impl LivePipeline {
    pub fn spawn(config: LiveConfig, state: LiveState) -> io::Result<Self> {
        if let Some(dir) = config.db_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let store = Store::open(&config.db_path).map_err(io::Error::other)?;
        let (tx, rx) = mpsc::channel();
        let watcher = start_watcher(&config, tx.clone());
        let thread = thread::Builder::new()
            .name("cockpit-live".to_owned())
            .spawn(move || {
                let mut worker = Worker {
                    config,
                    state,
                    store,
                    probe: PidProbe::new(),
                    registry: BTreeMap::new(),
                    active_files: HashMap::new(),
                    // El watcher vive lo mismo que el hilo.
                    _watcher: watcher,
                };
                worker.run(&rx);
            })?;
        Ok(Self {
            tx,
            thread: Some(thread),
        })
    }
}

impl Drop for LivePipeline {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Si `notify` no arranca, la canalización sigue con los sondeos de respaldo.
fn start_watcher(config: &LiveConfig, tx: Sender<Msg>) -> Option<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let _ = match res {
            Ok(event) if event.need_rescan() => tx.send(Msg::Rescan),
            Ok(event) => event
                .paths
                .into_iter()
                .try_for_each(|p| tx.send(Msg::Fs(p))),
            Err(_) => tx.send(Msg::Rescan),
        };
    })
    .ok()?;
    watcher
        .watch(&config.projects_root, RecursiveMode::Recursive)
        .ok()?;
    if let Some(dir) = &config.sessions_dir {
        // Puede no existir todavía: la comprobación periódica lo cubre.
        let _ = watcher.watch(dir, RecursiveMode::NonRecursive);
    }
    Some(watcher)
}

struct Worker {
    config: LiveConfig,
    state: LiveState,
    store: Store,
    probe: PidProbe,
    registry: BTreeMap<String, RegistryEntry>,
    /// JSONL con cambios recientes y cuándo se vio el último.
    active_files: HashMap<PathBuf, Instant>,
    _watcher: Option<RecommendedWatcher>,
}

impl Worker {
    fn run(&mut self, rx: &Receiver<Msg>) {
        self.reload_registry();
        self.seed();
        self.backfill();

        let mut last_poll = Instant::now();
        let mut last_pid_check = Instant::now();
        let mut last_rescan = Instant::now();
        loop {
            // Sin ficheros activos no hay sondeo de 2 s: el hilo duerme hasta 30 s.
            let wait = if self.active_files.is_empty() {
                PID_CHECK
            } else {
                ACTIVE_POLL
            };
            let mut transcripts = BTreeSet::new();
            let mut registry_changed = false;
            let mut rescan = false;
            match rx.recv_timeout(wait) {
                Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Ok(msg) => {
                    let mut pending = Some(msg);
                    let deadline = Instant::now() + DEBOUNCE;
                    while let Some(msg) = pending.take() {
                        match msg {
                            Msg::Stop => return,
                            Msg::Rescan => rescan = true,
                            Msg::Fs(path) => {
                                if self.is_registry_path(&path) {
                                    registry_changed = true;
                                } else if path.extension().is_some_and(|e| e == "jsonl") {
                                    transcripts.insert(path);
                                }
                            }
                        }
                        let left = deadline.saturating_duration_since(Instant::now());
                        pending = rx.recv_timeout(left).ok();
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }

            let now = Instant::now();
            if now.duration_since(last_poll) >= ACTIVE_POLL {
                last_poll = now;
                self.active_files
                    .retain(|_, seen| now.duration_since(*seen) < ACTIVE_FILE_WINDOW);
                transcripts.extend(self.active_files.keys().cloned());
            }
            if rescan || now.duration_since(last_rescan) >= RESCAN {
                last_rescan = now;
                self.rescan();
            }
            if !transcripts.is_empty() {
                self.ingest_paths(&transcripts);
            }
            if registry_changed || now.duration_since(last_pid_check) >= PID_CHECK {
                last_pid_check = now;
                self.reload_registry();
                self.prune();
            }
        }
    }

    fn is_registry_path(&self, path: &Path) -> bool {
        self.config
            .sessions_dir
            .as_deref()
            .is_some_and(|dir| path.parent() == Some(dir))
    }

    /// Sesiones que ya estaban en la base: las vivas y las de actividad reciente.
    fn seed(&mut self) {
        let mut ids: BTreeSet<String> = self.registry.keys().cloned().collect();
        if let Ok(recent) = self
            .store
            .sessions_active_since(now_ms() - RECENT_SESSION_MS)
        {
            ids.extend(recent);
        }
        self.refresh(&ids);
    }

    /// Backfill inicial, del fichero más reciente al más antiguo. Las sesiones se
    /// publican según se leen sus ficheros, sin esperar al final.
    fn backfill(&mut self) {
        let Ok(files) = scan_transcripts(&self.config.projects_root) else {
            return;
        };
        let mut progress = BackfillState {
            running: true,
            files_total: files.len() as u64,
            bytes_total: files.iter().map(|f| f.size).sum(),
            ..Default::default()
        };
        self.state.set_backfill(progress.clone());
        for file in &files {
            let mut stats = IngestStats::default();
            let _ = ingest_file(file, Some(&mut self.store), self.config.policy, &mut stats);
            progress.files_done += 1;
            progress.bytes_done += file.size;
            self.state.set_backfill(progress.clone());
            if !stats.touched_sessions.is_empty() {
                self.after_ingest(&stats);
            }
        }
        progress.running = false;
        self.state.set_backfill(progress);
    }

    fn rescan(&mut self) {
        let result = ingest_tree(
            &self.config.projects_root,
            Some(&mut self.store),
            self.config.policy,
            |_, _| {},
        );
        if let Ok(stats) = result {
            self.after_ingest(&stats);
        }
    }

    fn ingest_paths(&mut self, paths: &BTreeSet<PathBuf>) {
        let mut stats = IngestStats::default();
        for path in paths {
            let Some(file) = TranscriptFile::from_path(path) else {
                self.active_files.remove(path);
                continue;
            };
            let before = stats.bytes_read;
            let result: Result<(), IngestFailure> =
                ingest_file(&file, Some(&mut self.store), self.config.policy, &mut stats);
            if result.is_ok() && stats.bytes_read > before {
                self.active_files.insert(path.clone(), Instant::now());
            }
        }
        self.after_ingest(&stats);
    }

    fn after_ingest(&mut self, stats: &IngestStats) {
        // Solo interesan las sesiones vivas o con actividad reciente.
        let cutoff = now_ms() - RECENT_SESSION_MS;
        let known: BTreeSet<String> = self.state.session_ids().into_iter().collect();
        let recent: BTreeSet<String> = self
            .store
            .sessions_active_since(cutoff)
            .unwrap_or_default()
            .into_iter()
            .collect();
        let ids: BTreeSet<String> = stats
            .touched_sessions
            .iter()
            .filter(|id| {
                known.contains(*id) || recent.contains(*id) || self.registry.contains_key(*id)
            })
            .cloned()
            .collect();
        self.refresh(&ids);
    }

    fn reload_registry(&mut self) {
        let Some(dir) = self.config.sessions_dir.clone() else {
            return;
        };
        let registry = read_registry(&dir, &mut self.probe);
        if registry == self.registry {
            return;
        }
        let changed: BTreeSet<String> = registry
            .keys()
            .chain(self.registry.keys())
            .filter(|id| registry.get(*id) != self.registry.get(*id))
            .cloned()
            .collect();
        self.registry = registry;
        self.refresh(&changed);
    }

    /// Vuelve a leer de SQLite el resumen de cada sesión y lo publica.
    fn refresh(&mut self, ids: &BTreeSet<String>) {
        let now = now_ms();
        for id in ids {
            let entry = self.registry.get(id);
            let summary = match self.store.session_summary(id, now - RUNNING_TOOL_MS) {
                Ok(Some(summary)) => summary,
                // Sesión recién abierta: existe en el registro y aún no tiene transcript.
                Ok(None) => match entry {
                    Some(entry) => placeholder(entry, self.config.policy, now),
                    None => {
                        self.state.remove(id);
                        continue;
                    }
                },
                Err(_) => continue,
            };
            if entry.is_none() && summary.last_seen_at < now - RECENT_SESSION_MS {
                self.state.remove(id);
                continue;
            }
            let mut summary = summary;
            if entry.is_none() {
                // Sin proceso vivo no puede haber nada en curso.
                summary.running_tools.clear();
            }
            self.state.upsert(SessionView {
                summary,
                live: entry.map(Liveness::from),
            });
        }
    }

    /// Retira las sesiones sin proceso vivo y sin actividad reciente.
    fn prune(&mut self) {
        let ids: BTreeSet<String> = self.state.session_ids().into_iter().collect();
        self.refresh(&ids);
    }
}

fn placeholder(entry: &RegistryEntry, policy: PrivacyPolicy, now: i64) -> SessionSummary {
    let started = entry.started_at.unwrap_or(now);
    SessionSummary {
        id: entry.session_id.clone(),
        cwd: entry.cwd.clone().filter(|_| policy.record_paths),
        entrypoint: entry.entrypoint.clone(),
        engine_version: entry.version.clone(),
        started_at: started,
        last_seen_at: entry.updated_at.unwrap_or(started),
        model: None,
        usage: TokenUsage::default(),
        api_messages: 0,
        tool_calls: 0,
        tool_errors: 0,
        api_errors: 0,
        cost_state_usd: None,
        running_tools: Vec::new(),
        recent_tools: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const SESSION: &str = "22222222-2222-4222-8222-222222222222";

    fn iso(ms: i64) -> String {
        chrono_like(ms)
    }

    /// RFC 3339 en UTC sin depender de `chrono` en este crate.
    fn chrono_like(ms: i64) -> String {
        let secs = ms.div_euclid(1000);
        let millis = ms.rem_euclid(1000);
        let days = secs.div_euclid(86_400);
        let rem = secs.rem_euclid(86_400);
        // Algoritmo civil de Howard Hinnant.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
            rem / 3600,
            rem % 3600 / 60,
            rem % 60
        )
    }

    fn line(n: u32, ts: i64) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"{SESSION}","timestamp":"{}","version":"2.1.321","cwd":"C:\\w\\repo","message":{{"id":"msg_{n}","model":"claude-test","usage":{{"input_tokens":1,"output_tokens":10}},"content":[{{"type":"tool_use","id":"toolu_{n}","name":"Read","input":{{"file_path":"C:\\w\\repo\\a.rs"}}}}]}}}}"#,
            iso(ts)
        ) + "\n"
    }

    fn append(path: &Path, text: &str) {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    fn wait_for<T>(mut probe: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(v) = probe() {
                return v;
            }
            assert!(
                Instant::now() < deadline,
                "la canalización no ha reaccionado en 10 s"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn sessions(state: &LiveState) -> Vec<SessionView> {
        match state.snapshot() {
            LiveDelta::Snapshot { sessions, .. } => sessions,
            LiveDelta::Update { .. } => unreachable!(),
        }
    }

    #[test]
    fn take_update_reports_only_what_changed() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let state = LiveState::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(state.take_update(), None);

        let entry = RegistryEntry {
            pid: 1,
            session_id: "s1".to_owned(),
            cwd: None,
            status: None,
            kind: None,
            entrypoint: None,
            version: None,
            started_at: Some(5),
            updated_at: None,
        };
        let view = SessionView {
            summary: placeholder(&entry, PrivacyPolicy::APP_DEFAULT, 5),
            live: Some(Liveness::from(&entry)),
        };
        state.upsert(view.clone());
        state.upsert(view.clone()); // idéntica: no cuenta como cambio
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        match state.take_update().unwrap() {
            LiveDelta::Update {
                upserts,
                removed,
                backfill,
                ..
            } => {
                assert_eq!(upserts, [view]);
                assert!(removed.is_empty());
                assert_eq!(backfill, None);
            }
            LiveDelta::Snapshot { .. } => unreachable!(),
        }
        assert_eq!(state.take_update(), None);

        state.remove("s1");
        match state.take_update().unwrap() {
            LiveDelta::Update {
                upserts, removed, ..
            } => {
                assert!(upserts.is_empty());
                assert_eq!(removed, ["s1"]);
            }
            LiveDelta::Snapshot { .. } => unreachable!(),
        }
    }

    #[test]
    fn appended_transcript_lines_reach_the_live_state() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        let registry = dir.path().join("sessions");
        let project = projects.join("C--w-repo");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&registry).unwrap();
        let transcript = project.join(format!("{SESSION}.jsonl"));

        let now = now_ms();
        // Una sesión antigua que ya estaba en disco: no debe salir en la vista.
        append(
            &project.join("33333333-3333-4333-8333-333333333333.jsonl"),
            &line(99, now - 3 * RECENT_SESSION_MS)
                .replace(SESSION, "33333333-3333-4333-8333-333333333333"),
        );
        append(&transcript, &line(1, now - 1000));

        let state = LiveState::new(|| {});
        let _pipeline = LivePipeline::spawn(
            LiveConfig {
                projects_root: projects,
                sessions_dir: Some(registry.clone()),
                db_path: dir.path().join("db").join("cockpit.db"),
                policy: PrivacyPolicy::APP_DEFAULT,
            },
            state.clone(),
        )
        .unwrap();

        // 1. El backfill inicial trae la sesión reciente, sin proceso vivo.
        let first = wait_for(|| {
            sessions(&state)
                .into_iter()
                .find(|s| s.summary.tool_calls == 1)
        });
        assert_eq!(first.summary.id, SESSION);
        assert_eq!(first.live, None);
        assert!(first.summary.running_tools.is_empty());
        assert_eq!(sessions(&state).len(), 1);

        // 2. El registro la marca como viva: ahora sí hay una herramienta en curso.
        let me = std::process::id();
        fs::write(
            registry.join(format!("{me}.json")),
            format!(
                r#"{{"pid":{me},"sessionId":"{SESSION}","status":"busy","kind":"interactive"}}"#
            ),
        )
        .unwrap();
        let live = wait_for(|| sessions(&state).into_iter().find(|s| s.live.is_some()));
        assert_eq!(live.live.as_ref().unwrap().status.as_deref(), Some("busy"));
        assert_eq!(live.summary.running_tools.len(), 1);

        // 3. Una línea nueva llega sin reiniciar nada.
        append(&transcript, &line(2, now));
        let updated = wait_for(|| {
            sessions(&state)
                .into_iter()
                .find(|s| s.summary.tool_calls == 2)
        });
        assert_eq!(updated.summary.usage.output, 20);
        assert_eq!(
            updated.summary.recent_tools[0].rel_path.as_deref(),
            Some("a.rs")
        );
    }
}
