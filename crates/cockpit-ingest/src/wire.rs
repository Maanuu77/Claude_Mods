//! Contrato JSON entre el mod y la app (SPEC §6.2.1).
//!
//! Todos los tipos usan `deny_unknown_fields`: un campo fuera de la lista blanca
//! hace que se rechace la petición entera (422). Es el segundo punto de aplicación
//! de la política de privacidad de §10, independiente del filtro del mod.

use cockpit_core::model::{CockpitEvent, EventKind, RateLimit, Source, TokenUsage, ToolStatus};
use cockpit_core::privacy::{EXTERNAL_PATH, MetaValue, PrivacyPolicy, ToolMeta};
use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelloRequest {
    pub schema: u32,
    #[serde(rename = "mod")]
    pub module: ModInfo,
    pub engine: EngineInfo,
    pub session: SessionInfo,
    pub reason: String,
    pub hello_seq: i64,
    pub applied_policy: AppliedPolicy,
    pub snapshot: Snapshot,
    pub queue: QueueInfo,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModInfo {
    pub name: String,
    pub version: String,
    pub instance_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineInfo {
    pub version: String,
    #[serde(default)]
    pub base: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionInfo {
    pub id: String,
    #[serde(default)]
    pub surfaces: Vec<String>,
    pub is_interactive: bool,
    pub started_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedPolicy {
    pub id: String,
    pub capture_task_titles: bool,
    pub record_paths: bool,
    pub capture_shell_program: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    #[serde(default)]
    pub usage: Option<UsageWire>,
    /// Aceptado por contrato. Lo consume el Task Monitor (F4b).
    #[serde(default)]
    pub agents: Vec<AgentWire>,
    #[serde(default)]
    pub running_tools: Vec<RunningTool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentWire {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
    #[serde(default)]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueInfo {
    pub pending: u64,
    pub dropped: u64,
    pub next_seq: i64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageWire {
    #[serde(default)]
    pub context: Option<ContextWire>,
    #[serde(default)]
    pub rate_limits: Vec<RateLimitWire>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextWire {
    #[serde(default)]
    pub tokens: Option<u64>,
    #[serde(default)]
    pub window: Option<u64>,
    #[serde(default)]
    pub percent: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitWire {
    pub kind: String,
    pub percent_used: f64,
    #[serde(default)]
    pub resets_at: Option<String>,
}

/// Metadatos de una herramienta: exactamente la lista blanca de §10.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolMetaWire {
    #[serde(default)]
    pub rel_path: Option<String>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub ext: Option<String>,
    #[serde(default)]
    pub replace_all: Option<bool>,
    #[serde(default)]
    pub run_in_background: Option<bool>,
    #[serde(default)]
    pub program: Option<String>,
    #[serde(default)]
    pub subagent_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunningTool {
    pub tool_use_id: String,
    pub tool: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub started_at: i64,
    #[serde(default)]
    pub meta: ToolMetaWire,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnUsageWire {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub model: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventsRequest {
    pub schema: u32,
    pub instance_id: String,
    pub session_id: String,
    pub events: Vec<WireEvent>,
}

/// Un evento del mod. `seq` es estrictamente creciente por instancia y `t` es el
/// reloj de pared del mod en milisegundos.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WireEvent {
    ToolStarted {
        seq: i64,
        t: i64,
        tool_use_id: String,
        tool: String,
        #[serde(default)]
        agent_id: Option<String>,
        #[serde(default)]
        meta: ToolMetaWire,
    },
    ToolFinished {
        seq: i64,
        t: i64,
        tool_use_id: String,
        status: WireToolStatus,
        #[serde(default)]
        agent_id: Option<String>,
    },
    TurnCompleted {
        seq: i64,
        t: i64,
        turn_id: String,
        #[serde(default)]
        agent_id: Option<String>,
        duration_ms: u64,
        is_aborted: bool,
        #[serde(default)]
        usage: Option<TurnUsageWire>,
    },
    UsageSnapshot {
        seq: i64,
        t: i64,
        usage: UsageWire,
    },
    SessionEnded {
        seq: i64,
        t: i64,
        #[serde(default)]
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireToolStatus {
    Ok,
    Error,
    Interrupted,
}

impl WireEvent {
    pub fn seq(&self) -> i64 {
        match self {
            WireEvent::ToolStarted { seq, .. }
            | WireEvent::ToolFinished { seq, .. }
            | WireEvent::TurnCompleted { seq, .. }
            | WireEvent::UsageSnapshot { seq, .. }
            | WireEvent::SessionEnded { seq, .. } => *seq,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HelloResponse {
    pub schema: u32,
    pub server: ServerInfo,
    pub privacy: PrivacyWire,
    pub session: SessionKnown,
    pub events: Ack,
    pub min_engine: &'static str,
    pub engine_supported: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServerInfo {
    pub app_version: &'static str,
    pub accepted_schemas: [u32; 1],
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrivacyWire {
    pub policy_id: &'static str,
    pub capture_task_titles: bool,
    pub record_paths: bool,
    pub capture_shell_program: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionKnown {
    pub known: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Ack {
    pub ack_seq: i64,
}

// ---------------------------------------------------------------------------
// Conversión a eventos canónicos
// ---------------------------------------------------------------------------

/// Identificador corto y limpio: nombres de herramienta, ids y códigos.
fn ident(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':' | '.'))
        .take(max)
        .collect()
}

fn is_absolute(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with('/')
        || path.starts_with('\\')
        || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// La app no se fía del mod: vuelve a aplicar su política a lo recibido.
pub fn tool_meta(wire: &ToolMetaWire, policy: &PrivacyPolicy) -> ToolMeta {
    let mut meta = ToolMeta::default();
    if policy.record_paths {
        meta.rel_path = wire.rel_path.as_deref().map(|p| {
            // Una ruta absoluta o con `..` nunca debería llegar: se degrada.
            if is_absolute(p) || p.split(['/', '\\']).any(|s| s == "..") || p.len() > 1024 {
                EXTERNAL_PATH.to_owned()
            } else {
                p.replace('\\', "/")
            }
        });
        meta.host = wire
            .host
            .as_deref()
            .map(|h| ident(h, 253).to_ascii_lowercase());
    }
    let mut put = |key: &str, value: MetaValue| {
        meta.meta.insert(key.to_owned(), value);
    };
    if let Some(ext) = &wire.ext {
        put("ext", MetaValue::Str(ident(ext, 13)));
    }
    if let Some(v) = wire.replace_all {
        put("replace_all", MetaValue::Bool(v));
    }
    if let Some(v) = wire.run_in_background {
        put("run_in_background", MetaValue::Bool(v));
    }
    if policy.capture_shell_program
        && let Some(program) = &wire.program
    {
        put("program", MetaValue::Str(ident(program, 64)));
    }
    if let Some(kind) = &wire.subagent_type {
        put("subagent_type", MetaValue::Str(ident(kind, 64)));
    }
    meta
}

pub fn usage_event(usage: &UsageWire) -> EventKind {
    let context = usage.context.as_ref();
    EventKind::UsageSnapshot {
        context_tokens: context.and_then(|c| c.tokens),
        context_window: context.and_then(|c| c.window),
        context_percent: context.and_then(|c| c.percent),
        rate_limits: usage
            .rate_limits
            .iter()
            .map(|r| RateLimit {
                kind: ident(&r.kind, 48),
                percent_used: r.percent_used,
                resets_at: r.resets_at.as_deref().map(|s| ident(s, 40)),
            })
            .collect(),
        cost_usd: usage.cost_usd,
    }
}

pub fn mod_event(
    session_id: &str,
    agent_id: Option<&str>,
    ts_ms: i64,
    engine_version: Option<&str>,
    kind: EventKind,
) -> CockpitEvent {
    CockpitEvent {
        source: Source::Mod,
        session_id: session_id.to_owned(),
        agent_id: agent_id.map(|a| ident(a, 96)),
        ts_ms,
        engine_version: engine_version.map(str::to_owned),
        kind,
    }
}

/// Evento del mod → evento canónico, con la política de la app aplicada.
pub fn to_event(session_id: &str, event: &WireEvent, policy: &PrivacyPolicy) -> CockpitEvent {
    match event {
        WireEvent::ToolStarted {
            t,
            tool_use_id,
            tool,
            agent_id,
            meta,
            ..
        } => mod_event(
            session_id,
            agent_id.as_deref(),
            *t,
            None,
            EventKind::ToolStarted {
                tool_use_id: ident(tool_use_id, 128),
                tool: ident(tool, 96),
                meta: tool_meta(meta, policy),
            },
        ),
        WireEvent::ToolFinished {
            t,
            tool_use_id,
            status,
            agent_id,
            ..
        } => mod_event(
            session_id,
            agent_id.as_deref(),
            *t,
            None,
            EventKind::ToolFinished {
                tool_use_id: ident(tool_use_id, 128),
                status: match status {
                    WireToolStatus::Ok => ToolStatus::Ok,
                    WireToolStatus::Error => ToolStatus::Error,
                    WireToolStatus::Interrupted => ToolStatus::Interrupted,
                },
            },
        ),
        WireEvent::TurnCompleted {
            t,
            turn_id,
            agent_id,
            duration_ms,
            is_aborted,
            usage,
            ..
        } => mod_event(
            session_id,
            agent_id.as_deref(),
            *t,
            None,
            EventKind::TurnCompleted {
                turn_id: ident(turn_id, 128),
                duration_ms: *duration_ms,
                is_aborted: *is_aborted,
                model: usage.as_ref().map(|u| ident(&u.model, 96)),
                usage: usage.as_ref().map(|u| TokenUsage {
                    input: u.input_tokens,
                    output: u.output_tokens,
                    cache_read: u.cache_read_input_tokens,
                    cache_creation: u.cache_creation_input_tokens,
                }),
            },
        ),
        WireEvent::UsageSnapshot { t, usage, .. } => {
            mod_event(session_id, None, *t, None, usage_event(usage))
        }
        WireEvent::SessionEnded { t, reason, .. } => mod_event(
            session_id,
            None,
            *t,
            None,
            EventKind::SessionEnded {
                reason: reason.as_deref().map(|r| ident(r, 48)),
            },
        ),
    }
}

/// `a >= b` para versiones `x.y.z`. Una versión ilegible no se da por soportada.
pub fn version_at_least(version: &str, minimum: &str) -> bool {
    fn parse(v: &str) -> Option<Vec<u64>> {
        v.split('.')
            .map(|p| p.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok())
            .collect()
    }
    match (parse(version), parse(minimum)) {
        (Some(a), Some(b)) => a >= b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_fields_reject_the_whole_event() {
        let ok = r#"{"kind":"tool_started","seq":1,"t":5,"tool_use_id":"t1","tool":"Edit","meta":{"rel_path":"src/a.rs"}}"#;
        assert!(serde_json::from_str::<WireEvent>(ok).is_ok());
        for bad in [
            // argumentos de la herramienta colados en el evento
            r#"{"kind":"tool_started","seq":1,"t":5,"tool_use_id":"t1","tool":"Edit","input":{"old_string":"x"}}"#,
            r#"{"kind":"tool_started","seq":1,"t":5,"tool_use_id":"t1","tool":"Bash","meta":{"command":"rm -rf"}}"#,
            r#"{"kind":"tool_finished","seq":2,"t":6,"tool_use_id":"t1","status":"ok","result":"texto"}"#,
            r#"{"kind":"tipo_inventado","seq":3,"t":7}"#,
        ] {
            assert!(serde_json::from_str::<WireEvent>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_app_policy_is_applied_again_on_arrival() {
        let wire = ToolMetaWire {
            rel_path: Some("src/a.rs".to_owned()),
            host: Some("Example.COM".to_owned()),
            program: Some("git".to_owned()),
            replace_all: Some(true),
            ..Default::default()
        };
        let default = tool_meta(&wire, &PrivacyPolicy::APP_DEFAULT);
        assert_eq!(default.rel_path.as_deref(), Some("src/a.rs"));
        assert_eq!(default.host.as_deref(), Some("example.com"));
        assert_eq!(
            default.meta_json().as_deref(),
            Some(r#"{"replace_all":true}"#)
        );

        let strict = tool_meta(&wire, &PrivacyPolicy::PRE_HANDSHAKE);
        assert_eq!((strict.rel_path, strict.host), (None, None));
    }

    #[test]
    fn absolute_or_escaping_paths_are_degraded() {
        for bad in [
            r"C:\Users\x\secreto.txt",
            "/etc/passwd",
            "../fuera/a.rs",
            r"\\srv\share\a",
        ] {
            let wire = ToolMetaWire {
                rel_path: Some(bad.to_owned()),
                ..Default::default()
            };
            let meta = tool_meta(&wire, &PrivacyPolicy::APP_DEFAULT);
            assert_eq!(meta.rel_path.as_deref(), Some(EXTERNAL_PATH), "{bad}");
        }
    }

    #[test]
    fn engine_versions_compare_numerically() {
        assert!(version_at_least("2.1.289", "2.1.289"));
        assert!(version_at_least("2.1.321", "2.1.289"));
        assert!(version_at_least("2.10.0", "2.9.999"));
        assert!(!version_at_least("2.1.285", "2.1.289"));
        assert!(!version_at_least("desconocida", "2.1.289"));
    }
}
