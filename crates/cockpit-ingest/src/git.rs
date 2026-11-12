//! `GitReader`: Git del sistema, **exclusivamente de solo lectura** (SPEC §7.2).
//!
//! Se usa el binario `git` y no `git2`/`gix` para respetar la configuración real
//! del usuario (autocrlf, filtros, LFS, `safe.directory`, worktrees). Cada
//! invocación lleva el endurecimiento de §7.2 y pasa por una lista blanca de
//! subcomandos aplicada aquí, en el código.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde::Serialize;

/// Subcomandos permitidos (§7.2). Ninguno escribe en el repositorio.
pub const ALLOWED_SUBCOMMANDS: [&str; 8] = [
    "status",
    "diff",
    "rev-parse",
    "ls-files",
    "log",
    "show",
    "worktree",
    "cat-file",
];

/// Opciones que podrían escribir o ejecutar algo aunque el subcomando sea de lectura.
/// Las opciones globales (`--git-dir`, `--exec-path`…) no pueden aparecer: los
/// argumentos siempre empiezan por un subcomando de la lista blanca.
const FORBIDDEN_ARGS: [&str; 3] = ["--output", "--ext-diff", "--textconv"];

/// Tope de un diff de fichero (§7.2).
pub const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
/// Tope de la salida de las consultas de estado.
const MAX_LIST_BYTES: usize = 32 * 1024 * 1024;
/// Con más ficheros versionados, `status` no lista los no seguidos uno a uno.
pub const UNTRACKED_ALL_LIMIT: usize = 20_000;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("subcomando de Git no permitido: {0}")]
    Forbidden(String),
    #[error("no se encuentra el ejecutable de Git")]
    NotInstalled,
    #[error("no se pudo lanzar Git: {0}")]
    Spawn(io::Error),
    #[error("Git terminó con código {0:?}")]
    Failed(Option<i32>),
    #[error("Git no respondió a tiempo")]
    Timeout,
    #[error("ruta no válida")]
    BadPath,
}

pub type Result<T> = std::result::Result<T, GitError>;

/// Raíz de un `cwd` según Git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    /// `--show-toplevel`: la raíz del árbol de trabajo (cada worktree es una raíz).
    pub toplevel: PathBuf,
    /// `--show-prefix`: el `cwd` relativo a la raíz (`sub/dir/` o vacío).
    pub prefix: String,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
}

impl RepoInfo {
    pub fn is_worktree(&self) -> bool {
        self.git_dir != self.common_dir
    }

    /// Raíz del repositorio principal de un worktree.
    pub fn main_root(&self) -> Option<PathBuf> {
        if !self.is_worktree() {
            return None;
        }
        (self.common_dir.file_name()? == ".git")
            .then(|| self.common_dir.parent().map(Path::to_path_buf))?
    }
}

/// Un fichero con cambios según `status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusEntry {
    pub rel_path: String,
    pub from_rel_path: Option<String>,
    /// `modified`, `added`, `deleted`, `renamed`, `copied`, `typechange`,
    /// `untracked` o `conflict`.
    pub status: &'static str,
    /// Hay cambios en el índice (preparados para commit).
    pub staged: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GitStatus {
    pub branch: Option<String>,
    /// `None` antes del primer commit.
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub entries: Vec<StatusEntry>,
}

/// `+`/`−` de un fichero respecto a `HEAD`. `None` si es binario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LineCounts {
    pub added: Option<u64>,
    pub deleted: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileDiff {
    pub text: String,
    pub truncated: bool,
    pub binary: bool,
}

struct Output {
    stdout: Vec<u8>,
    truncated: bool,
}

#[derive(Debug, Clone)]
pub struct GitReader {
    program: OsString,
}

impl Default for GitReader {
    fn default() -> Self {
        Self::new()
    }
}

impl GitReader {
    pub fn new() -> Self {
        Self {
            program: OsString::from("git"),
        }
    }

    /// Comprueba la lista blanca. Es pública para poder probarla.
    pub fn check(args: &[&str]) -> Result<()> {
        let Some(sub) = args.first() else {
            return Err(GitError::Forbidden(String::new()));
        };
        let forbidden = || GitError::Forbidden(args.join(" "));
        if !ALLOWED_SUBCOMMANDS.contains(sub) {
            return Err(forbidden());
        }
        // Formas exactas de §7.2 para los subcomandos que también tienen variantes de escritura.
        let ok = match *sub {
            "log" => args.iter().any(|a| *a == "-n" || a.starts_with("-n")),
            "show" => args.contains(&"--stat"),
            "worktree" => args.get(1) == Some(&"list"),
            "cat-file" => args.get(1) == Some(&"-s"),
            _ => true,
        };
        // Lo que va después de `--` son rutas, no opciones.
        let options = args.iter().take_while(|a| **a != "--");
        if !ok
            || options
                .clone()
                .any(|a| FORBIDDEN_ARGS.iter().any(|f| a.starts_with(f)))
        {
            return Err(forbidden());
        }
        Ok(())
    }

    /// `git <endurecimiento> -C <dir> <args>` (§7.2).
    ///
    /// `diff.autoRefreshIndex=false` no está en la lista de §7.2: lo añadió F4a
    /// porque M5 demostró que `git diff … HEAD` reescribe `.git/index` cuando hay
    /// ficheros con `stat` obsoleto, **aunque lleve `--no-optional-locks`**
    /// (Git 2.51.2.windows.1). Con la opción desactivada, el índice no cambia y la
    /// salida es la misma, porque Git compara el contenido de esos ficheros.
    fn command(&self, dir: &Path, args: &[&str]) -> Result<Command> {
        Self::check(args)?;
        let mut cmd = Command::new(&self.program);
        cmd.args([
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
            "-c",
            "diff.autoRefreshIndex=false",
            "-C",
        ])
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        // Las rutas se toman al pie de la letra, sin comodines ni «magia».
        .env("GIT_LITERAL_PATHSPECS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: sin consola que parpadee desde una app de ventana.
            cmd.creation_flags(0x0800_0000);
        }
        Ok(cmd)
    }

    fn run(&self, dir: &Path, args: &[&str], cap: usize) -> Result<Output> {
        let mut child = self
            .command(dir, args)?
            .spawn()
            .map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => GitError::NotInstalled,
                _ => GitError::Spawn(e),
            })?;
        let mut stdout = child.stdout.take().ok_or(GitError::Failed(None))?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = Vec::new();
            let truncated = read_capped(&mut stdout, cap, &mut buf);
            let _ = tx.send((buf, truncated));
        });
        match rx.recv_timeout(TIMEOUT) {
            Ok((stdout, truncated)) => {
                if truncated {
                    let _ = child.kill();
                }
                let status = child.wait().map_err(GitError::Spawn)?;
                if !truncated && !status.success() {
                    return Err(GitError::Failed(status.code()));
                }
                Ok(Output { stdout, truncated })
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(GitError::Timeout)
            }
        }
    }

    /// Raíz de Git de un `cwd`, o `None` si no está dentro de un repositorio.
    pub fn resolve(&self, cwd: &Path) -> Result<Option<RepoInfo>> {
        let out = match self.run(
            cwd,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--show-toplevel",
                "--git-dir",
                "--git-common-dir",
                "--show-prefix",
            ],
            64 * 1024,
        ) {
            Ok(out) => out,
            Err(GitError::Failed(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let mut lines = text.lines();
        let (Some(top), Some(git_dir), Some(common)) = (lines.next(), lines.next(), lines.next())
        else {
            return Ok(None);
        };
        Ok(Some(RepoInfo {
            toplevel: native_path(top),
            git_dir: native_path(git_dir),
            common_dir: native_path(common),
            prefix: lines.next().unwrap_or("").to_owned(),
        }))
    }

    /// `status --porcelain=v2 -z --branch` (§7.2).
    pub fn status(&self, root: &Path, untracked_all: bool) -> Result<GitStatus> {
        let untracked = if untracked_all {
            "--untracked-files=all"
        } else {
            "--untracked-files=normal"
        };
        let out = self.run(
            root,
            &["status", "--porcelain=v2", "-z", "--branch", untracked],
            MAX_LIST_BYTES,
        )?;
        Ok(parse_status(&out.stdout))
    }

    /// `diff --no-ext-diff --no-textconv --numstat -z HEAD` (§7.2), por ruta.
    /// Vacío si todavía no hay ningún commit.
    pub fn numstat(&self, root: &Path) -> Result<BTreeMap<String, LineCounts>> {
        match self.run(
            root,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--numstat",
                "-z",
                "HEAD",
            ],
            MAX_LIST_BYTES,
        ) {
            Ok(out) => Ok(parse_numstat(&out.stdout)),
            // Sin `HEAD` (repositorio recién creado) no hay nada con qué comparar.
            Err(GitError::Failed(_)) => Ok(BTreeMap::new()),
            Err(e) => Err(e),
        }
    }

    /// Diff de un fichero respecto a `HEAD`, bajo demanda y limitado a 2 MB (§7.2).
    pub fn diff_file(&self, root: &Path, rel_path: &str) -> Result<FileDiff> {
        if !is_safe_rel_path(rel_path) {
            return Err(GitError::BadPath);
        }
        let out = self.run(
            root,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "HEAD",
                "--",
                rel_path,
            ],
            MAX_DIFF_BYTES,
        )?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let binary = text
            .lines()
            .any(|l| l.starts_with("Binary files ") && l.ends_with(" differ"));
        Ok(FileDiff {
            text,
            truncated: out.truncated,
            binary,
        })
    }

    /// Número de ficheros versionados (`ls-files -z`).
    pub fn count_tracked(&self, root: &Path) -> Result<usize> {
        let out = self.run(root, &["ls-files", "-z"], MAX_LIST_BYTES)?;
        Ok(out.stdout.iter().filter(|b| **b == 0).count())
    }

    /// Ficheros versionados (`ls-files -z`), relativos a la raíz y con `/`. Solo
    /// los que se pueden abrir sin salir de ella; `true` si la lista se cortó.
    pub fn list_tracked(&self, root: &Path) -> Result<(Vec<String>, bool)> {
        let out = self.run(root, &["ls-files", "-z"], MAX_LIST_BYTES)?;
        let mut paths: Vec<String> = out
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .filter(|p| is_safe_rel_path(p))
            .collect();
        // Con la salida cortada, el último puede estar a medias.
        if out.truncated {
            paths.pop();
        }
        Ok((paths, out.truncated))
    }
}

/// Lee como mucho `cap` bytes. `true` si había más.
fn read_capped(r: &mut impl Read, cap: usize, buf: &mut Vec<u8>) -> bool {
    let mut chunk = [0u8; 64 * 1024];
    loop {
        match r.read(&mut chunk) {
            Ok(0) => return false,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > cap {
                    buf.truncate(cap);
                    return true;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
}

/// `C:/Users/x` → `C:\Users\x` en Windows.
fn native_path(s: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(s.replace('/', "\\"))
    } else {
        PathBuf::from(s)
    }
}

/// Ruta relativa a una raíz que se puede pasar a Git o abrir sin salir de ella.
pub fn is_safe_rel_path(rel: &str) -> bool {
    !rel.is_empty()
        && rel.len() <= 4096
        && !rel.starts_with(['/', '\\'])
        && !rel.contains(':')
        && !rel.contains('\0')
        && rel
            .split(['/', '\\'])
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

fn status_name(code: u8) -> &'static str {
    match code {
        b'A' => "added",
        b'D' => "deleted",
        b'R' => "renamed",
        b'C' => "copied",
        b'T' => "typechange",
        _ => "modified",
    }
}

/// Analiza la salida de `status --porcelain=v2 -z --branch`.
pub fn parse_status(raw: &[u8]) -> GitStatus {
    let mut out = GitStatus::default();
    let mut records = raw.split(|b| *b == 0).filter(|r| !r.is_empty());
    while let Some(record) = records.next() {
        let line = String::from_utf8_lossy(record);
        if let Some(header) = line.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" if value != "(initial)" => out.head = Some(value.to_owned()),
                "branch.head" if value != "(detached)" => out.branch = Some(value.to_owned()),
                "branch.upstream" => out.upstream = Some(value.to_owned()),
                "branch.ab" => {
                    let mut parts = value.split(' ');
                    out.ahead = parts
                        .next()
                        .and_then(|a| a.trim_start_matches('+').parse().ok());
                    out.behind = parts
                        .next()
                        .and_then(|b| b.trim_start_matches('-').parse().ok());
                }
                _ => {}
            }
            continue;
        }
        let kind = line.as_bytes().first().copied();
        let entry = match kind {
            Some(b'1') => line.splitn(9, ' ').nth(8).and_then(|path| {
                let xy = line.split(' ').nth(1)?.as_bytes();
                Some(entry(path, None, xy))
            }),
            Some(b'2') => line.splitn(10, ' ').nth(9).and_then(|path| {
                let xy = line.split(' ').nth(1)?.as_bytes();
                // El origen del renombrado es el registro siguiente.
                let from = records
                    .next()
                    .map(|f| String::from_utf8_lossy(f).into_owned());
                Some(entry(path, from, xy))
            }),
            Some(b'u') => line.splitn(11, ' ').nth(10).map(|path| StatusEntry {
                rel_path: path.to_owned(),
                from_rel_path: None,
                status: "conflict",
                staged: false,
            }),
            Some(b'?') => line.get(2..).map(|path| StatusEntry {
                rel_path: path.to_owned(),
                from_rel_path: None,
                status: "untracked",
                staged: false,
            }),
            _ => None,
        };
        out.entries.extend(entry);
    }
    out
}

fn entry(path: &str, from: Option<String>, xy: &[u8]) -> StatusEntry {
    let (x, y) = (
        xy.first().copied().unwrap_or(b'.'),
        xy.get(1).copied().unwrap_or(b'.'),
    );
    let code = if y != b'.' { y } else { x };
    StatusEntry {
        rel_path: path.to_owned(),
        from_rel_path: from,
        status: status_name(code),
        staged: x != b'.',
    }
}

/// Analiza `diff --numstat -z`. Los renombrados se guardan con la ruta de destino.
pub fn parse_numstat(raw: &[u8]) -> BTreeMap<String, LineCounts> {
    let mut out = BTreeMap::new();
    let mut records = raw.split(|b| *b == 0);
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        let line = String::from_utf8_lossy(record);
        let mut parts = line.splitn(3, '\t');
        let (Some(added), Some(deleted), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let path = if path.is_empty() {
            // Renombrado: `añadidas\tborradas\t\0origen\0destino\0`.
            let _from = records.next();
            match records.next() {
                Some(to) => String::from_utf8_lossy(to).into_owned(),
                None => continue,
            }
        } else {
            path.to_owned()
        };
        out.insert(
            path,
            LineCounts {
                added: added.parse().ok(),
                deleted: deleted.parse().ok(),
            },
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::SystemTime;

    /// Git «de escritura» solo para montar el repositorio de prueba en un temporal.
    /// La identidad va por `-c`: nunca se toca la configuración global (W8).
    fn setup_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args([
                "-c",
                "user.name=prueba",
                "-c",
                "user.email=prueba@example.invalid",
            ])
            .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        setup_git(root, &["init", "-q", "-b", "main"]);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.rs"), "uno\ndos\ntres\n").unwrap();
        fs::write(root.join("viejo.txt"), "contenido estable\n").unwrap();
        fs::write(root.join("bin.dat"), [0u8, 1, 2, 3]).unwrap();
        setup_git(root, &["add", "-A"]);
        setup_git(root, &["commit", "-q", "-m", "inicial"]);
        dir
    }

    fn mtime(path: &Path) -> SystemTime {
        fs::metadata(path).unwrap().modified().unwrap()
    }

    #[test]
    fn whitelist_rejects_write_commands_and_dangerous_options() {
        for args in [
            &["commit", "-m", "x"][..],
            &["fetch"],
            &["pull"],
            &["checkout", "main"],
            &["add", "."],
            &["stash"],
            &["gc"],
            &["worktree", "add", "x"],
            &["log"],
            &["show", "HEAD"],
            &["diff", "--output=fuera.txt", "HEAD"],
            &["diff", "--ext-diff", "HEAD"],
            &["diff", "--textconv", "HEAD"],
            &["--git-dir=x", "status"],
            &[],
        ] {
            assert!(GitReader::check(args).is_err(), "{args:?}");
        }
        for args in [
            &["status", "--porcelain=v2"][..],
            &["log", "-n", "1"],
            &["show", "--stat", "HEAD"],
            &["worktree", "list"],
            &["cat-file", "-s", "HEAD"],
            // Tras `--` todo es ruta, aunque se parezca a una opción.
            &["diff", "HEAD", "--", "--output=raro.txt"],
        ] {
            assert!(GitReader::check(args).is_ok(), "{args:?}");
        }
    }

    #[test]
    fn rejects_paths_that_leave_the_root() {
        for bad in [
            "",
            "../x",
            "a/../../x",
            "/etc/passwd",
            r"C:\x",
            "a:b",
            "a//b",
            "./a",
        ] {
            assert!(!is_safe_rel_path(bad), "{bad}");
        }
        assert!(is_safe_rel_path("src/a.rs"));
        assert!(is_safe_rel_path(r"src\a.rs"));
    }

    #[test]
    fn resolves_root_prefix_and_worktrees() {
        let repo = fixture();
        let git = GitReader::new();
        let info = git.resolve(&repo.path().join("src")).unwrap().unwrap();
        assert_eq!(
            fs::canonicalize(&info.toplevel).unwrap(),
            fs::canonicalize(repo.path()).unwrap()
        );
        assert_eq!(info.prefix, "src/");
        assert!(!info.is_worktree());

        let outside = tempfile::tempdir().unwrap();
        assert_eq!(git.resolve(outside.path()).unwrap(), None);

        let wt = repo.path().join("wt");
        setup_git(
            repo.path(),
            &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "rama"],
        );
        let info = git.resolve(&wt).unwrap().unwrap();
        assert!(info.is_worktree());
        assert_eq!(
            fs::canonicalize(info.main_root().unwrap()).unwrap(),
            fs::canonicalize(repo.path()).unwrap()
        );
    }

    #[test]
    fn status_numstat_and_diff_describe_the_working_tree() {
        let repo = fixture();
        let root = repo.path();
        let git = GitReader::new();
        fs::write(root.join("src/a.rs"), "uno\nDOS\ntres\ncuatro\n").unwrap();
        fs::write(root.join("nuevo con espacios.txt"), "x\n").unwrap();
        fs::remove_file(root.join("viejo.txt")).unwrap();
        fs::write(root.join("bin.dat"), [9u8, 0, 9]).unwrap();

        let status = git.status(root, true).unwrap();
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert!(status.head.is_some());
        let by_path: BTreeMap<_, _> = status
            .entries
            .iter()
            .map(|e| (e.rel_path.as_str(), e.status))
            .collect();
        assert_eq!(by_path.get("src/a.rs"), Some(&"modified"));
        assert_eq!(by_path.get("viejo.txt"), Some(&"deleted"));
        assert_eq!(by_path.get("nuevo con espacios.txt"), Some(&"untracked"));

        let numstat = git.numstat(root).unwrap();
        assert_eq!(
            numstat.get("src/a.rs"),
            Some(&LineCounts {
                added: Some(2),
                deleted: Some(1)
            })
        );
        assert_eq!(
            numstat.get("bin.dat"),
            Some(&LineCounts {
                added: None,
                deleted: None
            })
        );

        let diff = git.diff_file(root, "src/a.rs").unwrap();
        assert!(diff.text.contains("+DOS") && diff.text.contains("-dos"));
        assert!(!diff.binary && !diff.truncated);
        assert!(git.diff_file(root, "bin.dat").unwrap().binary);
        assert!(matches!(
            git.diff_file(root, "../fuera"),
            Err(GitError::BadPath)
        ));
        assert_eq!(git.count_tracked(root).unwrap(), 3);
    }

    #[test]
    fn a_repository_without_commits_has_no_head_and_no_numstat() {
        let dir = tempfile::tempdir().unwrap();
        setup_git(dir.path(), &["init", "-q", "-b", "main"]);
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        let git = GitReader::new();
        let status = git.status(dir.path(), true).unwrap();
        assert_eq!(status.head, None);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.entries.len(), 1);
        assert!(git.numstat(dir.path()).unwrap().is_empty());
    }

    /// M5 (§11.1, R14): ninguna lectura reescribe el índice, ni siquiera cuando
    /// sus datos de `stat` están obsoletos y un `git status` normal lo refrescaría.
    #[test]
    fn m5_reads_never_touch_the_index() {
        let repo = fixture();
        let root = repo.path();
        let index = root.join(".git").join("index");
        // Reescribir el mismo contenido deja obsoleta la información de `stat` del índice.
        thread::sleep(Duration::from_millis(1100));
        fs::write(root.join("viejo.txt"), "contenido estable\n").unwrap();
        fs::write(root.join("src/a.rs"), "uno\ndos\ntres\n").unwrap();
        let before = mtime(&index);
        let bytes_before = fs::read(&index).unwrap();

        let git = GitReader::new();
        for _ in 0..3 {
            git.resolve(root).unwrap();
            git.status(root, true).unwrap();
            git.status(root, false).unwrap();
            git.numstat(root).unwrap();
            git.diff_file(root, "src/a.rs").unwrap();
            git.count_tracked(root).unwrap();
        }
        assert_eq!(mtime(&index), before, "una lectura ha reescrito .git/index");
        assert_eq!(fs::read(&index).unwrap(), bytes_before);

        // Control: el mismo `status` sin `--no-optional-locks` sí lo reescribe, así
        // que el test es capaz de detectar la escritura.
        thread::sleep(Duration::from_millis(1100));
        setup_git(root, &["status", "--porcelain"]);
        assert_ne!(
            mtime(&index),
            before,
            "el control no ha refrescado el índice"
        );
    }

    #[test]
    fn parses_porcelain_v2_records() {
        let raw =
            b"# branch.oid abc123\0# branch.head feature/x\0# branch.upstream origin/feature/x\0\
# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 h1 h2 src/con espacio.rs\0\
2 R. N... 100644 100644 100644 h1 h2 R100 nuevo.rs\0viejo.rs\0\
u UU N... 100644 100644 100644 100644 h1 h2 h3 conflicto.rs\0\
? sin seguir.txt\0";
        let s = parse_status(raw);
        assert_eq!(s.branch.as_deref(), Some("feature/x"));
        assert_eq!((s.ahead, s.behind), (Some(2), Some(1)));
        assert_eq!(s.entries.len(), 4);
        assert_eq!(s.entries[0].rel_path, "src/con espacio.rs");
        assert!(!s.entries[0].staged);
        assert_eq!(s.entries[1].status, "renamed");
        assert_eq!(s.entries[1].from_rel_path.as_deref(), Some("viejo.rs"));
        assert!(s.entries[1].staged);
        assert_eq!(s.entries[2].status, "conflict");
        assert_eq!(s.entries[3].rel_path, "sin seguir.txt");
    }

    #[test]
    fn parses_numstat_with_renames_and_binaries() {
        let raw = b"3\t1\tsrc/a.rs\0-\t-\timg.png\0\x30\t0\t\0viejo.rs\0nuevo.rs\0";
        let n = parse_numstat(raw);
        assert_eq!(n["src/a.rs"].added, Some(3));
        assert_eq!(n["img.png"].added, None);
        assert!(n.contains_key("nuevo.rs") && !n.contains_key("viejo.rs"));
    }
}
