//! `WorkspaceWatcher` (SPEC §7): raíces activas, Git por raíz y atribución.
//!
//! - **Raíces activas** (§7.1): el `git rev-parse --show-toplevel` del `cwd` de
//!   cada sesión viva. Sin Git, el propio `cwd`. Una raíz se desactiva tras 10
//!   minutos sin sesiones vivas.
//! - **Git** (§7.2): un hilo por raíz, una consulta en vuelo, coalescencia de
//!   500 ms tras el último evento. Nunca se sondea.
//! - **Atribución** (§7.3): provisional durante 2 s, reevaluada con cada
//!   herramienta de la raíz, y después definitiva.
//!
//! Todo lo que escribe pasa por el `Store` del hilo de la canalización.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use cockpit_core::PATH_DETAIL_DAYS;
use cockpit_core::attribution::{
    Attribution, Candidate, Observed, SETTLE_MS, attribute, should_replace, tool_class,
};
use cockpit_core::project_map::manifest_kind;
use cockpit_core::time::local_day_minus;
use cockpit_core::workspace::{FileChange, root_relative};
use cockpit_store::{FileEventView, NewProject, Store};
use serde::Serialize;

use crate::git::{GitReader, GitStatus, LineCounts, UNTRACKED_ALL_LIMIT, is_safe_rel_path};
use crate::watcher::{RootWatcher, WatchEvent, WatchSink, is_network_path};

/// Una raíz sin sesiones vivas se desactiva tras este tiempo (§7.1).
pub const ROOT_IDLE: Duration = Duration::from_secs(600);
/// Coalescencia de Git tras el último evento de fichero (§7.2).
pub const GIT_COALESCE: Duration = Duration::from_millis(500);
/// Con actividad continua, ningún cambio espera más que esto a su consulta de Git,
/// contado desde el cambio (B7: `numstat` < 1,5 s). Medido en F4a: con 2 s desde
/// el aviso, el p95 por cambio era de 2,4 s.
pub const GIT_MAX_WAIT: Duration = Duration::from_millis(1_000);
/// Vista previa de ficheros sin seguimiento (§7.2).
pub const PREVIEW_BYTES: usize = 256 * 1024;
/// Tope de ficheros con cambios que se publican por raíz.
const MAX_DIRTY_FILES: usize = 1000;
/// Un `unknown` definitivo se puede subir a R1/R2 durante este tiempo (§7.3).
const UPGRADE_WINDOW_MS: i64 = 30 * 60 * 1000;
/// Una atribución no se asienta antes de que la vista la haya podido mostrar.
const MIN_PROVISIONAL_MS: i64 = 500;

/// Lo que los hilos del Watcher, de Git y del resolutor entregan a la canalización.
#[derive(Debug)]
pub enum WorkspaceMsg {
    Watch {
        root_id: i64,
        event: WatchEvent,
    },
    Git {
        root_id: i64,
        snapshot: GitSnapshot,
    },
    Resolved {
        key: String,
        root: Option<ResolvedRoot>,
    },
}

pub type Sink = Arc<dyn Fn(WorkspaceMsg) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct GitSnapshot {
    pub status: Result<GitStatus, String>,
    pub numstat: BTreeMap<String, LineCounts>,
    /// Hora del último cambio de fichero incluido en esta consulta (B7).
    pub based_on_ts: i64,
    pub finished_at: i64,
    pub took_ms: i64,
}

/// Raíz de un `cwd`, resuelta con Git o, sin Git, el propio `cwd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRoot {
    pub root: PathBuf,
    /// `cwd` relativo a la raíz (`sub/dir/`), para traducir las rutas de las herramientas.
    pub prefix: String,
    pub is_git: bool,
    pub git_dir: Option<PathBuf>,
    pub git_common_dir: Option<PathBuf>,
    pub is_worktree: bool,
    pub main_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchState {
    Active,
    /// Unidad de red: §7.1 desactiva el watcher.
    Network,
    /// Raíz sin Git demasiado amplia (unidad entera, carpeta del usuario).
    TooBroad,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DirtyFile {
    pub rel_path: String,
    pub from_rel_path: Option<String>,
    pub status: String,
    pub staged: bool,
    pub added: Option<u64>,
    pub deleted: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GitView {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub files: Vec<DirtyFile>,
    pub files_total: usize,
    pub based_on_ts: i64,
    pub updated_at: i64,
    pub took_ms: i64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BulkView {
    /// `storm` (más de 500 eventos/s) o `rescan` (el watcher perdió eventos).
    pub kind: &'static str,
    pub dir: String,
    pub events: usize,
    pub at: i64,
}

/// Una raíz tal como la pinta la vista «Cambios en directo».
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkspaceView {
    pub root_id: i64,
    pub root: String,
    pub name: String,
    pub is_git: bool,
    pub is_worktree: bool,
    pub watch: WatchState,
    pub sessions: Vec<String>,
    pub git: Option<GitView>,
    pub bulk: Option<BulkView>,
}

/// Lo que el gestor publica; `LiveState` lo implementa.
pub trait WorkspacePublisher {
    fn upsert_workspace(&self, view: WorkspaceView);
    fn remove_workspace(&self, root_id: i64);
    fn push_file_events(&self, events: Vec<FileEventView>);
}

/// Quita el prefijo `\\?\` que añade `canonicalize` en Windows.
fn plain_path(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    p
}

/// Clave de caché de un `cwd`: Windows no distingue mayúsculas ni separadores.
pub fn cwd_key(cwd: &str) -> String {
    cwd.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// Resuelve la raíz de un `cwd` (§7.1). `None` si el directorio ya no existe.
pub fn resolve_cwd(git: &GitReader, cwd: &Path) -> Option<ResolvedRoot> {
    if !cwd.is_dir() {
        return None;
    }
    match git.resolve(cwd) {
        Ok(Some(info)) => {
            let canon =
                |p: &Path| plain_path(fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()));
            Some(ResolvedRoot {
                is_worktree: info.is_worktree(),
                main_root: info.main_root().map(|p| canon(&p)),
                root: canon(&info.toplevel),
                prefix: info.prefix.clone(),
                is_git: true,
                git_dir: Some(canon(&info.git_dir)),
                git_common_dir: Some(canon(&info.common_dir)),
            })
        }
        Ok(None) => Some(ResolvedRoot {
            root: plain_path(fs::canonicalize(cwd).ok()?),
            prefix: String::new(),
            is_git: false,
            git_dir: None,
            git_common_dir: None,
            is_worktree: false,
            main_root: None,
        }),
        Err(_) => None,
    }
}

/// Una raíz sin Git que no se vigila: la unidad entera o la carpeta del usuario
/// (o una que la contenga). Sin `.gitignore`, sería vigilar medio disco.
fn too_broad(root: &Path) -> bool {
    if root.parent().is_none() {
        return true;
    }
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    home.is_some_and(|h| {
        let (h, r) = (
            h.to_string_lossy().to_lowercase(),
            root.to_string_lossy().to_lowercase(),
        );
        h == r || h.starts_with(&format!("{}\\", r.trim_end_matches('\\')))
    })
}

/// Vista previa de un fichero sin seguimiento: primeros 256 KB, en memoria.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FilePreview {
    pub text: String,
    pub truncated: bool,
    pub binary: bool,
    pub size: u64,
}

/// Lee la vista previa sin salir de la raíz (ni por `..` ni por enlaces).
pub fn read_preview(root: &Path, rel_path: &str) -> std::io::Result<FilePreview> {
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidInput, "ruta no válida");
    if !is_safe_rel_path(rel_path) {
        return Err(bad());
    }
    let root = fs::canonicalize(root)?;
    let path = fs::canonicalize(root.join(rel_path))?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(bad());
    }
    let mut file = fs::File::open(&path)?;
    let size = file.metadata()?.len();
    let mut buf = Vec::with_capacity(PREVIEW_BYTES.min(size as usize));
    file.by_ref()
        .take(PREVIEW_BYTES as u64)
        .read_to_end(&mut buf)?;
    let binary = buf.iter().take(8192).any(|b| *b == 0);
    Ok(FilePreview {
        text: if binary {
            String::new()
        } else {
            String::from_utf8_lossy(&buf).into_owned()
        },
        truncated: size > PREVIEW_BYTES as u64,
        binary,
        size,
    })
}

/// Hilo de Git de una raíz: una consulta en vuelo y coalescencia (§7.2).
struct GitWorker {
    tx: Sender<i64>,
}

impl GitWorker {
    fn spawn(root_id: i64, root: PathBuf, sink: Sink) -> Option<Self> {
        let (tx, rx) = mpsc::channel::<i64>();
        thread::Builder::new()
            .name(format!("cockpit-git-{root_id}"))
            .spawn(move || {
                let git = GitReader::new();
                // Con muchos ficheros versionados, los no seguidos no se listan uno a uno.
                let untracked_all = git
                    .count_tracked(&root)
                    .map_or(true, |n| n <= UNTRACKED_ALL_LIMIT);
                while let Ok(first) = rx.recv() {
                    let now = crate::live::now_ms();
                    let mut latest = first;
                    // El tope se cuenta desde el primer cambio pendiente, no desde que
                    // se recibe el aviso: los cambios que llegaron durante la consulta
                    // anterior no esperan una ventana entera más (B7).
                    let earliest = if first > 0 { first.min(now) } else { now };
                    let deadline = earliest + GIT_MAX_WAIT.as_millis() as i64;
                    loop {
                        let left = deadline - crate::live::now_ms();
                        if left <= 0 {
                            break;
                        }
                        let wait = GIT_COALESCE.min(Duration::from_millis(left as u64));
                        match rx.recv_timeout(wait) {
                            Ok(ts) => latest = latest.max(ts),
                            // 500 ms sin cambios, o el tope alcanzado.
                            Err(RecvTimeoutError::Timeout) => break,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    let t0 = Instant::now();
                    let status = git.status(&root, untracked_all).map_err(|e| e.to_string());
                    let numstat = match &status {
                        Ok(s) if s.head.is_some() => git.numstat(&root).unwrap_or_default(),
                        _ => BTreeMap::new(),
                    };
                    sink(WorkspaceMsg::Git {
                        root_id,
                        snapshot: GitSnapshot {
                            status,
                            numstat,
                            based_on_ts: latest,
                            finished_at: crate::live::now_ms(),
                            took_ms: t0.elapsed().as_millis() as i64,
                        },
                    });
                }
            })
            .ok()?;
        Some(Self { tx })
    }

    fn poke(&self, change_ts: i64) {
        let _ = self.tx.send(change_ts);
    }
}

struct ActiveRoot {
    _watcher: Option<RootWatcher>,
    git: Option<GitWorker>,
    view: WorkspaceView,
    last_live: Instant,
}

/// Gestor de raíces. Vive en el hilo de la canalización, que es el único escritor.
pub struct Workspaces {
    sink: Sink,
    resolver: Sender<(String, String)>,
    record_paths: bool,
    /// `retention.path_detail_days`: un cambio de un día más antiguo no se guarda.
    path_detail_days: u32,
    /// Clave de `cwd` → `(raíz, prefijo)`, o `None` si no se pudo resolver.
    resolved: HashMap<String, Option<(i64, String)>>,
    requested: HashSet<String>,
    /// Sesiones a las que asignar la raíz cuando se resuelva su `cwd`.
    waiting: HashMap<String, BTreeSet<String>>,
    infos: HashMap<i64, ResolvedRoot>,
    /// Sesión → `(raíz, prefijo)`.
    session_roots: HashMap<String, (i64, String)>,
    /// Sesiones vivas por raíz, según la última sincronización.
    live: BTreeMap<i64, BTreeSet<String>>,
    roots: BTreeMap<i64, ActiveRoot>,
    /// Cambios provisionales: id → hora a la que se asientan.
    unsettled: BTreeMap<i64, i64>,
}

impl Workspaces {
    /// Días de detalle de rutas (`retention.path_detail_days`, §6.4.1). Por defecto, 30.
    pub fn with_path_detail_days(mut self, days: u32) -> Self {
        self.path_detail_days = days;
        self
    }

    pub fn new(sink: Sink, record_paths: bool) -> Self {
        let (tx, rx) = mpsc::channel::<(String, String)>();
        let resolver_sink = sink.clone();
        let _ = thread::Builder::new()
            .name("cockpit-resolver".to_owned())
            .spawn(move || {
                let git = GitReader::new();
                while let Ok((key, cwd)) = rx.recv() {
                    let root = resolve_cwd(&git, Path::new(&cwd));
                    resolver_sink(WorkspaceMsg::Resolved { key, root });
                }
            });
        Self {
            sink,
            resolver: tx,
            record_paths,
            path_detail_days: PATH_DETAIL_DAYS,
            resolved: HashMap::new(),
            requested: HashSet::new(),
            waiting: HashMap::new(),
            infos: HashMap::new(),
            session_roots: HashMap::new(),
            live: BTreeMap::new(),
            roots: BTreeMap::new(),
            unsettled: BTreeMap::new(),
        }
    }

    fn request(&mut self, key: &str, cwd: &str) {
        if self.requested.insert(key.to_owned()) {
            let _ = self.resolver.send((key.to_owned(), cwd.to_owned()));
        }
    }

    /// Resuelve en segundo plano la raíz de todas las sesiones guardadas, para que
    /// el histórico se agregue por proyecto (los triggers recalculan lo agregado).
    pub fn resolve_history(&mut self, store: &Store) {
        let Ok(cwds) = store.session_cwds() else {
            return;
        };
        for (session, cwd) in cwds {
            let key = cwd_key(&cwd);
            self.waiting.entry(key.clone()).or_default().insert(session);
            self.request(&key, &cwd);
        }
    }

    /// Los cambios provisionales de una ejecución anterior se dan por definitivos.
    pub fn settle_leftovers(&mut self, store: &mut Store) {
        if let Ok(rows) = store.unsettled_file_events() {
            let ids: Vec<i64> = rows.iter().map(|r| r.0).collect();
            let _ = store.settle_file_events(&ids);
        }
    }

    pub fn root_of(&self, session_id: &str) -> Option<i64> {
        self.session_roots.get(session_id).map(|(r, _)| *r)
    }

    /// Asigna la raíz conocida a la sesión y a sus herramientas sin raíz.
    ///
    /// La raíz puede resolverse antes de que exista la fila de la sesión (el
    /// registro la ve antes que el transcript o el `hello`), así que se repite
    /// antes de cada reevaluación. Sin cambios pendientes, no escribe nada.
    pub fn assign_root(&self, session_id: &str, store: &mut Store) -> Option<i64> {
        let root = self.root_of(session_id)?;
        let _ = store.assign_session_root(session_id, root);
        Some(root)
    }

    /// Sesiones vivas con su `cwd`: activa sus raíces y desactiva las inactivas.
    pub fn sync_live(
        &mut self,
        live: &[(String, Option<String>)],
        store: &mut Store,
        out: &dyn WorkspacePublisher,
    ) {
        let mut by_root: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
        for (session, cwd) in live {
            let Some(cwd) = cwd else { continue };
            let key = cwd_key(cwd);
            match self.resolved.get(&key) {
                Some(Some((root, prefix))) => {
                    let (root, prefix) = (*root, prefix.clone());
                    // Idempotente: la fila de la sesión puede no existir la primera vez.
                    let _ = store.assign_session_root(session, root);
                    self.session_roots.insert(session.clone(), (root, prefix));
                    by_root.entry(root).or_default().insert(session.clone());
                }
                Some(None) => {}
                None => {
                    self.waiting
                        .entry(key.clone())
                        .or_default()
                        .insert(session.clone());
                    self.request(&key, cwd);
                }
            }
        }
        let now = Instant::now();
        for (root_id, sessions) in &by_root {
            self.activate(*root_id, out);
            if let Some(active) = self.roots.get_mut(root_id) {
                active.last_live = now;
                let list: Vec<String> = sessions.iter().cloned().collect();
                if active.view.sessions != list {
                    active.view.sessions = list;
                    out.upsert_workspace(active.view.clone());
                }
            }
        }
        let idle: Vec<i64> = self
            .roots
            .iter()
            .filter(|(id, r)| {
                !by_root.contains_key(id) && now.duration_since(r.last_live) >= ROOT_IDLE
            })
            .map(|(id, _)| *id)
            .collect();
        for id in idle {
            self.roots.remove(&id);
            out.remove_workspace(id);
        }
        // Las raíces que siguen activas sin sesiones vivas se muestran sin sesiones.
        for (id, active) in &mut self.roots {
            if !by_root.contains_key(id) && !active.view.sessions.is_empty() {
                active.view.sessions.clear();
                out.upsert_workspace(active.view.clone());
            }
        }
        self.live = by_root;
    }

    /// Resultado del hilo resolutor. `true` si hay que volver a sincronizar.
    pub fn on_resolved(
        &mut self,
        key: String,
        root: Option<ResolvedRoot>,
        store: &mut Store,
        now_ms: i64,
    ) -> bool {
        let waiting = self.waiting.remove(&key).unwrap_or_default();
        let Some(root) = root else {
            self.resolved.insert(key, None);
            return false;
        };
        let root_str = root.root.to_string_lossy().into_owned();
        let git_dir = root
            .git_dir
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        let main_root = root
            .main_root
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());
        let Ok(id) = store.upsert_project(
            &NewProject {
                root: &root_str,
                git_dir: git_dir.as_deref(),
                is_worktree: root.is_worktree,
                main_root: main_root.as_deref(),
            },
            now_ms,
        ) else {
            self.requested.remove(&key);
            return false;
        };
        self.resolved.insert(key, Some((id, root.prefix.clone())));
        for session in waiting {
            let _ = store.assign_session_root(&session, id);
            self.session_roots
                .insert(session, (id, root.prefix.clone()));
        }
        self.infos.insert(id, root);
        true
    }

    fn activate(&mut self, root_id: i64, out: &dyn WorkspacePublisher) {
        if self.roots.contains_key(&root_id) {
            return;
        }
        let Some(info) = self.infos.get(&root_id).cloned() else {
            return;
        };
        let mut watch = WatchState::Active;
        let mut watcher = None;
        if is_network_path(&info.root) {
            watch = WatchState::Network;
        } else if !info.is_git && too_broad(&info.root) {
            watch = WatchState::TooBroad;
        } else {
            let sink = self.sink.clone();
            let watch_sink: WatchSink =
                Arc::new(move |event| sink(WorkspaceMsg::Watch { root_id, event }));
            match RootWatcher::start(&info.root, info.git_common_dir.as_deref(), watch_sink) {
                Ok(w) => watcher = Some(w),
                Err(_) => watch = WatchState::Error,
            }
        }
        let git = info
            .is_git
            .then(|| GitWorker::spawn(root_id, info.root.clone(), self.sink.clone()))
            .flatten();
        if let Some(git) = &git {
            git.poke(0);
        }
        let root = info.root.to_string_lossy().into_owned();
        let view = WorkspaceView {
            root_id,
            name: info
                .root
                .file_name()
                .map_or_else(|| root.clone(), |n| n.to_string_lossy().into_owned()),
            root,
            is_git: info.is_git,
            is_worktree: info.is_worktree,
            watch,
            sessions: Vec::new(),
            git: None,
            bulk: None,
        };
        out.upsert_workspace(view.clone());
        self.roots.insert(
            root_id,
            ActiveRoot {
                _watcher: watcher,
                git,
                view,
                last_live: Instant::now(),
            },
        );
    }

    pub fn is_active(&self, root_id: i64) -> bool {
        self.roots.contains_key(&root_id)
    }

    pub fn active_roots(&self) -> Vec<i64> {
        self.roots.keys().copied().collect()
    }

    fn live_count(&self, root_id: i64) -> usize {
        self.live.get(&root_id).map_or(0, BTreeSet::len)
    }

    fn candidates(
        &self,
        store: &Store,
        root_id: i64,
        from: i64,
        to: i64,
        now: i64,
    ) -> Vec<Candidate> {
        store
            .attribution_candidates(root_id, from, to, now)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|t| {
                let class = tool_class(&t.tool)?;
                let prefix = self
                    .session_roots
                    .get(&t.session_id)
                    .map_or("", |(_, p)| p.as_str());
                // Sin `record_paths` no hay atribución por ruta: R1 pasa a R2 (§10).
                let rel_path = t
                    .rel_path
                    .as_deref()
                    .filter(|_| self.record_paths)
                    .and_then(|p| root_relative(prefix, p));
                Some(Candidate {
                    session_id: t.session_id,
                    tool_use_id: t.tool_use_id,
                    class,
                    started_at: t.started_at,
                    finished_at: t.finished_at,
                    rel_path,
                })
            })
            .collect()
    }

    /// Mensaje de un vigilante: cambios, metadatos de Git, ráfaga o reescaneo.
    pub fn on_watch(
        &mut self,
        root_id: i64,
        event: WatchEvent,
        store: &mut Store,
        out: &dyn WorkspacePublisher,
        now: i64,
    ) {
        let Some(active) = self.roots.get_mut(&root_id) else {
            return;
        };
        match event {
            WatchEvent::Changes(changes) => {
                let latest = changes.iter().map(|c| c.ts_ms).max().unwrap_or(now);
                if let Some(git) = &active.git {
                    git.poke(latest);
                }
                self.ingest(root_id, changes, store, out, now);
            }
            WatchEvent::GitMeta => {
                if let Some(git) = &active.git {
                    git.poke(now);
                }
            }
            WatchEvent::Bulk { dir, events } => {
                active.view.bulk = Some(BulkView {
                    kind: "storm",
                    dir,
                    events,
                    at: now,
                });
                out.upsert_workspace(active.view.clone());
                if let Some(git) = &active.git {
                    git.poke(now);
                }
            }
            WatchEvent::Rescan => {
                active.view.bulk = Some(BulkView {
                    kind: "rescan",
                    dir: String::new(),
                    events: 0,
                    at: now,
                });
                out.upsert_workspace(active.view.clone());
                if let Some(git) = &active.git {
                    git.poke(now);
                }
            }
        }
    }

    /// Atribuye y guarda cambios nuevos (provisionales).
    fn ingest(
        &mut self,
        root_id: i64,
        changes: Vec<FileChange>,
        store: &mut Store,
        out: &dyn WorkspacePublisher,
        now: i64,
    ) {
        let (Some(from), Some(to)) = (
            changes.iter().map(|c| c.ts_ms).min(),
            changes.iter().map(|c| c.ts_ms).max(),
        ) else {
            return;
        };
        // Un manifiesto que cambia invalida el mapa guardado de la raíz (§9).
        if touches_manifest(&changes) {
            let _ = store.mark_project_map_stale(root_id);
        }
        let candidates = self.candidates(store, root_id, from, to, now);
        let active = self.live_count(root_id);
        let rows: Vec<(FileChange, Attribution)> = changes
            .into_iter()
            .map(|c| {
                let a = attribute(
                    &Observed {
                        ts: c.ts_ms,
                        rel_path: Some(&c.rel_path),
                        from_rel_path: c.from_rel_path.as_deref(),
                    },
                    &candidates,
                    active,
                    now,
                );
                (c, a)
            })
            .collect();
        let cutoff = local_day_minus(now, self.path_detail_days);
        let Ok(ids) = store.insert_file_events(root_id, &rows, self.record_paths, &cutoff) else {
            return;
        };
        let mut inserted = Vec::new();
        for (id, (c, _)) in ids.iter().zip(&rows) {
            if let Some(id) = id {
                self.unsettled
                    .insert(*id, (c.ts_ms + SETTLE_MS).max(now + MIN_PROVISIONAL_MS));
                inserted.push(*id);
            }
        }
        if let Ok(views) = store.file_events_by_id(&inserted) {
            out.push_file_events(views);
        }
    }

    /// Reevalúa la atribución en las raíces donde ha habido herramientas nuevas.
    pub fn reevaluate(
        &mut self,
        roots: &BTreeSet<i64>,
        store: &mut Store,
        out: &dyn WorkspacePublisher,
        now: i64,
    ) {
        for &root_id in roots {
            let Ok(changes) = store.changes_to_reevaluate(root_id, now - UPGRADE_WINDOW_MS) else {
                continue;
            };
            let (Some(from), Some(to)) = (
                changes.iter().map(|c| c.ts).min(),
                changes.iter().map(|c| c.ts).max(),
            ) else {
                continue;
            };
            let candidates = self.candidates(store, root_id, from, to, now);
            let active = self.live_count(root_id);
            let mut changed = Vec::new();
            for ch in changes {
                let new = attribute(
                    &Observed {
                        ts: ch.ts,
                        rel_path: ch.rel_path.as_deref(),
                        from_rel_path: ch.from_rel_path.as_deref(),
                    },
                    &candidates,
                    active,
                    now,
                );
                let old = &ch.attribution;
                let same = old.actor == new.actor
                    && old.session_id == new.session_id
                    && old.tool_use_id == new.tool_use_id
                    && old.confidence == new.confidence;
                if !same
                    && should_replace(old, &new, ch.settled)
                    && store.update_file_event(ch.id, &new, false).is_ok()
                {
                    changed.push(ch.id);
                }
            }
            if !changed.is_empty()
                && let Ok(views) = store.file_events_by_id(&changed)
            {
                out.push_file_events(views);
            }
        }
    }

    /// Asienta las atribuciones vencidas. Devuelve cuándo vence la siguiente.
    pub fn settle_due(
        &mut self,
        store: &mut Store,
        out: &dyn WorkspacePublisher,
        now: i64,
    ) -> Option<i64> {
        let due: Vec<i64> = self
            .unsettled
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(id, _)| *id)
            .collect();
        if !due.is_empty() && store.settle_file_events(&due).is_ok() {
            for id in &due {
                self.unsettled.remove(id);
            }
            if let Ok(views) = store.file_events_by_id(&due) {
                out.push_file_events(views);
            }
        }
        self.unsettled.values().min().copied()
    }

    /// Resultado de Git de una raíz.
    pub fn on_git(&mut self, root_id: i64, snapshot: GitSnapshot, out: &dyn WorkspacePublisher) {
        let Some(active) = self.roots.get_mut(&root_id) else {
            return;
        };
        let view = match snapshot.status {
            Ok(status) => {
                let files_total = status.entries.len();
                let files = status
                    .entries
                    .into_iter()
                    .take(MAX_DIRTY_FILES)
                    .map(|e| {
                        let counts = snapshot.numstat.get(&e.rel_path);
                        DirtyFile {
                            added: counts.and_then(|c| c.added),
                            deleted: counts.and_then(|c| c.deleted),
                            rel_path: e.rel_path,
                            from_rel_path: e.from_rel_path,
                            status: e.status.to_owned(),
                            staged: e.staged,
                        }
                    })
                    .collect();
                GitView {
                    branch: status.branch,
                    head: status.head.map(|h| h.chars().take(10).collect()),
                    upstream: status.upstream,
                    ahead: status.ahead,
                    behind: status.behind,
                    files,
                    files_total,
                    based_on_ts: snapshot.based_on_ts,
                    updated_at: snapshot.finished_at,
                    took_ms: snapshot.took_ms,
                    error: None,
                }
            }
            Err(error) => GitView {
                branch: None,
                head: None,
                upstream: None,
                ahead: None,
                behind: None,
                files: Vec::new(),
                files_total: 0,
                based_on_ts: snapshot.based_on_ts,
                updated_at: snapshot.finished_at,
                took_ms: snapshot.took_ms,
                error: Some(error),
            },
        };
        active.view.git = Some(view);
        out.upsert_workspace(active.view.clone());
    }
}

/// Algún cambio crea, modifica, borra o renombra un manifiesto de §9.
fn touches_manifest(changes: &[FileChange]) -> bool {
    changes.iter().any(|c| {
        std::iter::once(c.rel_path.as_str())
            .chain(c.from_rel_path.as_deref())
            .any(|p| manifest_kind(p).is_some())
    })
}

#[cfg(test)]
mod tests {
    use cockpit_core::workspace::ChangeKind;

    use super::*;

    #[test]
    fn only_manifest_changes_invalidate_the_cached_map() {
        let change = |path: &str, from: Option<&str>| FileChange {
            rel_path: path.to_owned(),
            change: if from.is_some() {
                ChangeKind::Renamed
            } else {
                ChangeKind::Modified
            },
            from_rel_path: from.map(str::to_owned),
            size: None,
            ts_ms: 0,
        };
        assert!(!touches_manifest(&[change("src/main.rs", None)]));
        assert!(touches_manifest(&[
            change("src/main.rs", None),
            change("crates/core/Cargo.toml", None),
        ]));
        // Renombrar un manifiesto a otro nombre también lo quita del mapa.
        assert!(touches_manifest(&[change("old.txt", Some("package.json"))]));
        // Los de dependencias de terceros no cuentan.
        assert!(!touches_manifest(&[change(
            "node_modules/react/package.json",
            None
        )]));
    }

    #[test]
    fn cwd_keys_ignore_case_and_separators() {
        assert_eq!(cwd_key(r"C:\Users\X\Repo\"), cwd_key("c:/users/x/repo"));
    }

    #[test]
    fn broad_plain_roots_are_not_watched() {
        assert!(too_broad(Path::new(r"C:\")));
        if let Some(home) = std::env::var_os("USERPROFILE") {
            let home = PathBuf::from(home);
            assert!(too_broad(&home));
            assert!(too_broad(home.parent().unwrap()));
            assert!(!too_broad(&home.join("proyecto")));
        }
    }

    #[test]
    fn previews_stay_inside_the_root() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hola\n").unwrap();
        fs::write(dir.path().join("b.bin"), [1u8, 0, 2]).unwrap();
        let p = read_preview(dir.path(), "a.txt").unwrap();
        assert_eq!(p.text, "hola\n");
        assert!(!p.binary && !p.truncated);
        assert!(read_preview(dir.path(), "b.bin").unwrap().binary);
        assert!(read_preview(dir.path(), "../a.txt").is_err());
        assert!(read_preview(dir.path(), "no-existe").is_err());
    }

    #[test]
    fn resolves_plain_directories_without_git() {
        let dir = tempfile::tempdir().unwrap();
        let r = resolve_cwd(&GitReader::new(), dir.path()).unwrap();
        assert!(!r.is_git);
        assert!(!r.root.to_string_lossy().starts_with(r"\\?\"));
        assert_eq!(
            resolve_cwd(&GitReader::new(), &dir.path().join("no-existe")),
            None
        );
    }
}
