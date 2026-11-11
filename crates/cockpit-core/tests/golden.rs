//! Fixtures dorados del parser de transcripts (SPEC §15, F1).
//!
//! Los `.jsonl` de `tests/fixtures/` son **sintéticos**: reproducen la forma de las
//! líneas de cada motor sin ningún dato real. Cada uno tiene su `.golden.json` con
//! los eventos esperados. Para regenerarlos tras un cambio intencionado:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test -p cockpit-core --test golden
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use cockpit_core::model::{CockpitEvent, EventKind, TokenUsage};
use cockpit_core::privacy::PrivacyPolicy;
use cockpit_core::transcript::{LineClass, TranscriptParser};

/// Cadena plantada en todos los campos que nunca deben salir del parser.
const TRAP: &str = "SECRETO-TRAMPA";

const FIXTURES: [&str; 2] = ["session-2.1.321", "session-2.1.128"];

fn fixture_path(name: &str, ext: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(format!("{name}.{ext}"))
}

struct Parsed {
    events: Vec<CockpitEvent>,
    unknown: Vec<String>,
}

fn parse(name: &str, policy: PrivacyPolicy) -> Parsed {
    let text = fs::read_to_string(fixture_path(name, "jsonl")).unwrap();
    let mut parser = TranscriptParser::new(policy);
    let mut events = Vec::new();
    let mut unknown = Vec::new();
    for (i, line) in text.lines().enumerate() {
        match parser.feed(line.as_bytes(), &mut events) {
            Ok(LineClass::Unknown(kind)) => unknown.push(kind),
            Ok(_) => {}
            Err(e) => panic!("{name}, línea {}: {e}", i + 1),
        }
    }
    parser.finish(&mut events);
    Parsed { events, unknown }
}

#[test]
fn events_match_the_golden_files() {
    for name in FIXTURES {
        let parsed = parse(name, PrivacyPolicy::APP_DEFAULT);
        let actual = serde_json::to_string_pretty(&parsed.events).unwrap() + "\n";
        let golden = fixture_path(name, "golden.json");
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            fs::write(&golden, &actual).unwrap();
            continue;
        }
        let expected = fs::read_to_string(&golden)
            .unwrap_or_else(|_| panic!("falta {}; genera con UPDATE_GOLDEN=1", golden.display()))
            .replace("\r\n", "\n");
        assert_eq!(
            actual, expected,
            "{name}: los eventos difieren del fichero dorado"
        );
    }
}

/// R17: ningún campo de contenido sale del parser, con ninguna política.
#[test]
fn trap_content_never_reaches_the_events() {
    let shell = PrivacyPolicy {
        capture_shell_program: true,
        ..PrivacyPolicy::APP_DEFAULT
    };
    for name in FIXTURES {
        let raw = fs::read_to_string(fixture_path(name, "jsonl")).unwrap();
        assert!(
            raw.contains(TRAP),
            "{name}: el fixture debe contener la trampa"
        );
        for policy in [
            PrivacyPolicy::APP_DEFAULT,
            PrivacyPolicy::PRE_HANDSHAKE,
            shell,
        ] {
            let parsed = parse(name, policy);
            let json = serde_json::to_string(&parsed.events).unwrap();
            assert!(
                !json.contains(TRAP),
                "{name}: la trampa ha llegado a los eventos"
            );
            assert!(
                !json.contains("privado"),
                "{name}: se ha filtrado una ruta externa"
            );
            assert!(
                !json.contains("token="),
                "{name}: se ha filtrado la consulta de una URL"
            );
        }
    }
}

#[test]
fn pre_handshake_policy_emits_no_paths_at_all() {
    for name in FIXTURES {
        let parsed = parse(name, PrivacyPolicy::PRE_HANDSHAKE);
        for e in &parsed.events {
            match &e.kind {
                EventKind::SessionSeen { cwd, .. } => assert_eq!(*cwd, None),
                EventKind::ToolStarted { meta, .. } => {
                    assert_eq!(meta.rel_path, None);
                    assert_eq!(meta.host, None);
                }
                _ => {}
            }
        }
        let json = serde_json::to_string(&parsed.events).unwrap();
        assert!(!json.contains("work"), "{name}: queda un fragmento de ruta");
        assert!(!json.contains("example.com"), "{name}: queda un host");
    }
}

/// M7: deduplicado por `message_id`, el uso coincide con la línea `cost-state`.
#[test]
fn deduplicated_usage_equals_cost_state() {
    for name in FIXTURES {
        let parsed = parse(name, PrivacyPolicy::APP_DEFAULT);
        let mut by_message: HashMap<&str, TokenUsage> = HashMap::new();
        let mut rows = 0;
        let mut cost_state = None;
        for e in &parsed.events {
            match &e.kind {
                EventKind::ApiMessage {
                    message_id, usage, ..
                } => {
                    rows += 1;
                    let entry = by_message.entry(message_id).or_default();
                    if usage.output >= entry.output {
                        *entry = *usage;
                    }
                }
                EventKind::CostState { usage, .. } => cost_state = Some(*usage),
                _ => {}
            }
        }
        let mut total = TokenUsage::default();
        by_message.values().for_each(|u| total.add(u));
        assert_eq!(rows, 6, "{name}: filas de assistant con uso");
        assert_eq!(by_message.len(), 4, "{name}: mensajes únicos");
        assert_eq!(Some(total), cost_state, "{name}");
    }
}

#[test]
fn schema_drift_is_reported_by_type_name_only() {
    let modern = parse("session-2.1.321", PrivacyPolicy::APP_DEFAULT);
    assert_eq!(modern.unknown, ["tipo-del-futuro"]);
    let old = parse("session-2.1.128", PrivacyPolicy::APP_DEFAULT);
    assert!(old.unknown.is_empty());
}

#[test]
fn request_blobs_do_not_create_tool_events() {
    let parsed = parse("session-2.1.321", PrivacyPolicy::APP_DEFAULT);
    let ids: Vec<&str> = parsed
        .events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::ToolStarted { tool_use_id, .. } => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        ids,
        [
            "toolu_edit",
            "toolu_bash",
            "toolu_read",
            "toolu_fetch",
            "toolu_grep",
            "toolu_plan",
            "toolu_agent"
        ]
    );
}
