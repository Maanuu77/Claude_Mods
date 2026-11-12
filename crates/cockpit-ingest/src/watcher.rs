//! Vigilante de una raíz de proyecto (SPEC §7.1).
//!
//! Un watcher `notify` recursivo por raíz, con `notify-debouncer-full` (ventanas
//! de 200 ms, renombrados unidos, sin duplicados). El filtrado ocurre aquí, en el
//! hilo del debouncer, antes de llegar a la canalización:
//!
//! 1. exclusiones fijas (`node_modules`, `target`, `.git/`…);
//! 2. `.gitignore` de cada directorio, `.git/info/exclude` y el global (`ignore`);
//! 3. modo «cambio masivo» si la raíz supera 500 eventos/s.
//!
//! **Hallazgo de F4a (lectura del código de `notify` 8.2.2):** en Windows usa un
//! buffer de 16 KiB (no de 64 KiB), ningún camino emite `Rescan`, y un código de
//! error desconocido de `ReadDirectoryChangesW` deja de vigilar la carpeta sin
//! avisar. La sonda `overflow_probe` (20.000 ficheros a unos 5.000/s) no llegó a
//! desbordarlo: 40.000 de 40.000 eventos. La red de seguridad es el modo masivo y
//! el `git status` posterior, que es la verdad del estado del árbol.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cockpit_core::workspace::{ChangeKind, FileChange};
use file_id::FileId;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::event::{EventKind, ModifyKind, RenameMode};
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, DebouncedEvent, Debouncer, FileIdCache};

/// Ventana del debouncer (§7.1).
pub const DEBOUNCE: Duration = Duration::from_millis(200);
/// Por encima de este ritmo, la raíz pasa a modo «cambio masivo» (§7.1).
pub const STORM_EVENTS_PER_SEC: usize = 500;
/// Pausa del procesamiento fichero a fichero en modo masivo.
pub const STORM_PAUSE: Duration = Duration::from_secs(2);

/// Directorios que nunca se vigilan, en cualquier nivel (§7.1).
const EXCLUDED_DIRS: [&str; 6] = ["node_modules", "target", "dist", "build", ".venv", ".git"];

/// Lo que el vigilante entrega a la canalización.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    Changes(Vec<FileChange>),
    /// Ha cambiado `HEAD`, el índice o una referencia: hay que releer el estado.
    GitMeta,
    /// Ráfaga por encima del umbral: se pausa el detalle y se resincroniza.
    Bulk {
        dir: String,
        events: usize,
    },
    /// `notify` pidió un reescaneo, o hubo errores del watcher.
    Rescan,
}

pub type WatchSink = Arc<dyn Fn(WatchEvent) + Send + Sync>;

/// Caché de file-IDs que empareja cada `To` con el `From` inmediatamente anterior.
///
/// La `FileIdMap` recomendada recorre el árbol entero (siguiendo enlaces) al
/// vigilar la raíz y con cada directorio nuevo, y abre cada fichero para leer su
/// id: con 70.000 ficheros en `node_modules` (M6) es inasumible. En Windows,
/// `ReadDirectoryChangesW` entrega `RENAMED_OLD_NAME` y `RENAMED_NEW_NAME` juntos
/// y en orden, así que basta con emparejarlos por posición. G7 lo valida.
#[derive(Debug, Default)]
pub struct PairCache;

static SAME: FileId = FileId::LowRes {
    volume_serial_number: 0,
    file_index: 0,
};

impl FileIdCache for PairCache {
    fn cached_file_id(&self, _path: &Path) -> Option<impl AsRef<FileId>> {
        Some(&SAME)
    }
    fn add_path(&mut self, _path: &Path, _recursive_mode: RecursiveMode) {}
    fn remove_path(&mut self, _path: &Path) {}
}

/// Pila de reglas de `.gitignore`, cargadas bajo demanda por directorio.
pub struct IgnoreStack {
    root: PathBuf,
    /// `.gitignore` por directorio relativo (`""` es la raíz); `None` si no tiene.
    per_dir: HashMap<String, Option<Gitignore>>,
    exclude: Option<Gitignore>,
    global: Gitignore,
}

impl IgnoreStack {
    /// `git_common_dir` aloja `info/exclude` (en un worktree, el del repo principal).
    pub fn new(root: &Path, git_common_dir: Option<&Path>) -> Self {
        let exclude = git_common_dir.and_then(|dir| {
            let path = dir.join("info").join("exclude");
            path.is_file().then(|| build(root, &path))
        });
        let mut global = GitignoreBuilder::new(root);
        global.case_insensitive(true).ok();
        let (global, _) = global.build_global();
        Self {
            root: root.to_path_buf(),
            per_dir: HashMap::new(),
            exclude,
            global,
        }
    }

    /// Olvida el `.gitignore` de un directorio para releerlo.
    pub fn invalidate(&mut self, dir_rel: &str) {
        self.per_dir.remove(dir_rel);
    }

    fn dir_matcher(&mut self, dir_rel: &str) -> Option<&Gitignore> {
        let root = &self.root;
        self.per_dir
            .entry(dir_rel.to_owned())
            .or_insert_with(|| {
                let dir = root.join(dir_rel);
                let file = dir.join(".gitignore");
                file.is_file().then(|| build(&dir, &file))
            })
            .as_ref()
    }

    /// `true` si Git ignoraría `rel` (relativa a la raíz, con `/`).
    pub fn is_ignored(&mut self, rel: &str, is_dir: bool) -> bool {
        // Del directorio más profundo a la raíz: manda la regla más cercana.
        let mut dirs: Vec<&str> = rel.match_indices('/').map(|(i, _)| &rel[..i]).collect();
        dirs.reverse();
        dirs.push("");
        for dir in dirs {
            let sub = if dir.is_empty() {
                rel
            } else {
                &rel[dir.len() + 1..]
            };
            if let Some(m) = self.dir_matcher(dir) {
                let found = m.matched_path_or_any_parents(sub, is_dir);
                if found.is_ignore() {
                    return true;
                }
                if found.is_whitelist() {
                    return false;
                }
            }
        }
        for m in self.exclude.iter().chain(std::iter::once(&self.global)) {
            let found = m.matched_path_or_any_parents(rel, is_dir);
            if found.is_ignore() {
                return true;
            }
            if found.is_whitelist() {
                return false;
            }
        }
        false
    }
}

fn build(base: &Path, file: &Path) -> Gitignore {
    let mut b = GitignoreBuilder::new(base);
    // Git en Windows trabaja con `core.ignorecase = true`.
    b.case_insensitive(true).ok();
    b.add(file);
    b.build().unwrap_or_else(|_| Gitignore::empty())
}

/// Qué es una ruta relativa dentro de la raíz.
#[derive(Debug, PartialEq, Eq)]
pub enum PathClass {
    /// `HEAD`, `index`, `packed-refs` o `refs/` de `.git`: cambian commits y ramas.
    GitMeta,
    /// Exclusión fija de §7.1.
    Excluded,
    Normal,
}

pub fn classify(rel: &str) -> PathClass {
    let lower = rel.to_ascii_lowercase();
    if let Some(inside) = lower.strip_prefix(".git/") {
        return if matches!(inside, "head" | "index" | "packed-refs") || inside.starts_with("refs/")
        {
            PathClass::GitMeta
        } else {
            PathClass::Excluded
        };
    }
    if lower == ".git" || lower.starts_with(".claude/worktrees/") || lower == ".claude/worktrees" {
        return PathClass::Excluded;
    }
    let segments: Vec<&str> = lower.split('/').collect();
    // Unity: `Library/` solo en la raíz del proyecto.
    if segments.len() > 1 && segments[0] == "library" {
        return PathClass::Excluded;
    }
    // Solo cuentan los directorios, no el nombre del fichero.
    let dirs = &segments[..segments.len().saturating_sub(1)];
    if dirs.iter().any(|s| EXCLUDED_DIRS.contains(s)) {
        return PathClass::Excluded;
    }
    PathClass::Normal
}

/// Ruta relativa a la raíz con `/`, o `None` si queda fuera.
pub fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = match path.strip_prefix(root) {
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => {
            // Windows no distingue mayúsculas: se compara sin ellas.
            let r = root.to_string_lossy().to_lowercase();
            let p = path.to_string_lossy();
            let head = p.get(..r.len())?;
            if head.to_lowercase() != r {
                return None;
            }
            let rest = &p[r.len()..];
            // `C:\w\repo2` no está dentro de `C:\w\repo`.
            if !rest.is_empty() && !rest.starts_with(['\\', '/']) {
                return None;
            }
            rest.to_owned()
        }
    };
    let rel = rel.replace('\\', "/");
    let rel = rel.trim_start_matches('/').to_owned();
    (!rel.is_empty()).then_some(rel)
}

fn wall_ms(at: Instant) -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    now - Instant::now().saturating_duration_since(at).as_millis() as i64
}

/// Temporal del guardado atómico de Claude Code: `<nombre>.tmp.<pid>.<hex>`.
///
/// Medido en F4a con Desktop 2.1.321: `Edit` y `Write` escriben ese temporal,
/// borran el original (si existe) y renombran el temporal sobre él.
pub fn is_transient(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    let mut parts = name.rsplitn(4, '.');
    let (Some(hex), Some(pid), Some("tmp"), Some(base)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    !base.is_empty()
        && hex.len() >= 6
        && hex.chars().all(|c| c.is_ascii_hexdigit())
        && !pid.is_empty()
        && pid.chars().all(|c| c.is_ascii_digit())
}

/// Un guardado atómico que sustituye a un fichero existente es una modificación.
///
/// NTFS conserva la fecha de creación del fichero sustituido («tunneling»): si
/// es anterior al cambio, el fichero ya existía; si no, es nuevo.
fn replaced_existing(path: &Path, ts_ms: i64) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.created())
        .ok()
        .and_then(|c| c.duration_since(UNIX_EPOCH).ok())
        .is_some_and(|c| (c.as_millis() as i64) < ts_ms - 2_000)
}

/// Estado del filtro: vive en el hilo del debouncer.
pub struct Filter {
    root: PathBuf,
    ignore: IgnoreStack,
    window_start: Instant,
    window_count: usize,
    storm_until: Option<Instant>,
    storm_events: usize,
    storm_dirs: BTreeMap<String, usize>,
}

impl Filter {
    pub fn new(root: &Path, git_common_dir: Option<&Path>) -> Self {
        Self {
            root: root.to_path_buf(),
            ignore: IgnoreStack::new(root, git_common_dir),
            window_start: Instant::now(),
            window_count: 0,
            storm_until: None,
            storm_events: 0,
            storm_dirs: BTreeMap::new(),
        }
    }

    /// `true` si la ruta es un fichero (no un directorio) que Git no ignora.
    fn keep(&mut self, rel: &str) -> bool {
        if self.root.join(rel).is_dir() {
            return false;
        }
        !self.ignore.is_ignored(rel, false)
    }

    /// Procesa un lote del debouncer y entrega lo que haya que entregar.
    pub fn handle(&mut self, events: &[DebouncedEvent], sink: &dyn Fn(WatchEvent)) {
        let now = Instant::now();
        let mut git_meta = false;
        let mut relevant: Vec<(&DebouncedEvent, Vec<String>)> = Vec::new();
        for ev in events {
            if ev.need_rescan() {
                sink(WatchEvent::Rescan);
                continue;
            }
            if matches!(
                ev.kind,
                EventKind::Access(_) | EventKind::Other | EventKind::Any
            ) {
                continue;
            }
            let rels: Vec<String> = ev
                .paths
                .iter()
                .filter_map(|p| relative(&self.root, p))
                .collect();
            let mut normal = Vec::new();
            for rel in rels {
                match classify(&rel) {
                    PathClass::GitMeta => git_meta = true,
                    PathClass::Excluded => {}
                    PathClass::Normal => normal.push(rel),
                }
            }
            // Un temporal del guardado atómico solo cuenta como origen de un renombrado.
            let is_rename = matches!(
                ev.kind,
                EventKind::Modify(ModifyKind::Name(RenameMode::Both))
            );
            if !is_rename && normal.iter().all(|r| is_transient(r)) {
                continue;
            }
            if !normal.is_empty() {
                relevant.push((ev, normal));
            }
        }
        if git_meta {
            sink(WatchEvent::GitMeta);
        }

        // Ritmo por ventanas de 1 s, contando lo que sobrevive a las exclusiones fijas.
        if now.duration_since(self.window_start) >= Duration::from_secs(1) {
            self.window_start = now;
            self.window_count = 0;
        }
        self.window_count += relevant.len();
        let storming = self.storm_until.is_some_and(|until| now < until);
        if self.window_count > STORM_EVENTS_PER_SEC || storming {
            if !storming {
                self.storm_events = 0;
                self.storm_dirs.clear();
            }
            if self.window_count > STORM_EVENTS_PER_SEC {
                self.storm_until = Some(now + STORM_PAUSE);
            }
            self.storm_events += relevant.len();
            for (_, rels) in &relevant {
                let top = rels[0].split('/').next().unwrap_or("").to_owned();
                *self.storm_dirs.entry(top).or_default() += 1;
            }
            let dir = self
                .storm_dirs
                .iter()
                .max_by_key(|(_, n)| **n)
                .map(|(d, _)| d.clone())
                .unwrap_or_default();
            sink(WatchEvent::Bulk {
                dir,
                events: self.storm_events,
            });
            return;
        }
        self.storm_until = None;

        // Un `.gitignore` que cambia invalida sus reglas antes de filtrar el lote.
        for (_, rels) in &relevant {
            for rel in rels {
                if rel == ".gitignore" || rel.ends_with("/.gitignore") {
                    let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
                    self.ignore.invalidate(dir);
                }
            }
        }

        // Rutas de este lote que son padres de otras: directorios borrados.
        let prefixes: Vec<String> = relevant
            .iter()
            .flat_map(|(_, r)| r.iter().map(|p| p.to_lowercase()))
            .collect();
        let is_parent = |rel: &str| {
            let prefix = format!("{}/", rel.to_lowercase());
            prefixes.iter().any(|o| o.starts_with(&prefix))
        };

        let mut changes = Vec::new();
        for (ev, rels) in relevant {
            let ts_ms = wall_ms(ev.time);
            let change = match ev.kind {
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if ev.paths.len() == 2 => {
                    // Origen y destino dentro de la raíz; cualquiera de los dos puede filtrarse.
                    let normal = |p: &Path| {
                        relative(&self.root, p).filter(|r| classify(r) == PathClass::Normal)
                    };
                    let (from, to) = (normal(&ev.paths[0]), normal(&ev.paths[1]));
                    let to = to.filter(|t| !is_transient(t));
                    let to_kept = to.clone().filter(|t| self.keep(t));
                    // Origen temporal, ignorado o igual al destino: el fichero se ha
                    // sustituido entero (guardado atómico), no renombrado.
                    let replaced = from.as_ref().is_none_or(|f| {
                        is_transient(f)
                            || self.ignore.is_ignored(f, false)
                            || to.as_ref().is_some_and(|t| t.eq_ignore_ascii_case(f))
                    });
                    let to_ignored = match &to {
                        None => true,
                        Some(t) => self.ignore.is_ignored(t, false),
                    };
                    match (from, to_kept) {
                        (_, Some(t)) if replaced => {
                            let kind = if replaced_existing(&self.root.join(&t), ts_ms) {
                                ChangeKind::Modified
                            } else {
                                ChangeKind::Created
                            };
                            Some((t, kind, None))
                        }
                        (Some(f), Some(t)) => Some((t, ChangeKind::Renamed, Some(f))),
                        (Some(f), None) if to_ignored && !is_transient(&f) => {
                            Some((f, ChangeKind::Deleted, None))
                        }
                        _ => None,
                    }
                }
                EventKind::Modify(ModifyKind::Name(RenameMode::From)) | EventKind::Remove(_) => {
                    let rel = &rels[0];
                    (!is_parent(rel) && !self.ignore.is_ignored(rel, false))
                        .then(|| (rel.clone(), ChangeKind::Deleted, None))
                }
                EventKind::Modify(ModifyKind::Name(_)) | EventKind::Create(_) => {
                    let rel = &rels[rels.len() - 1];
                    self.keep(rel)
                        .then(|| (rel.clone(), ChangeKind::Created, None))
                }
                EventKind::Modify(_) => {
                    // Windows avisa de que un directorio cambia cuando cambian sus hijos.
                    let rel = &rels[0];
                    self.keep(rel)
                        .then(|| (rel.clone(), ChangeKind::Modified, None))
                }
                _ => None,
            };
            if let Some((rel_path, change, from_rel_path)) = change {
                let size = match change {
                    ChangeKind::Deleted => None,
                    _ => std::fs::metadata(self.root.join(&rel_path))
                        .ok()
                        .map(|m| m.len()),
                };
                changes.push(FileChange {
                    rel_path,
                    change,
                    from_rel_path,
                    size,
                    ts_ms,
                });
            }
        }
        if !changes.is_empty() {
            let mut changes = collapse_same_path(changes, &self.root);
            pair_moves(&mut changes);
            changes.sort_by_key(|c| c.ts_ms);
            if !changes.is_empty() {
                sink(WatchEvent::Changes(changes));
            }
        }
    }
}

/// Un solo cambio por ruta y lote, según el estado final del fichero.
///
/// El guardado atómico (borrar el original y renombrar un temporal sobre él)
/// llega del debouncer como varios eventos de la misma ruta en el mismo lote.
fn collapse_same_path(changes: Vec<FileChange>, root: &Path) -> Vec<FileChange> {
    let mut groups: Vec<(String, Vec<FileChange>)> = Vec::new();
    for c in changes {
        let key = c.rel_path.to_lowercase();
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, g)) => g.push(c),
            None => groups.push((key, vec![c])),
        }
    }
    let mut out = Vec::with_capacity(groups.len());
    for (_, mut group) in groups {
        if group.len() == 1 {
            out.extend(group.pop());
            continue;
        }
        let ts_ms = group.iter().map(|c| c.ts_ms).min().unwrap_or(0);
        let last = group.pop().unwrap_or_else(|| unreachable!());
        let path = root.join(&last.rel_path);
        let exists = path.is_file();
        let renamed_from = group
            .iter()
            .chain(std::iter::once(&last))
            .find_map(|c| c.from_rel_path.clone())
            .filter(|f| !f.eq_ignore_ascii_case(&last.rel_path));
        let existed_before = group
            .iter()
            .chain(std::iter::once(&last))
            .any(|c| matches!(c.change, ChangeKind::Deleted | ChangeKind::Modified));
        let change = match (exists, &renamed_from) {
            (false, _) if group.iter().all(|c| c.change == ChangeKind::Created) => continue,
            (false, _) => ChangeKind::Deleted,
            (true, Some(_)) => ChangeKind::Renamed,
            (true, None) if existed_before || replaced_existing(&path, ts_ms) => {
                ChangeKind::Modified
            }
            (true, None) => ChangeKind::Created,
        };
        out.push(FileChange {
            from_rel_path: renamed_from.filter(|_| change == ChangeKind::Renamed),
            size: exists
                .then(|| std::fs::metadata(&path).ok().map(|m| m.len()))
                .flatten(),
            change,
            rel_path: last.rel_path,
            ts_ms,
        });
    }
    out
}

/// Distancia máxima entre el borrado y el creado de un mismo movimiento.
const MOVE_PAIR_MS: i64 = 20;

/// Une los movimientos entre carpetas que llegan como borrado + creado.
///
/// `ReadDirectoryChangesW` solo da `RENAMED_OLD/NEW_NAME` dentro de un mismo
/// directorio: un movimiento entre carpetas llega como `REMOVED` + `ADDED` en el
/// mismo buffer, y el debouncer no los une (G7). Aquí se unen solo si, en el
/// mismo lote, hay **un único** borrado y **un único** creado con ese nombre de
/// fichero y a menos de 20 ms. Un movimiento que además cambia el nombre se queda
/// como borrado + creado.
fn pair_moves(changes: &mut Vec<FileChange>) {
    let name = |c: &FileChange| c.rel_path.rsplit('/').next().unwrap_or("").to_lowercase();
    let mut counts: HashMap<(String, ChangeKind), usize> = HashMap::new();
    for c in changes.iter() {
        if matches!(c.change, ChangeKind::Deleted | ChangeKind::Created) {
            *counts.entry((name(c), c.change)).or_default() += 1;
        }
    }
    let unique = |n: &str, k: ChangeKind| counts.get(&(n.to_owned(), k)) == Some(&1);
    let mut gone = Vec::new();
    for i in 0..changes.len() {
        if changes[i].change != ChangeKind::Deleted {
            continue;
        }
        let n = name(&changes[i]);
        if !unique(&n, ChangeKind::Deleted) || !unique(&n, ChangeKind::Created) {
            continue;
        }
        let ts = changes[i].ts_ms;
        if let Some(j) = changes.iter().position(|c| {
            c.change == ChangeKind::Created && name(c) == n && (c.ts_ms - ts).abs() <= MOVE_PAIR_MS
        }) {
            changes[j].change = ChangeKind::Renamed;
            changes[j].from_rel_path = Some(changes[i].rel_path.clone());
            changes[j].ts_ms = changes[j].ts_ms.min(ts);
            gone.push(i);
        }
    }
    for i in gone.into_iter().rev() {
        changes.remove(i);
    }
}

/// Vigilante en marcha. Al soltarlo se detiene.
pub struct RootWatcher {
    _debouncer: Debouncer<RecommendedWatcher, PairCache>,
}

impl RootWatcher {
    pub fn start(
        root: &Path,
        git_common_dir: Option<&Path>,
        sink: WatchSink,
    ) -> notify::Result<Self> {
        let mut filter = Filter::new(root, git_common_dir);
        let handler_sink = sink.clone();
        let mut debouncer = notify_debouncer_full::new_debouncer_opt::<_, RecommendedWatcher, _>(
            DEBOUNCE,
            None,
            move |result: DebounceEventResult| match result {
                Ok(events) => filter.handle(&events, &|e| handler_sink(e)),
                Err(_) => handler_sink(WatchEvent::Rescan),
            },
            PairCache,
            notify::Config::default(),
        )?;
        debouncer.watch(root, RecursiveMode::Recursive)?;
        Ok(Self {
            _debouncer: debouncer,
        })
    }
}

/// `\\servidor\recurso`: §7.1 desactiva el watcher en unidades de red.
pub fn is_network_path(root: &Path) -> bool {
    let s = root.to_string_lossy();
    s.starts_with(r"\\") && !s.starts_with(r"\\?\")
        || s.to_ascii_uppercase().starts_with(r"\\?\UNC\")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;
    use std::sync::mpsc;

    #[test]
    fn fixed_exclusions_apply_to_directories_only() {
        assert_eq!(classify("node_modules/x/index.js"), PathClass::Excluded);
        assert_eq!(classify("web/node_modules/x.js"), PathClass::Excluded);
        assert_eq!(classify("crates/a/target/debug/x"), PathClass::Excluded);
        assert_eq!(classify("Library/x.asset"), PathClass::Excluded);
        assert_eq!(
            classify(".claude/worktrees/a/src/x.rs"),
            PathClass::Excluded
        );
        assert_eq!(classify(".git/objects/ab/cd"), PathClass::Excluded);
        assert_eq!(classify("sub/.git/config"), PathClass::Excluded);
        assert_eq!(classify(".git/HEAD"), PathClass::GitMeta);
        assert_eq!(classify(".git/refs/heads/main"), PathClass::GitMeta);
        assert_eq!(classify(".git/index"), PathClass::GitMeta);
        // Un fichero que se llama como un directorio excluido no se excluye.
        assert_eq!(classify("docs/build"), PathClass::Normal);
        assert_eq!(classify("src/Library/x.rs"), PathClass::Normal);
        assert_eq!(classify("src/main.rs"), PathClass::Normal);
    }

    #[test]
    fn gitignore_rules_nest_and_whitelist() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("sub/deep")).unwrap();
        fs::create_dir_all(root.join(".git/info")).unwrap();
        fs::write(root.join(".gitignore"), "*.log\nsecret/\n").unwrap();
        fs::write(root.join("sub/.gitignore"), "!keep.log\n*.tmp\n").unwrap();
        fs::write(root.join(".git/info/exclude"), "local.txt\n").unwrap();
        let mut stack = IgnoreStack::new(root, Some(&root.join(".git")));
        assert!(stack.is_ignored("a.log", false));
        assert!(
            stack.is_ignored("A.LOG", false),
            "sin distinguir mayúsculas"
        );
        assert!(stack.is_ignored("secret/x.rs", false));
        assert!(stack.is_ignored("sub/deep/x.tmp", false));
        assert!(!stack.is_ignored("sub/keep.log", false));
        assert!(stack.is_ignored("local.txt", false));
        assert!(!stack.is_ignored("src/main.rs", false));

        fs::write(root.join(".gitignore"), "").unwrap();
        stack.invalidate("");
        assert!(!stack.is_ignored("a.log", false));
    }

    #[test]
    fn relative_paths_ignore_case_and_use_slashes() {
        let root = Path::new(r"C:\W\Repo");
        assert_eq!(
            relative(root, Path::new(r"c:\w\repo\src\a.rs")).as_deref(),
            Some("src/a.rs")
        );
        assert_eq!(relative(root, Path::new(r"C:\W\Repo")), None);
        assert_eq!(relative(root, Path::new(r"C:\W\Repo2\a.rs")), None);
        assert!(is_network_path(Path::new(r"\\nas\share\repo")));
        assert!(!is_network_path(Path::new(r"C:\repo")));
    }

    fn collect(
        rx: &mpsc::Receiver<WatchEvent>,
        until: impl Fn(&[FileChange]) -> bool,
    ) -> Vec<FileChange> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut out = Vec::new();
        while Instant::now() < deadline {
            if let Ok(WatchEvent::Changes(c)) = rx.recv_timeout(Duration::from_millis(100)) {
                out.extend(c);
                if until(&out) {
                    break;
                }
            }
        }
        out
    }

    /// Con el watcher real de Windows: crear, modificar, renombrar y borrar.
    #[test]
    fn real_watcher_reports_filtered_changes_and_renames() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: WatchSink = Arc::new(move |e| {
            let _ = tx.lock().unwrap().send(e);
        });
        let _w = RootWatcher::start(&root, None, sink).unwrap();
        thread_sleep(100);

        fs::write(root.join("src/a.rs"), "a").unwrap();
        fs::write(root.join("ruido.log"), "x").unwrap();
        fs::write(root.join("node_modules/pkg/i.js"), "x").unwrap();
        let got = collect(&rx, |c| c.iter().any(|x| x.rel_path == "src/a.rs"));
        assert!(
            got.iter()
                .any(|c| c.rel_path == "src/a.rs" && c.change == ChangeKind::Created),
            "{got:?}"
        );
        assert!(
            got.iter()
                .all(|c| c.rel_path != "ruido.log" && !c.rel_path.starts_with("node_modules"))
        );

        thread_sleep(300);
        fs::rename(root.join("src/a.rs"), root.join("src/b.rs")).unwrap();
        let got = collect(&rx, |c| c.iter().any(|x| x.change == ChangeKind::Renamed));
        let renamed = got
            .iter()
            .find(|c| c.change == ChangeKind::Renamed)
            .expect("renombrado");
        assert_eq!(renamed.rel_path, "src/b.rs");
        assert_eq!(renamed.from_rel_path.as_deref(), Some("src/a.rs"));

        // Movimiento entre carpetas con el mismo nombre: Windows da borrado + creado.
        thread_sleep(300);
        fs::create_dir_all(root.join("otra")).unwrap();
        fs::rename(root.join("src/b.rs"), root.join("otra/b.rs")).unwrap();
        let got = collect(&rx, |c| c.iter().any(|x| x.rel_path == "otra/b.rs"));
        let moved = got
            .iter()
            .find(|c| c.rel_path == "otra/b.rs")
            .expect("movido");
        assert_eq!(moved.change, ChangeKind::Renamed, "{got:?}");
        assert_eq!(moved.from_rel_path.as_deref(), Some("src/b.rs"));

        // Movimiento que además cambia el nombre: se queda como borrado + creado.
        thread_sleep(300);
        fs::rename(root.join("otra/b.rs"), root.join("src/c.rs")).unwrap();
        let got = collect(&rx, |c| c.len() >= 2);
        assert!(
            got.iter()
                .any(|c| c.rel_path == "otra/b.rs" && c.change == ChangeKind::Deleted),
            "{got:?}"
        );
        assert!(
            got.iter()
                .any(|c| c.rel_path == "src/c.rs" && c.change == ChangeKind::Created),
            "{got:?}"
        );

        thread_sleep(300);
        fs::remove_file(root.join("src/c.rs")).unwrap();
        let got = collect(&rx, |c| c.iter().any(|x| x.change == ChangeKind::Deleted));
        assert!(
            got.iter()
                .any(|c| c.rel_path == "src/c.rs" && c.change == ChangeKind::Deleted)
        );
    }

    #[test]
    fn recognises_claude_code_temporaries() {
        assert!(is_transient("src/objetivo.txt.tmp.25984.82cb86f02ec0"));
        assert!(is_transient("a.rs.tmp.1.abcdef"));
        assert!(!is_transient("src/a.rs"));
        assert!(!is_transient("notas.tmp"));
        assert!(!is_transient("x.tmp.12.zz"));
        assert!(!is_transient(".tmp.12.abcdef12"));
    }

    /// Secuencia cruda que produce Claude Code (Desktop 2.1.321) en `Edit` y `Write`:
    /// temporal, borrado del original y renombrado del temporal sobre él.
    #[test]
    fn claude_code_atomic_saves_are_single_changes() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        fs::write(root.join("existente.rs"), "uno\n").unwrap();
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let sink: WatchSink = Arc::new(move |e| {
            let _ = tx.lock().unwrap().send(e);
        });
        let _w = RootWatcher::start(&root, None, sink).unwrap();
        // El original tiene que ser «viejo» para el tunneling de NTFS.
        thread_sleep(2_200);

        // Edit: fichero existente.
        let tmp = root.join("existente.rs.tmp.25984.82cb86f02ec0");
        fs::write(&tmp, "uno\ndos\n").unwrap();
        fs::remove_file(root.join("existente.rs")).unwrap();
        fs::rename(&tmp, root.join("existente.rs")).unwrap();
        let got = collect(&rx, |c| c.iter().any(|x| x.rel_path == "existente.rs"));
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].change, ChangeKind::Modified, "{got:?}");

        // Write: fichero nuevo.
        thread_sleep(300);
        let tmp = root.join("nuevo.rs.tmp.25984.8c30986bc5b7");
        fs::write(&tmp, "nuevo\n").unwrap();
        fs::rename(&tmp, root.join("nuevo.rs")).unwrap();
        let got = collect(&rx, |c| c.iter().any(|x| x.rel_path == "nuevo.rs"));
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].change, ChangeKind::Created, "{got:?}");
    }

    /// Sonda del desbordamiento de `ReadDirectoryChangesW` con `notify` 8.2.2:
    /// ¿llega un `Rescan` o se pierden eventos en silencio? Se ejecuta a mano:
    /// `cargo test -p cockpit-ingest --release --lib overflow_probe -- --ignored --nocapture`.
    #[test]
    #[ignore = "sonda manual de F4a (desbordamiento del buffer de notify)"]
    fn overflow_probe() {
        use notify::Watcher;
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let (tx, rx) = mpsc::channel();
        let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let _ = tx.send(res.map(|e| e.need_rescan()));
        })
        .unwrap();
        w.watch(&root, RecursiveMode::Recursive).unwrap();
        thread_sleep(200);
        const N: usize = 20_000;
        let t = Instant::now();
        for i in 0..N {
            fs::write(root.join(format!("f{i:05}.txt")), "x").unwrap();
        }
        let written = t.elapsed();
        thread_sleep(3_000);
        let (mut events, mut rescans, mut errors) = (0, 0, 0);
        while let Ok(r) = rx.try_recv() {
            match r {
                Ok(true) => rescans += 1,
                Ok(false) => events += 1,
                Err(_) => errors += 1,
            }
        }
        println!(
            "ficheros={N} escritos_en={written:?} eventos={events} rescan={rescans} errores={errors} \
             (sin pérdida serían ≥ {} eventos: crear + modificar)",
            N * 2
        );
    }

    /// Más de 500 eventos/s: se pausa el detalle y se avisa una vez por lote.
    #[test]
    fn a_burst_switches_to_bulk_mode() {
        use notify::event::CreateKind;
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let mut filter = Filter::new(&root, None);
        let batch: Vec<DebouncedEvent> = (0..600)
            .map(|i| {
                let ev = notify::Event::new(EventKind::Create(CreateKind::File))
                    .add_path(root.join(format!("gen/f{i}.js")));
                DebouncedEvent::new(ev, Instant::now())
            })
            .collect();
        let out = Mutex::new(Vec::new());
        filter.handle(&batch, &|e| out.lock().unwrap().push(e));
        let out = out.into_inner().unwrap();
        assert_eq!(
            out,
            [WatchEvent::Bulk {
                dir: "gen".to_owned(),
                events: 600
            }]
        );
        // Un lote pequeño dentro de la pausa sigue contando como ráfaga.
        let small = [DebouncedEvent::new(
            notify::Event::new(EventKind::Create(CreateKind::File)).add_path(root.join("src/a.rs")),
            Instant::now(),
        )];
        let out = Mutex::new(Vec::new());
        filter.handle(&small, &|e| out.lock().unwrap().push(e));
        assert!(matches!(
            out.into_inner().unwrap()[..],
            [WatchEvent::Bulk { events: 601, .. }]
        ));
    }

    fn fc(path: &str, change: ChangeKind, ts: i64) -> FileChange {
        FileChange {
            rel_path: path.to_owned(),
            change,
            from_rel_path: None,
            size: None,
            ts_ms: ts,
        }
    }

    #[test]
    fn moves_are_paired_only_when_unambiguous() {
        let mut c = vec![
            fc("a/x.rs", ChangeKind::Deleted, 100),
            fc("b/X.rs", ChangeKind::Created, 105),
        ];
        pair_moves(&mut c);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].change, ChangeKind::Renamed);
        assert_eq!(c[0].from_rel_path.as_deref(), Some("a/x.rs"));

        // Demasiado separados en el tiempo.
        let mut c = vec![
            fc("a/x.rs", ChangeKind::Deleted, 100),
            fc("b/x.rs", ChangeKind::Created, 200),
        ];
        pair_moves(&mut c);
        assert_eq!(c.len(), 2);

        // Ambiguo: dos creados con el mismo nombre.
        let mut c = vec![
            fc("a/x.rs", ChangeKind::Deleted, 100),
            fc("b/x.rs", ChangeKind::Created, 100),
            fc("c/x.rs", ChangeKind::Created, 100),
        ];
        pair_moves(&mut c);
        assert_eq!(c.len(), 3);
    }

    fn thread_sleep(ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}
