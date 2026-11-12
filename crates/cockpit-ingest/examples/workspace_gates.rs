//! Puertas G5 y G7 de F4a (SPEC §13.2) contra la canalización real.
//!
//! Levanta `LivePipeline` (HTTP, `notify`, Git y SQLite) sobre un repositorio
//! temporal, con dos sesiones vivas en un registro de pruebas y un cliente HTTP
//! que hace de mod. No toca `~/.claude` ni la base de datos real.
//!
//! ```text
//! cargo run -p cockpit-ingest --release --example workspace_gates -- g5 salida.json
//! cargo run -p cockpit-ingest --release --example workspace_gates -- g7 salida.json
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use cockpit_core::privacy::PrivacyPolicy;
use cockpit_ingest::live::{LiveConfig, LivePipeline, LiveState, now_ms};
use cockpit_store::Store;
use serde_json::json;

const SESSION_A: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const SESSION_B: &str = "bbbbbbbb-0000-4000-8000-00000000000b";
/// Retardo de envío del mod: `$.clock.after(50)` antes del `fetch` (§5.3).
const FLUSH_MS: u64 = 50;

/// Generador pseudoaleatorio reproducible (xorshift).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

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

    fn post(&self, path: &str, body: &serde_json::Value) {
        let body = body.to_string();
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
        assert!(raw.starts_with("HTTP/1.1 200"), "{path}: {raw}");
    }
}

/// Un mod simulado: numera los eventos y los envía como lo haría `register.ts`.
struct Mod<'a> {
    client: &'a Client,
    session: &'static str,
    instance: String,
    seq: i64,
}

impl<'a> Mod<'a> {
    fn hello(client: &'a Client, session: &'static str) -> Self {
        let instance = format!("inst-{}", &session[..8]);
        client.post(
            "/v1/hello",
            &json!({
                "schema": 1,
                "mod": {"name": "dev-cockpit-bridge", "version": "0.1.0", "instance_id": instance},
                "engine": {"version": "2.1.321", "base": "2.1.321"},
                "session": {"id": session, "surfaces": ["terminal"], "is_interactive": true, "started_at": now_ms() - 60_000},
                "reason": "startup", "hello_seq": 1,
                "applied_policy": {"id": "pre_handshake", "capture_task_titles": false, "record_paths": false, "capture_shell_program": false},
                "snapshot": {"agents": [], "running_tools": []},
                "queue": {"pending": 0, "dropped": 0, "next_seq": 1}
            }),
        );
        Self {
            client,
            session,
            instance,
            seq: 0,
        }
    }

    fn send(&mut self, events: Vec<serde_json::Value>) {
        let events: Vec<serde_json::Value> = events
            .into_iter()
            .map(|mut e| {
                self.seq += 1;
                e["seq"] = json!(self.seq);
                e
            })
            .collect();
        self.client.post(
            "/v1/events",
            &json!({"schema": 1, "instance_id": self.instance, "session_id": self.session, "events": events}),
        );
    }
}

fn started(t: i64, id: &str, tool: &str, rel_path: Option<&str>) -> serde_json::Value {
    let meta = match rel_path {
        Some(p) => json!({"rel_path": p}),
        None => json!({}),
    };
    json!({"kind": "tool_started", "t": t, "tool_use_id": id, "tool": tool, "meta": meta})
}

fn finished(t: i64, id: &str) -> serde_json::Value {
    json!({"kind": "tool_finished", "t": t, "tool_use_id": id, "status": "ok"})
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(["-c", "user.name=g5", "-c", "user.email=g5@example.invalid"])
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

fn sleep_ms(ms: u64) {
    thread::sleep(Duration::from_millis(ms));
}

/// Entorno de prueba: repositorio, registro con sesiones vivas y canalización.
struct Env {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    sessions: PathBuf,
    db: PathBuf,
    state: LiveState,
    _pipeline: LivePipeline,
}

impl Env {
    fn new(sessions: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let repo = base.join("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
        for i in 0..80 {
            fs::write(
                repo.join(format!("src/f{i:03}.rs")),
                format!("// fichero {i}\n"),
            )
            .unwrap();
        }
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        let registry = base.join("sessions");
        fs::create_dir_all(&registry).unwrap();
        let me = std::process::id();
        for s in sessions {
            fs::write(
                registry.join(format!("{}.json", &s[..8])),
                json!({"pid": me, "sessionId": s, "cwd": repo.to_string_lossy(), "status": "busy"})
                    .to_string(),
            )
            .unwrap();
        }
        let db_dir = base.join("db");
        let projects = base.join("projects");
        fs::create_dir_all(&projects).unwrap();
        let state = LiveState::new(|| {});
        let pipeline = LivePipeline::spawn(
            LiveConfig {
                projects_root: projects,
                sessions_dir: Some(registry.clone()),
                db_path: db_dir.join("cockpit.db"),
                endpoint_path: Some(db_dir.join("endpoint.json")),
                policy: PrivacyPolicy::APP_DEFAULT,
            },
            state.clone(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !state.workspaces().iter().any(|w| {
            w.git.as_ref().is_some_and(|g| g.branch.is_some()) && w.sessions.len() == sessions.len()
        }) {
            if Instant::now() >= deadline {
                let sessions = match state.snapshot() {
                    cockpit_ingest::live::LiveDelta::Snapshot { sessions, .. } => sessions
                        .iter()
                        .map(|s| {
                            (
                                s.summary.id.clone(),
                                s.live.is_some(),
                                s.summary.cwd.clone(),
                            )
                        })
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                };
                panic!(
                    "la raíz no se activa: raíces {:?}, sesiones {sessions:?}",
                    state
                        .workspaces()
                        .iter()
                        .map(|w| (&w.root, &w.sessions, w.git.is_some()))
                        .collect::<Vec<_>>()
                );
            }
            sleep_ms(50);
        }
        Self {
            _dir: dir,
            repo,
            sessions: registry,
            db: db_dir,
            state,
            _pipeline: pipeline,
        }
    }

    fn client(&self) -> Client {
        Client::new(&self.db)
    }

    fn write(&self, rel: &str, text: &str) -> i64 {
        let t = now_ms();
        let path = self.repo.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
        t
    }

    /// Escritura como la de `Edit`/`Write` de Claude Code (medida en F4a con
    /// Desktop 2.1.321): temporal `<nombre>.tmp.<pid>.<hex>`, borrado del original
    /// si existe y renombrado del temporal sobre él.
    fn claude_write(&self, rel: &str, text: &str, n: u32) {
        let path = self.repo.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let tmp = self.repo.join(format!(
            "{rel}.tmp.{}.{:012x}",
            std::process::id(),
            0xa5a5_0000u64 + u64::from(n)
        ));
        fs::write(&tmp, text).unwrap();
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        fs::rename(&tmp, &path).unwrap();
    }

    fn root_id(&self) -> i64 {
        self.state.workspaces()[0].root_id
    }

    /// Filas finales de `file_events`, por ruta.
    fn rows(&self) -> BTreeMap<String, Vec<Row>> {
        let store = Store::open_read_only(&self.db.join("cockpit.db")).unwrap();
        let mut stmt = store
            .connection()
            .prepare(
                "SELECT fe.rel_path, fe.from_rel_path, fe.change, fe.ts, fe.actor, fe.actor_tool_use_id,
                        fe.confidence, fe.settled, tc.started_at, tc.finished_at
                 FROM file_events fe LEFT JOIN tool_calls tc
                   ON tc.session_id = fe.actor_session AND tc.tool_use_id = fe.actor_tool_use_id
                 WHERE fe.root_id = ?1 ORDER BY fe.id",
            )
            .unwrap();
        let mut out: BTreeMap<String, Vec<Row>> = BTreeMap::new();
        let rows = stmt
            .query_map([self.root_id()], |r| {
                Ok(Row {
                    rel_path: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    from: r.get(1)?,
                    change: r.get(2)?,
                    ts: r.get(3)?,
                    actor: r.get(4)?,
                    tool: r.get(5)?,
                    confidence: r.get(6)?,
                    settled: r.get(7)?,
                })
            })
            .unwrap();
        for row in rows {
            let row = row.unwrap();
            out.entry(row.rel_path.clone()).or_default().push(row);
        }
        out
    }
}

#[derive(Debug, Clone)]
struct Row {
    rel_path: String,
    from: Option<String>,
    change: String,
    ts: i64,
    actor: String,
    tool: Option<String>,
    confidence: String,
    settled: bool,
}

/// Verdad de un cambio: quién lo hizo de verdad.
#[derive(Debug, Clone)]
struct Truth {
    scenario: &'static str,
    rel_path: String,
    /// `claude`, `claude-shell` o `external`.
    actor: &'static str,
    tool: Option<String>,
    /// Intervalo real de la herramienta, para calibrar ε y δ.
    interval: Option<(i64, i64)>,
}

fn g5(out: &Path) {
    let env = Env::new(&[SESSION_A, SESSION_B]);
    let client = env.client();
    let mut a = Mod::hello(&client, SESSION_A);
    let mut b = Mod::hello(&client, SESSION_B);
    sleep_ms(500);
    let mut rng = Rng(0x5eed_1234_abcd_ef01);
    let mut truth: Vec<Truth> = Vec::new();
    let mut n = 0u32;
    let mut next_id = |prefix: &str| {
        n += 1;
        format!("toolu_{prefix}_{n:04}")
    };
    // Separación entre escenarios: más que δ, para que no se mezclen intervalos.
    let gap = || sleep_ms(1_800);

    // 1. Edit y Write de Claude: el mod envía 50 ms después de empezar y, a veces,
    //    mucho más tarde (reconexión: el ToolFinished llega pasada la ventana).
    // Cada fichero se toca una sola vez: la verdad se compara por ruta.
    let mut edited = 0u32;
    for i in 0..90 {
        let id = next_id("edit");
        let tool = if i % 3 == 0 { "Write" } else { "Edit" };
        let rel = if tool == "Write" {
            format!("src/nuevo_{i:03}.rs")
        } else {
            edited += 1;
            format!("src/f{:03}.rs", edited - 1)
        };
        let late = rng.chance(10);
        let t0 = now_ms();
        sleep_ms(rng.range(3, 25));
        env.claude_write(&rel, &format!("// editado {i}\n"), i);
        sleep_ms(rng.range(2, 15));
        let t1 = now_ms();
        let wait = if late {
            rng.range(2_600, 4_000)
        } else {
            FLUSH_MS
        };
        let elapsed = (now_ms() - t0) as u64;
        sleep_ms(wait.saturating_sub(elapsed));
        a.send(vec![started(t0, &id, tool, Some(&rel)), finished(t1, &id)]);
        truth.push(Truth {
            scenario: if late { "edit_mod_tardío" } else { "edit" },
            rel_path: rel,
            actor: "claude",
            tool: Some(id),
            interval: Some((t0, t1)),
        });
        sleep_ms(rng.range(150, 400));
        if i % 30 == 29 {
            gap();
        }
    }
    gap();

    // 2. Bash de Claude (sesión B): escribe 2–3 ficheros durante 0,5–2 s.
    for i in 0..20 {
        let id = next_id("bash");
        let t0 = now_ms();
        sleep_ms(FLUSH_MS);
        b.send(vec![started(t0, &id, "Bash", None)]);
        let files = rng.range(2, 3);
        let mut written = Vec::new();
        for k in 0..files {
            sleep_ms(rng.range(150, 600));
            let rel = format!("out/bash_{i:02}_{k}.txt");
            env.write(&rel, "salida\n");
            written.push(rel);
        }
        sleep_ms(rng.range(50, 300));
        let t1 = now_ms();
        sleep_ms(FLUSH_MS);
        b.send(vec![finished(t1, &id)]);
        for rel in written {
            truth.push(Truth {
                scenario: "bash",
                rel_path: rel,
                actor: "claude-shell",
                tool: Some(id.clone()),
                interval: Some((t0, t1)),
            });
        }
        sleep_ms(400);
    }
    gap();

    // 3. Cambios externos con las sesiones activas y sin herramientas en curso.
    for i in 0..30 {
        let rel = format!("docs/externo_{i:02}.md");
        env.write(&rel, "a mano\n");
        truth.push(Truth {
            scenario: "externo_sesión_activa",
            rel_path: rel,
            actor: "external",
            tool: None,
            interval: None,
        });
        sleep_ms(rng.range(300, 700));
    }
    gap();

    // 4. Herramientas en paralelo de la misma sesión, cada una con su fichero.
    for i in 0..12 {
        let (id1, id2) = (next_id("par"), next_id("par"));
        let (p1, p2) = (
            format!("src/par_{i:02}_a.rs"),
            format!("src/par_{i:02}_b.rs"),
        );
        let t0 = now_ms();
        sleep_ms(rng.range(2, 10));
        let t0b = now_ms();
        env.claude_write(&p2, "b\n", 1000 + i * 2);
        sleep_ms(rng.range(5, 20));
        env.claude_write(&p1, "a\n", 1001 + i * 2);
        sleep_ms(rng.range(2, 10));
        let t1 = now_ms();
        sleep_ms(FLUSH_MS);
        a.send(vec![
            started(t0, &id1, "Edit", Some(&p1)),
            started(t0b, &id2, "Write", Some(&p2)),
            finished(t1, &id1),
            finished(t1, &id2),
        ]);
        truth.push(Truth {
            scenario: "paralelo",
            rel_path: p1,
            actor: "claude",
            tool: Some(id1),
            interval: Some((t0, t1)),
        });
        truth.push(Truth {
            scenario: "paralelo",
            rel_path: p2,
            actor: "claude",
            tool: Some(id2),
            interval: Some((t0b, t1)),
        });
        sleep_ms(rng.range(300, 600));
    }
    gap();

    // 5. Dos sesiones en la misma raíz editando a la vez.
    for i in 0..12 {
        let (ida, idb) = (next_id("dos"), next_id("dos"));
        let (pa, pb) = (
            format!("src/dos_{i:02}_a.rs"),
            format!("src/dos_{i:02}_b.rs"),
        );
        let t0 = now_ms();
        env.claude_write(&pa, "a\n", 2000 + i * 2);
        sleep_ms(rng.range(1, 15));
        env.claude_write(&pb, "b\n", 2001 + i * 2);
        let t1 = now_ms();
        sleep_ms(FLUSH_MS);
        a.send(vec![
            started(t0, &ida, "Edit", Some(&pa)),
            finished(t1, &ida),
        ]);
        b.send(vec![
            started(t0, &idb, "Edit", Some(&pb)),
            finished(t1, &idb),
        ]);
        truth.push(Truth {
            scenario: "dos_sesiones",
            rel_path: pa,
            actor: "claude",
            tool: Some(ida),
            interval: Some((t0, t1)),
        });
        truth.push(Truth {
            scenario: "dos_sesiones",
            rel_path: pb,
            actor: "claude",
            tool: Some(idb),
            interval: Some((t0, t1)),
        });
        sleep_ms(rng.range(300, 600));
    }
    gap();

    // 6. Adverso: cambios externos mientras corre un Bash largo de la sesión B.
    for i in 0..8 {
        let id = next_id("bashlargo");
        let t0 = now_ms();
        sleep_ms(FLUSH_MS);
        b.send(vec![started(t0, &id, "Bash", None)]);
        sleep_ms(rng.range(200, 600));
        let rel = format!("docs/externo_durante_bash_{i:02}.md");
        env.write(&rel, "a mano\n");
        truth.push(Truth {
            scenario: "externo_durante_bash",
            rel_path: rel,
            actor: "external",
            tool: None,
            interval: None,
        });
        sleep_ms(rng.range(200, 600));
        let t1 = now_ms();
        b.send(vec![finished(t1, &id)]);
        sleep_ms(400);
    }
    gap();

    // 7. Adverso: un cambio externo justo mientras Claude edita otro fichero.
    for i in 0..8 {
        let id = next_id("edit_vs_ext");
        let rel_claude = format!("src/vs_{i:02}.rs");
        let rel_ext = format!("docs/externo_durante_edit_{i:02}.md");
        let t0 = now_ms();
        env.claude_write(&rel_claude, "claude\n", 3000 + i);
        sleep_ms(rng.range(5, 40));
        env.write(&rel_ext, "a mano\n");
        let t1 = now_ms();
        sleep_ms(FLUSH_MS);
        a.send(vec![
            started(t0, &id, "Edit", Some(&rel_claude)),
            finished(t1, &id),
        ]);
        truth.push(Truth {
            scenario: "edit_vs_externo",
            rel_path: rel_claude,
            actor: "claude",
            tool: Some(id),
            interval: Some((t0, t1)),
        });
        truth.push(Truth {
            scenario: "externo_durante_edit",
            rel_path: rel_ext,
            actor: "external",
            tool: None,
            interval: None,
        });
        sleep_ms(rng.range(400, 800));
    }
    gap();

    // 8. Sin sesiones vivas: R5.
    a.send(vec![
        json!({"kind": "session_ended", "t": now_ms(), "reason": "exit"}),
    ]);
    b.send(vec![
        json!({"kind": "session_ended", "t": now_ms(), "reason": "exit"}),
    ]);
    for entry in fs::read_dir(&env.sessions).unwrap().flatten() {
        fs::remove_file(entry.path()).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(40);
    while !env.state.workspaces().iter().all(|w| w.sessions.is_empty()) {
        assert!(Instant::now() < deadline, "las sesiones no terminan");
        sleep_ms(100);
    }
    for i in 0..15 {
        let rel = format!("docs/sin_sesion_{i:02}.md");
        env.write(&rel, "a mano\n");
        truth.push(Truth {
            scenario: "sin_sesión",
            rel_path: rel,
            actor: "external",
            tool: None,
            interval: None,
        });
        sleep_ms(rng.range(200, 400));
    }
    // Que todo se asiente (2 s) y que lleguen los envíos tardíos.
    sleep_ms(5_000);

    report_g5(&truth, &env.rows(), out);
}

fn report_g5(truth: &[Truth], rows: &BTreeMap<String, Vec<Row>>, out: &Path) {
    #[derive(Default)]
    struct Cell {
        total: u32,
        correct: u32,
    }
    let mut by_conf: BTreeMap<String, Cell> = BTreeMap::new();
    let mut by_scenario: BTreeMap<&str, BTreeMap<String, u32>> = BTreeMap::new();
    let mut matrix: BTreeMap<(&str, String), u32> = BTreeMap::new();
    let mut missing = Vec::new();
    let mut unsettled = 0;
    let mut external_as_claude = 0;
    let mut offsets_start = Vec::new();
    let mut offsets_end = Vec::new();
    for t in truth {
        let Some(found) = rows.get(&t.rel_path) else {
            missing.push(t.rel_path.clone());
            continue;
        };
        let row = found.last().unwrap();
        if !row.settled {
            unsettled += 1;
        }
        let correct = row.actor == t.actor
            || (t.actor == "external" && row.actor == "unknown" && row.confidence == "low");
        let correct = correct && (t.tool.is_none() || row.tool == t.tool);
        let cell = by_conf.entry(row.confidence.clone()).or_default();
        cell.total += 1;
        cell.correct += u32::from(correct);
        *by_scenario
            .entry(t.scenario)
            .or_default()
            .entry(format!("{}/{}", row.actor, row.confidence))
            .or_default() += 1;
        *matrix.entry((t.actor, row.actor.clone())).or_default() += 1;
        if t.actor == "external" && row.actor == "claude" {
            external_as_claude += 1;
        }
        if let Some((start, end)) = t.interval {
            offsets_start.push(row.ts - start);
            offsets_end.push(row.ts - end);
        }
        let _ = (&row.from, &row.change);
    }
    let pct = |v: &mut Vec<i64>, p: f64| -> i64 {
        v.sort_unstable();
        if v.is_empty() {
            return 0;
        }
        v[((p * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1]
    };
    let precision = |c: &str| {
        by_conf.get(c).map(|cell| {
            (
                cell.correct,
                cell.total,
                f64::from(cell.correct) / f64::from(cell.total.max(1)),
            )
        })
    };
    let report = json!({
        "changes": truth.len(),
        "missing": missing,
        "unsettled_at_end": unsettled,
        "precision": {
            "high": precision("high"),
            "medium": precision("medium"),
            "low": precision("low"),
        },
        "external_attributed_to_claude": external_as_claude,
        "matrix_truth_to_actor": matrix.iter().map(|((t, a), n)| json!({"truth": t, "actor": a, "n": n})).collect::<Vec<_>>(),
        "by_scenario": by_scenario,
        "offset_event_minus_tool_start_ms": {
            "min": pct(&mut offsets_start.clone(), 0.0001), "p50": pct(&mut offsets_start.clone(), 0.5),
            "p95": pct(&mut offsets_start.clone(), 0.95), "max": pct(&mut offsets_start, 1.0)
        },
        "offset_event_minus_tool_end_ms": {
            "min": pct(&mut offsets_end.clone(), 0.0001), "p50": pct(&mut offsets_end.clone(), 0.5),
            "p95": pct(&mut offsets_end.clone(), 0.95), "max": pct(&mut offsets_end, 1.0)
        },
    });
    let text = serde_json::to_string_pretty(&report).unwrap();
    fs::write(out, &text).unwrap();
    println!("{text}");
}

/// G7: renombrados en NTFS (mismo directorio, entre carpetas, con y sin cambio de nombre).
fn g7(out: &Path) {
    let env = Env::new(&[SESSION_A]);
    let mut expected: Vec<(&str, String, Option<String>)> = Vec::new();
    for i in 0..40 {
        // Mismo directorio.
        let (from, to) = (format!("src/f{i:03}.rs"), format!("src/r{i:03}.rs"));
        fs::rename(env.repo.join(&from), env.repo.join(&to)).unwrap();
        expected.push(("mismo_directorio", to, Some(from)));
        sleep_ms(120);
    }
    fs::create_dir_all(env.repo.join("movidos")).unwrap();
    for i in 40..60 {
        // Entre carpetas, mismo nombre.
        let (from, to) = (format!("src/f{i:03}.rs"), format!("movidos/f{i:03}.rs"));
        fs::rename(env.repo.join(&from), env.repo.join(&to)).unwrap();
        expected.push(("entre_carpetas", to, Some(from)));
        sleep_ms(120);
    }
    for i in 60..80 {
        // Entre carpetas y con otro nombre: Windows da borrado + creado.
        let (from, to) = (format!("src/f{i:03}.rs"), format!("movidos/otro{i:03}.rs"));
        fs::rename(env.repo.join(&from), env.repo.join(&to)).unwrap();
        expected.push(("entre_carpetas_con_otro_nombre", to, Some(from)));
        sleep_ms(120);
    }
    sleep_ms(3_000);
    let rows = env.rows();
    let mut results: BTreeMap<&str, (u32, u32, u32)> = BTreeMap::new();
    for (kind, to, from) in &expected {
        let entry = results.entry(kind).or_default();
        entry.0 += 1;
        match rows.get(to).and_then(|r| r.last()) {
            Some(r) if r.change == "renamed" && r.from == *from => entry.1 += 1,
            Some(r) if r.change == "created" => entry.2 += 1,
            _ => {}
        }
    }
    let report = json!({
        "results": results.iter().map(|(k, (total, renamed, as_create))| json!({
            "case": k, "total": total, "renamed_with_from": renamed, "delete_plus_create": as_create
        })).collect::<Vec<_>>(),
    });
    let text = serde_json::to_string_pretty(&report).unwrap();
    fs::write(out, &text).unwrap();
    println!("{text}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = PathBuf::from(args.get(2).map_or("gate.json", String::as_str));
    match args.get(1).map(String::as_str) {
        Some("g5") => g5(&out),
        Some("g7") => g7(&out),
        _ => eprintln!("uso: workspace_gates g5|g7 <salida.json>"),
    }
}
