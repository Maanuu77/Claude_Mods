//! Protocolo mod ↔ app de extremo a extremo (SPEC §6.2.1): un cliente HTTP hace de
//! mod contra la canalización real, con su SQLite y su `LiveState`.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use cockpit_core::privacy::PrivacyPolicy;
use cockpit_ingest::live::{LiveConfig, LiveDelta, LivePipeline, LiveState, SessionView, now_ms};

const SESSION: &str = "44444444-4444-4444-8444-444444444444";
const INSTANCE: &str = "inst-0001";

struct Client {
    port: u16,
    token: String,
}

impl Client {
    fn new(dir: &Path) -> Self {
        let raw = std::fs::read(dir.join("endpoint.json")).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        Self {
            port: v["port"].as_u64().unwrap() as u16,
            token: v["token"].as_str().unwrap().to_owned(),
        }
    }

    fn post(&self, path: &str, body: &str) -> (u16, serde_json::Value) {
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
        let status = raw
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let body = raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
        (
            status,
            serde_json::from_str(body).unwrap_or(serde_json::Value::Null),
        )
    }
}

fn hello(engine: &str, hello_seq: i64, now: i64) -> String {
    format!(
        r#"{{"schema":1,
            "mod":{{"name":"dev-cockpit-bridge","version":"0.1.0","instance_id":"{INSTANCE}"}},
            "engine":{{"version":"{engine}","base":"{engine}"}},
            "session":{{"id":"{SESSION}","surfaces":["terminal"],"is_interactive":true,"started_at":{}}},
            "reason":"startup","hello_seq":{hello_seq},
            "applied_policy":{{"id":"pre_handshake","capture_task_titles":false,"record_paths":false,"capture_shell_program":false}},
            "snapshot":{{
                "usage":{{"context":{{"tokens":86000,"window":200000,"percent":43}},
                          "rate_limits":[{{"kind":"five_hour","percent_used":37,"resets_at":"2026-11-11T20:00:00Z"}}],
                          "cost_usd":1.25}},
                "agents":[],
                "running_tools":[{{"tool_use_id":"toolu_running","tool":"Bash","started_at":{},"meta":{{}}}}]}},
            "queue":{{"pending":0,"dropped":0,"next_seq":1}}}}"#,
        now - 60_000,
        now - 2_000
    )
}

fn events(now: i64) -> String {
    format!(
        r#"{{"schema":1,"instance_id":"{INSTANCE}","session_id":"{SESSION}","events":[
            {{"kind":"tool_started","seq":1,"t":{},"tool_use_id":"toolu_edit","tool":"Edit","meta":{{"rel_path":"src/main.rs","replace_all":false}}}},
            {{"kind":"tool_finished","seq":2,"t":{},"tool_use_id":"toolu_edit","status":"ok"}},
            {{"kind":"tool_finished","seq":3,"t":{},"tool_use_id":"toolu_running","status":"interrupted"}},
            {{"kind":"turn_completed","seq":4,"t":{now},"turn_id":"turn-1","duration_ms":4200,"is_aborted":false,
              "usage":{{"input_tokens":10,"output_tokens":300,"cache_read_input_tokens":5000,"cache_creation_input_tokens":40,"model":"claude-test"}}}},
            {{"kind":"usage_snapshot","seq":5,"t":{now},"usage":{{"context":{{"tokens":90000,"window":200000,"percent":45}},"rate_limits":[],"cost_usd":1.5}}}}
        ]}}"#,
        now - 900,
        now - 400,
        now - 300
    )
}

fn session(state: &LiveState) -> Option<SessionView> {
    match state.snapshot() {
        LiveDelta::Snapshot { sessions, .. } => {
            sessions.into_iter().find(|s| s.summary.id == SESSION)
        }
        LiveDelta::Update { .. } => None,
    }
}

fn wait_for<T>(mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(v) = probe() {
            return v;
        }
        assert!(Instant::now() < deadline, "sin respuesta en 10 s");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn hello_then_events_is_idempotent_and_feeds_the_live_state() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("projects")).unwrap();
    let state = LiveState::new(|| {});
    let pipeline = LivePipeline::spawn(
        LiveConfig {
            projects_root: dir.path().join("projects"),
            sessions_dir: None,
            db_path: dir.path().join("cockpit.db"),
            endpoint_path: Some(dir.path().join("endpoint.json")),
            policy: PrivacyPolicy::APP_DEFAULT,
            desktop_plan: true,
        },
        state.clone(),
    )
    .unwrap();
    let client = Client::new(dir.path());
    let now = now_ms();

    // Sin `hello` previo, los eventos se rechazan con 409.
    assert_eq!(client.post("/v1/events", &events(now)).0, 409);

    // Un motor anterior al mínimo se rechaza sin registrar nada.
    let (status, old) = client.post("/v1/hello", &hello("2.1.285", 1, now));
    assert_eq!(status, 200);
    assert_eq!(old["engine_supported"], false);
    assert_eq!(client.post("/v1/events", &events(now)).0, 409);

    // Handshake: la app responde con su política y la marca de agua.
    let (status, reply) = client.post("/v1/hello", &hello("2.1.321", 2, now));
    assert_eq!(status, 200);
    assert_eq!(reply["engine_supported"], true);
    assert_eq!(reply["min_engine"], "2.1.289");
    assert_eq!(reply["events"]["ack_seq"], 0);
    assert_eq!(reply["privacy"]["record_paths"], true);
    assert_eq!(reply["privacy"]["capture_shell_program"], false);

    // El resync ya pinta la herramienta en curso y el uso, sin ningún evento.
    let view = wait_for(|| session(&state).filter(|s| s.mod_connected));
    assert_eq!(view.live, None);
    assert_eq!(view.summary.running_tools.len(), 1);
    assert_eq!(view.summary.running_tools[0].tool, "Bash");
    assert_eq!(
        view.summary
            .usage_snapshot
            .as_ref()
            .unwrap()
            .context_percent,
        Some(43.0)
    );

    // Lote de eventos, y el mismo lote repetido: nada se duplica.
    for _ in 0..2 {
        let (status, ack) = client.post("/v1/events", &events(now));
        assert_eq!(status, 200);
        assert_eq!(ack["ack_seq"], 5);
    }
    let view = session(&state).unwrap();
    assert_eq!(view.summary.tool_calls, 2);
    assert!(view.summary.running_tools.is_empty());
    let edit = view
        .summary
        .recent_tools
        .iter()
        .find(|t| t.tool == "Edit")
        .unwrap();
    assert_eq!((edit.status.as_str(), edit.duration_ms), ("ok", Some(500)));
    assert_eq!(edit.rel_path.as_deref(), Some("src/main.rs"));
    let bash = view
        .summary
        .recent_tools
        .iter()
        .find(|t| t.tool == "Bash")
        .unwrap();
    assert_eq!(bash.status, "interrupted");
    let usage = view.summary.usage_snapshot.unwrap();
    assert_eq!(
        (usage.context_percent, usage.cost_usd),
        (Some(45.0), Some(1.5))
    );
    assert!(usage.rate_limits.is_empty());

    // G3: una herramienta en curso pasa a esperar permiso, vuelve y termina.
    let send = |seq: i64, event: &str| {
        let body = format!(
            r#"{{"schema":1,"instance_id":"{INSTANCE}","session_id":"{SESSION}","events":[
                {{"seq":{seq},"t":{now},"tool_use_id":"toolu_wait",{event}}}]}}"#
        );
        assert_eq!(client.post("/v1/events", &body).0, 200);
        session(&state).unwrap().summary
    };
    let running = |summary: &cockpit_store::SessionSummary| -> Vec<String> {
        summary
            .running_tools
            .iter()
            .map(|t| t.status.clone())
            .collect()
    };
    assert_eq!(
        running(&send(6, r#""kind":"tool_started","tool":"Write""#)),
        ["running"]
    );
    assert_eq!(
        running(&send(7, r#""kind":"permission_requested""#)),
        ["awaiting_permission"]
    );
    assert_eq!(
        running(&send(8, r#""kind":"permission_resolved""#)),
        ["running"]
    );
    assert_eq!(
        running(&send(9, r#""kind":"permission_requested""#)),
        ["awaiting_permission"]
    );
    let done = send(10, r#""kind":"tool_finished","status":"ok""#);
    assert!(done.running_tools.is_empty());
    assert_eq!(
        done.recent_tools
            .iter()
            .find(|t| t.tool == "Write")
            .unwrap()
            .status,
        "ok"
    );
    // Un permiso que llega tarde no reabre una herramienta terminada.
    assert!(
        send(11, r#""kind":"permission_requested""#)
            .running_tools
            .is_empty()
    );

    // Un `hello` repetido devuelve la marca de agua y no reabre nada.
    let (_, again) = client.post("/v1/hello", &hello("2.1.321", 2, now));
    assert_eq!(again["events"]["ack_seq"], 11);
    assert_eq!(again["session"]["known"], true);
    assert!(session(&state).unwrap().summary.running_tools.is_empty());

    // Un campo fuera de la lista blanca tumba la petición entera.
    let leaked = events(now).replace(
        r#""rel_path":"src/main.rs""#,
        r#""rel_path":"a","old_string":"SECRETO""#,
    );
    assert_eq!(client.post("/v1/events", &leaked).0, 422);

    // Cierre limpio: desaparece `endpoint.json`.
    drop(pipeline);
    assert!(!dir.path().join("endpoint.json").exists());
}

/// Task Monitor por el mod (SPEC §8): operaciones de tareas, hooks y subagentes.
#[test]
fn task_ops_and_agents_reach_the_session_view() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("projects")).unwrap();
    let state = LiveState::new(|| {});
    let _pipeline = LivePipeline::spawn(
        LiveConfig {
            projects_root: dir.path().join("projects"),
            sessions_dir: None,
            db_path: dir.path().join("cockpit.db"),
            endpoint_path: Some(dir.path().join("endpoint.json")),
            policy: PrivacyPolicy::APP_DEFAULT,
            desktop_plan: true,
        },
        state.clone(),
    )
    .unwrap();
    let client = Client::new(dir.path());
    let now = now_ms();
    assert_eq!(client.post("/v1/hello", &hello("2.1.321", 1, now)).0, 200);

    let batch = format!(
        r#"{{"schema":1,"instance_id":"{INSTANCE}","session_id":"{SESSION}","events":[
            {{"kind":"task_op","seq":1,"t":{now},"tool_use_id":"toolu_p1","source":"desktop_plan",
              "op":{{"type":"replace","items":[{{"subject":"Leer la SPEC","status":"in_progress"}},{{"subject":"Escribir","status":"pending"}},{{"status":"pending"}}]}}}},
            {{"kind":"task_op","seq":2,"t":{},"tool_use_id":"toolu_p2","source":"desktop_plan","op":{{"type":"advance"}}}},
            {{"kind":"task_op","seq":3,"t":{now},"source":"task_tools","op":{{"type":"create","task_id":"1","subject":"Revisar"}}}},
            {{"kind":"task_op","seq":4,"t":{},"tool_use_id":"toolu_u1","agent_id":"a1","source":"task_tools",
              "op":{{"type":"update","task_id":"1","status":"in_progress","owner":"ana","blocked_by":["2"]}}}},
            {{"kind":"task_op","seq":5,"t":{now},"tool_use_id":"toolu_bad","source":"desktop_plan","op":{{"type":"create","task_id":"9"}}}},
            {{"kind":"agents","seq":6,"t":{now},"agents":[
                {{"id":"a1","type":"Explore","status":"running"}},
                {{"id":"a2","type":"general-purpose","status":"completed","parent_id":"a1"}}]}}
        ]}}"#,
        now + 1,
        now + 2
    );
    let (status, ack) = client.post("/v1/events", &batch);
    assert_eq!(status, 200);
    assert_eq!(ack["ack_seq"], 6);

    let view = wait_for(|| session(&state).filter(|s| s.summary.tasks.len() == 2)).summary;
    let plan = view
        .tasks
        .iter()
        .find(|l| l.source == cockpit_core::tasks::TaskSource::DesktopPlan)
        .unwrap();
    let statuses: Vec<_> = plan.items.iter().map(|i| i.status.as_str()).collect();
    assert_eq!(statuses, ["completed", "in_progress", "pending"]);
    assert_eq!((plan.progress.completed, plan.progress.total), (1, 3));
    // Sin título: la interfaz lo numera.
    assert_eq!(plan.items[2].subject, None);
    let tasks = view
        .tasks
        .iter()
        .find(|l| l.source == cockpit_core::tasks::TaskSource::TaskTools)
        .unwrap();
    assert_eq!(tasks.items.len(), 1, "un `create` en A3 se descarta");
    assert_eq!(tasks.items[0].owner.as_deref(), Some("ana"));
    assert_eq!(tasks.items[0].blocked_by, ["2"]);
    assert_eq!(view.agents.len(), 2);
    assert_eq!(view.agents[1].parent_id.as_deref(), Some("a1"));

    // Un campo fuera de la lista blanca en una operación tumba el lote.
    let leaked = format!(
        r#"{{"schema":1,"instance_id":"{INSTANCE}","session_id":"{SESSION}","events":[
            {{"kind":"task_op","seq":7,"t":{now},"tool_use_id":"toolu_x","source":"todo_write",
              "op":{{"type":"replace","items":[{{"subject":"a","status":"pending","activeForm":"SECRETO"}}]}}}}]}}"#
    );
    assert_eq!(client.post("/v1/events", &leaked).0, 422);
}
