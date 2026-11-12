//! Puerta G6 (SPEC §13.2): ¿el esquema de `update_plan` del Desktop es estable y
//! el adaptador A3 reproduce el plan que ve el Desktop?
//!
//! Los fixtures (`tests/fixtures/plan/desktop-<versión>.jsonl`) salen de
//! transcripts reales con `scripts/fixtures/extract-plan-fixtures.mjs`, sin ningún
//! contenido: cada paso se llama «Paso k». Cada llamada lleva la primera frase de
//! su resultado, que dice cuántos pasos tiene el plan y cuáles están en curso:
//!
//! - `Plan shown (7 steps).`
//! - `Plan updated: step 2 of 7 is in progress.`
//! - `Plan updated: steps 2, 3 of 7 are in progress; …`
//!
//! Cada llamada pasa por el parser de transcripts real (la misma lectura indulgente
//! de la ingesta) y el estado reproducido tiene que coincidir con ese resultado.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use cockpit_core::model::EventKind;
use cockpit_core::privacy::PrivacyPolicy;
use cockpit_core::tasks::{TaskOp, TaskSource, TaskStatus, replay};
use cockpit_core::transcript::TranscriptParser;
use serde_json::{Value, json};

#[derive(Debug, PartialEq)]
struct Expected {
    total: usize,
    in_progress: Option<Vec<usize>>,
}

fn expected(result: &str) -> Option<Expected> {
    let number = |s: &str| s.trim().parse::<usize>().ok();
    if let Some(rest) = result.strip_prefix("Plan shown (") {
        let n = rest.split(' ').next().and_then(number)?;
        return Some(Expected {
            total: n,
            in_progress: None,
        });
    }
    let rest = result.strip_prefix("Plan updated: ")?;
    let (steps, tail) = if let Some(r) = rest.strip_prefix("step ") {
        r.split_once(" of ")?
    } else {
        rest.strip_prefix("steps ")?.split_once(" of ")?
    };
    let total = tail.split(' ').next().and_then(number)?;
    let in_progress = steps.split(',').map(number).collect::<Option<Vec<_>>>()?;
    Some(Expected {
        total,
        in_progress: Some(in_progress),
    })
}

#[derive(Default, Debug)]
struct Tally {
    calls: usize,
    checked: usize,
    matched: usize,
    wrap_ups: usize,
    errors: usize,
}

#[test]
fn g6_desktop_plan_adapter_reproduces_the_desktop_state() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plan");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .expect("faltan los fixtures de G6")
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    assert!(!files.is_empty());

    let mut by_version: BTreeMap<String, Tally> = BTreeMap::new();
    let mut failures = Vec::new();
    for file in &files {
        // Un parser por fichero, como en la ingesta; el estado, por sesión.
        let mut parser = TranscriptParser::new(PrivacyPolicy::APP_DEFAULT);
        let mut ops: BTreeMap<String, Vec<(i64, TaskOp)>> = BTreeMap::new();
        for (n, line) in fs::read_to_string(file).unwrap().lines().enumerate() {
            let rec: Value = serde_json::from_str(line).unwrap();
            let version = rec["version"].as_str().unwrap_or("?").to_owned();
            let tally = by_version.entry(version.clone()).or_default();
            tally.calls += 1;
            if rec["is_error"] == json!(true) {
                tally.errors += 1;
                continue;
            }
            let session = rec["session"].as_str().unwrap().to_owned();
            if rec["tool"] == json!("wrap_up") {
                // `wrap_up` no cambia el plan que ven las llamadas siguientes.
                tally.wrap_ups += 1;
                continue;
            }
            let ts = 1_762_790_000_000 + n as i64 * 1000;
            let line = json!({
                "type": "assistant",
                "sessionId": session,
                "timestamp": cockpit_core_time(ts),
                "version": version,
                "message": {
                    "id": format!("m{n}"),
                    "model": "claude-test",
                    "usage": {"input_tokens": 1, "output_tokens": 1},
                    "content": [{
                        "type": "tool_use",
                        "id": format!("toolu_{n}"),
                        "name": "mcp__ccd_turn__update_plan",
                        "input": rec["input"],
                    }],
                },
            })
            .to_string();
            let mut events = Vec::new();
            parser.feed(line.as_bytes(), &mut events).unwrap();
            assert!(parser.take_notes().is_empty(), "deriva en {line}");
            let op = events
                .into_iter()
                .find_map(|e| match e.kind {
                    EventKind::TaskOp { source, op, .. } => {
                        assert_eq!(source, TaskSource::DesktopPlan);
                        Some(op)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("sin operación para {line}"));
            let list = ops.entry(session.clone()).or_default();
            list.push((ts, op));

            let Some(want) = rec["result"].as_str().and_then(expected) else {
                continue;
            };
            tally.checked += 1;
            let state = replay(TaskSource::DesktopPlan, list);
            let in_progress: Vec<usize> = state
                .iter()
                .enumerate()
                .filter(|(_, t)| t.status == TaskStatus::InProgress)
                .map(|(i, _)| i + 1)
                .collect();
            let got = Expected {
                total: state.len(),
                in_progress: want.in_progress.as_ref().map(|_| in_progress),
            };
            if got == want {
                tally.matched += 1;
            } else {
                failures.push(format!(
                    "{} {session}#{}: {:?} → {:?}, esperado {:?}",
                    file.file_name().unwrap().to_string_lossy(),
                    rec["seq"],
                    rec["input"],
                    got,
                    want
                ));
            }
        }
    }

    println!("G6 · versión · llamadas · comprobadas · coinciden · wrap_up · errores");
    for (version, t) in &by_version {
        println!(
            "G6 · {version} · {} · {} · {} · {} · {}",
            t.calls, t.checked, t.matched, t.wrap_ups, t.errors
        );
    }
    // El método de G6 pide 2.1.317 y 2.1.321: tienen que estar.
    for required in ["2.1.317", "2.1.321"] {
        let t = by_version
            .get(required)
            .unwrap_or_else(|| panic!("faltan fixtures de {required}"));
        assert!(t.checked > 0, "{required} sin llamadas comprobables");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// RFC 3339 en UTC con milisegundos, como el transcript.
fn cockpit_core_time(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Algoritmo de días civiles (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}
