//! Workspace Watcher de extremo a extremo (SPEC §7): repositorio temporal, sesión
//! viva en el registro, un cliente HTTP que hace de mod y cambios reales en disco.

use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use cockpit_core::privacy::PrivacyPolicy;
use cockpit_ingest::live::{LiveConfig, LivePipeline, LiveState, now_ms};
use cockpit_ingest::workspace::{WatchState, WorkspaceView};
use cockpit_store::FileEventView;

const SESSION: &str = "55555555-5555-4555-8555-555555555555";
const INSTANCE: &str = "inst-ws-1";

struct Client {
    port: u16,
    token: String,
}

impl Client {
    fn new(dir: &Path) -> Self {
        let raw = fs::read(dir.join("endpoint.json")).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        Self {
            port: v["port"].as_u64().unwrap() as u16,
            token: v["token"].as_str().unwrap().to_owned(),
        }
    }

    fn post(&self, path: &str, body: &str) -> u16 {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, self.port)).unwrap();
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\
             Authorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            self.port,
            self.token,
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body.as_bytes()).unwrap();
        let mut raw = String::new();
        let _ = stream.read_to_string(&mut raw);
        raw.split(' ')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    fn events(&self, events: &[String]) {
        let body = format!(
            r#"{{"schema":1,"instance_id":"{INSTANCE}","session_id":"{SESSION}","events":[{}]}}"#,
            events.join(",")
        );
        assert_eq!(self.post("/v1/events", &body), 200);
    }
}

fn hello(now: i64) -> String {
    format!(
        r#"{{"schema":1,
            "mod":{{"name":"dev-cockpit-bridge","version":"0.1.0","instance_id":"{INSTANCE}"}},
            "engine":{{"version":"2.1.321","base":"2.1.321"}},
            "session":{{"id":"{SESSION}","surfaces":["terminal"],"is_interactive":true,"started_at":{}}},
            "reason":"startup","hello_seq":1,
            "applied_policy":{{"id":"pre_handshake","capture_task_titles":false,"record_paths":false,"capture_shell_program":false}},
            "snapshot":{{"agents":[],"running_tools":[]}},
            "queue":{{"pending":0,"dropped":0,"next_seq":1}}}}"#,
        now - 60_000
    )
}

fn started(seq: i64, t: i64, id: &str, tool: &str, path: Option<&str>) -> String {
    let meta = path.map_or(String::new(), |p| format!(r#""rel_path":"{p}""#));
    format!(
        r#"{{"kind":"tool_started","seq":{seq},"t":{t},"tool_use_id":"{id}","tool":"{tool}","meta":{{{meta}}}}}"#
    )
}

fn finished(seq: i64, t: i64, id: &str) -> String {
    format!(r#"{{"kind":"tool_finished","seq":{seq},"t":{t},"tool_use_id":"{id}","status":"ok"}}"#)
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args([
            "-c",
            "user.name=prueba",
            "-c",
            "user.email=prueba@example.invalid",
        ])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.autocrlf=false",
            "-C",
        ])
        .arg(dir)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn wait_for<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(v) = probe() {
            return v;
        }
        assert!(Instant::now() < deadline, "sin respuesta en 15 s: {what}");
        thread::sleep(Duration::from_millis(25));
    }
}

fn event_for(
    state: &LiveState,
    root: i64,
    path: &str,
    pred: impl Fn(&FileEventView) -> bool,
) -> Option<FileEventView> {
    state
        .recent_file_events(root)
        .into_iter()
        .rev()
        .find(|e| e.rel_path.as_deref() == Some(path) && pred(e))
}

#[test]
fn file_changes_are_attributed_and_git_state_is_published() {
    let dir = tempfile::tempdir().unwrap();
    let repo = fs::canonicalize(dir.path()).unwrap().join("repo");
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
    fs::write(repo.join("src/a.rs"), "uno\n").unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "inicial"]);

    let sessions = dir.path().join("sessions");
    let projects = dir.path().join("projects");
    fs::create_dir_all(&sessions).unwrap();
    fs::create_dir_all(&projects).unwrap();
    let me = std::process::id();
    let entry = serde_json::json!({
        "pid": me, "sessionId": SESSION, "cwd": repo.join("src").to_string_lossy(), "status": "busy"
    });
    fs::write(sessions.join(format!("{me}.json")), entry.to_string()).unwrap();

    let db_dir = dir.path().join("db");
    let state = LiveState::new(|| {});
    let _pipeline = LivePipeline::spawn(
        LiveConfig {
            projects_root: projects,
            sessions_dir: Some(sessions.clone()),
            db_path: db_dir.join("cockpit.db"),
            endpoint_path: Some(db_dir.join("endpoint.json")),
            policy: PrivacyPolicy::APP_DEFAULT,
        },
        state.clone(),
    )
    .unwrap();

    // La raíz se activa con el `cwd` de la sesión viva (un subdirectorio del repo).
    let ws: WorkspaceView = wait_for("raíz activa con estado de Git", || {
        state
            .workspaces()
            .into_iter()
            .find(|w| w.git.as_ref().is_some_and(|g| g.branch.is_some()))
    });
    assert_eq!(ws.watch, WatchState::Active);
    assert!(ws.is_git);
    assert_eq!(ws.name, "repo");
    assert_eq!(ws.sessions, [SESSION]);
    let root = ws.root_id;

    let client = wait_for("endpoint.json", || {
        db_dir
            .join("endpoint.json")
            .exists()
            .then(|| Client::new(&db_dir))
    });
    assert_eq!(client.post("/v1/hello", &hello(now_ms())), 200);
    thread::sleep(Duration::from_millis(400));

    // R4: un cambio con la sesión activa pero sin herramienta en curso.
    fs::write(repo.join("notas.txt"), "x").unwrap();
    let e = wait_for("notas.txt", || {
        event_for(&state, root, "notas.txt", |_| true)
    });
    assert_eq!(
        (e.actor.as_str(), e.confidence.as_str()),
        ("unknown", "low")
    );

    // R1: Edit con la misma ruta. El mod relativiza al `cwd` (`src/`) y el Watcher
    // a la raíz: la atribución traduce con el prefijo de `rev-parse`.
    let t = now_ms();
    client.events(&[started(1, t, "toolu_edit", "Edit", Some("a.rs"))]);
    fs::write(repo.join("src/a.rs"), "uno\ndos\n").unwrap();
    client.events(&[finished(2, now_ms(), "toolu_edit")]);
    let e = wait_for("src/a.rs atribuido", || {
        event_for(&state, root, "src/a.rs", |e| e.actor == "claude")
    });
    assert_eq!(e.confidence, "high");
    assert_eq!(e.tool_use_id.as_deref(), Some("toolu_edit"));
    assert_eq!(e.tool.as_deref(), Some("Edit"));
    assert_eq!(e.session_id.as_deref(), Some(SESSION));

    // Ventana de asentamiento: el fichero cambia antes de que llegue el `ToolStarted`.
    thread::sleep(Duration::from_millis(300));
    let t = now_ms();
    fs::write(repo.join("src/b.rs"), "nuevo\n").unwrap();
    let early = wait_for("src/b.rs provisional", || {
        event_for(&state, root, "src/b.rs", |_| true)
    });
    assert!(!early.settled);
    client.events(&[
        started(3, t, "toolu_write", "Write", Some("b.rs")),
        finished(4, now_ms(), "toolu_write"),
    ]);
    let e = wait_for("src/b.rs subido a R1", || {
        event_for(&state, root, "src/b.rs", |e| {
            e.actor == "claude" && e.settled
        })
    });
    assert_eq!(
        (e.confidence.as_str(), e.tool_use_id.as_deref()),
        ("high", Some("toolu_write"))
    );

    // R3: lo que escribe un Bash de la sesión.
    thread::sleep(Duration::from_millis(300));
    let t = now_ms();
    client.events(&[started(5, t, "toolu_bash", "Bash", None)]);
    fs::create_dir_all(repo.join("out")).unwrap();
    fs::write(repo.join("out/x.txt"), "x").unwrap();
    fs::write(repo.join("ruido.log"), "ignorado").unwrap();
    thread::sleep(Duration::from_millis(500));
    client.events(&[finished(6, now_ms(), "toolu_bash")]);
    let e = wait_for("out/x.txt", || {
        event_for(&state, root, "out/x.txt", |e| e.settled)
    });
    assert_eq!(
        (e.actor.as_str(), e.confidence.as_str()),
        ("claude-shell", "medium")
    );
    assert!(
        event_for(&state, root, "ruido.log", |_| true).is_none(),
        ".gitignore"
    );

    // Git: rama y `numstat` del fichero editado.
    let git_view = wait_for("numstat de src/a.rs", || {
        state
            .workspaces()
            .into_iter()
            .find(|w| w.root_id == root)
            .and_then(|w| {
                w.git.filter(|g| {
                    g.files
                        .iter()
                        .any(|f| f.rel_path == "src/a.rs" && f.added == Some(1))
                })
            })
    });
    assert_eq!(git_view.branch.as_deref(), Some("main"));
    assert!(
        git_view
            .files
            .iter()
            .any(|f| f.rel_path == "out/x.txt" && f.status == "untracked")
    );

    // R5: la sesión termina y su proceso desaparece del registro.
    client.events(
        &[r#"{"kind":"session_ended","seq":7,"t":0,"reason":"exit"}"#
            .replace("\"t\":0", &format!("\"t\":{}", now_ms()))],
    );
    fs::remove_file(sessions.join(format!("{me}.json"))).unwrap();
    wait_for("raíz sin sesiones vivas", || {
        state
            .workspaces()
            .into_iter()
            .find(|w| w.root_id == root && w.sessions.is_empty())
    });
    fs::write(repo.join("externo.txt"), "x").unwrap();
    let e = wait_for("externo.txt", || {
        event_for(&state, root, "externo.txt", |_| true)
    });
    assert_eq!(
        (e.actor.as_str(), e.confidence.as_str()),
        ("external", "high")
    );
}
