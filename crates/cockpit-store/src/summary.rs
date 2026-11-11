//! Consultas de solo lectura para la vista En vivo (SPEC §6.5).
//!
//! La base de datos es la verdad: el estado en vivo es una caché de estos
//! resúmenes, que se vuelven a consultar para las sesiones que cambian.

use cockpit_core::model::{RateLimit, TokenUsage};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::{Result, Store};

/// Herramientas recientes que se devuelven por sesión.
const RECENT_TOOLS: i64 = 12;
/// Tope de herramientas en curso por sesión.
const MAX_RUNNING: i64 = 32;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolView {
    pub tool_use_id: String,
    pub tool: String,
    pub agent_id: Option<String>,
    pub status: String,
    pub started_at: i64,
    pub duration_ms: Option<i64>,
    pub rel_path: Option<String>,
}

/// Último `UsageSnapshot` del mod: contexto, límites y coste del motor.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageView {
    pub ts: i64,
    pub context_tokens: Option<i64>,
    pub context_window: Option<i64>,
    pub context_percent: Option<f64>,
    pub rate_limits: Vec<RateLimit>,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub cwd: Option<String>,
    pub entrypoint: Option<String>,
    pub engine_version: Option<String>,
    pub started_at: i64,
    pub last_seen_at: i64,
    /// Modelo de la respuesta más reciente.
    pub model: Option<String>,
    /// Tokens deduplicados por `message_id`.
    pub usage: TokenUsage,
    pub api_messages: u64,
    pub tool_calls: u64,
    pub tool_errors: u64,
    pub api_errors: u64,
    /// Coste de la última línea `cost-state`: una estimación del cliente.
    pub cost_state_usd: Option<f64>,
    /// Solo con el mod conectado. Sin él, la interfaz muestra «n/d».
    pub usage_snapshot: Option<UsageView>,
    pub running_tools: Vec<ToolView>,
    pub recent_tools: Vec<ToolView>,
}

fn tool_view(r: &rusqlite::Row<'_>) -> rusqlite::Result<ToolView> {
    Ok(ToolView {
        tool_use_id: r.get(0)?,
        tool: r.get(1)?,
        agent_id: r.get(2)?,
        status: r.get(3)?,
        started_at: r.get(4)?,
        duration_ms: r.get(5)?,
        rel_path: r.get(6)?,
    })
}

const TOOL_COLUMNS: &str = "tool_use_id, tool, agent_id, status, started_at, duration_ms, rel_path";

impl Store {
    /// Resumen de una sesión, o `None` si no existe.
    ///
    /// `running_since_ms` descarta las herramientas que se quedaron en `running`
    /// hace mucho (sesiones interrumpidas): no se muestran como en curso.
    pub fn session_summary(
        &self,
        id: &str,
        running_since_ms: i64,
    ) -> Result<Option<SessionSummary>> {
        let conn = self.connection();
        let Some((cwd, entrypoint, engine_version, started_at, last_seen_at, cost_state_usd)) =
            conn.prepare_cached(
                "SELECT cwd, entrypoint, engine_version, started_at, last_seen_at, cost_state_usd
                 FROM sessions WHERE id = ?1",
            )?
            .query_row([id], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, Option<f64>>(5)?,
                ))
            })
            .optional()?
        else {
            return Ok(None);
        };

        let (usage, api_messages) = conn
            .prepare_cached(
                "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                        COALESCE(SUM(cache_read_tokens), 0), COALESCE(SUM(cache_creation_tokens), 0),
                        COUNT(*)
                 FROM api_messages WHERE session_id = ?1",
            )?
            .query_row([id], |r| {
                Ok((
                    TokenUsage {
                        input: r.get::<_, i64>(0)? as u64,
                        output: r.get::<_, i64>(1)? as u64,
                        cache_read: r.get::<_, i64>(2)? as u64,
                        cache_creation: r.get::<_, i64>(3)? as u64,
                    },
                    r.get::<_, i64>(4)? as u64,
                ))
            })?;
        let model: Option<String> = conn
            .prepare_cached(
                "SELECT model FROM api_messages WHERE session_id = ?1 ORDER BY ts DESC LIMIT 1",
            )?
            .query_row([id], |r| r.get(0))
            .optional()?;
        let (tool_calls, tool_errors) = conn
            .prepare_cached(
                "SELECT COUNT(*), COALESCE(SUM(status = 'error'), 0)
                 FROM tool_calls WHERE session_id = ?1 AND is_copy = 0",
            )?
            .query_row([id], |r| {
                Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64))
            })?;
        let api_errors: i64 = conn
            .prepare_cached("SELECT COUNT(*) FROM api_errors WHERE session_id = ?1")?
            .query_row([id], |r| r.get(0))?;

        let usage_snapshot = conn
            .prepare_cached(
                "SELECT ts, context_tokens, context_window, context_percent, rate_limits_json, cost_usd
                 FROM usage_snapshots WHERE session_id = ?1 ORDER BY id DESC LIMIT 1",
            )?
            .query_row([id], |r| {
                let limits: String = r.get(4)?;
                Ok(UsageView {
                    ts: r.get(0)?,
                    context_tokens: r.get(1)?,
                    context_window: r.get(2)?,
                    context_percent: r.get(3)?,
                    rate_limits: serde_json::from_str(&limits).unwrap_or_default(),
                    cost_usd: r.get(5)?,
                })
            })
            .optional()?;

        let running_tools = conn
            .prepare_cached(&format!(
                "SELECT {TOOL_COLUMNS} FROM tool_calls
                 WHERE session_id = ?1 AND is_copy = 0 AND status = 'running' AND started_at >= ?2
                 ORDER BY started_at LIMIT ?3"
            ))?
            .query_map(params![id, running_since_ms, MAX_RUNNING], tool_view)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let recent_tools = conn
            .prepare_cached(&format!(
                "SELECT {TOOL_COLUMNS} FROM tool_calls
                 WHERE session_id = ?1 AND is_copy = 0
                 ORDER BY started_at DESC, tool_use_id LIMIT ?2"
            ))?
            .query_map(params![id, RECENT_TOOLS], tool_view)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(Some(SessionSummary {
            id: id.to_owned(),
            cwd,
            entrypoint,
            engine_version,
            started_at,
            last_seen_at,
            model,
            usage,
            api_messages,
            tool_calls,
            tool_errors,
            api_errors: api_errors as u64,
            cost_state_usd,
            usage_snapshot,
            running_tools,
            recent_tools,
        }))
    }

    /// Ids de las sesiones con actividad desde `since_ms`, de la más reciente a la más antigua.
    pub fn sessions_active_since(&self, since_ms: i64) -> Result<Vec<String>> {
        let mut stmt = self.connection().prepare_cached(
            "SELECT id FROM sessions WHERE last_seen_at >= ?1 ORDER BY last_seen_at DESC, id",
        )?;
        let ids = stmt
            .query_map([since_ms], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Batch;
    use cockpit_core::model::{CockpitEvent, EventKind, Source, ToolStatus};
    use cockpit_core::privacy::ToolMeta;

    const T0: i64 = 1_794_304_800_000;

    fn ev(ts: i64, agent: Option<&str>, kind: EventKind) -> CockpitEvent {
        CockpitEvent {
            source: Source::Transcript,
            session_id: "s1".to_owned(),
            agent_id: agent.map(str::to_owned),
            ts_ms: ts,
            engine_version: Some("2.1.321".to_owned()),
            kind,
        }
    }

    fn tool(ts: i64, id: &str, name: &str, agent: Option<&str>) -> CockpitEvent {
        ev(
            ts,
            agent,
            EventKind::ToolStarted {
                tool_use_id: id.to_owned(),
                tool: name.to_owned(),
                meta: ToolMeta::default(),
            },
        )
    }

    fn message(ts: i64, id: &str, model: &str, output: u64) -> CockpitEvent {
        ev(
            ts,
            None,
            EventKind::ApiMessage {
                message_id: id.to_owned(),
                model: model.to_owned(),
                usage: TokenUsage {
                    input: 1,
                    output,
                    cache_read: 0,
                    cache_creation: 0,
                },
                stop_reason: None,
            },
        )
    }

    #[test]
    fn summary_reflects_the_session_state() {
        let mut store = Store::open_in_memory().unwrap();
        let events = [
            ev(
                T0,
                None,
                EventKind::SessionSeen {
                    cwd: Some(r"C:\work\demo".to_owned()),
                    entrypoint: Some("cli".to_owned()),
                },
            ),
            message(T0 + 1, "m1", "modelo-a", 10),
            message(T0 + 2, "m1", "modelo-a", 40),
            message(T0 + 50, "m2", "modelo-b", 5),
            tool(T0 + 3, "t-vieja", "Bash", None),
            tool(T0 + 10_000, "t1", "Edit", None),
            tool(T0 + 10_001, "t2", "Read", Some("agente-1")),
            ev(
                T0 + 10_500,
                None,
                EventKind::ToolFinished {
                    tool_use_id: "t1".to_owned(),
                    status: ToolStatus::Error,
                },
            ),
        ];
        store
            .apply(&Batch {
                events: &events,
                now_ms: T0,
                ..Default::default()
            })
            .unwrap();

        let s = store.session_summary("s1", T0 + 5_000).unwrap().unwrap();
        assert_eq!(s.cwd.as_deref(), Some(r"C:\work\demo"));
        assert_eq!(s.model.as_deref(), Some("modelo-b"));
        assert_eq!((s.api_messages, s.usage.output), (2, 45));
        assert_eq!((s.tool_calls, s.tool_errors), (3, 1));
        assert_eq!(s.last_seen_at, T0 + 10_500);
        // «t-vieja» sigue en `running`, pero empezó antes del umbral.
        let running: Vec<_> = s
            .running_tools
            .iter()
            .map(|t| t.tool_use_id.as_str())
            .collect();
        assert_eq!(running, ["t2"]);
        assert_eq!(s.running_tools[0].agent_id.as_deref(), Some("agente-1"));
        let recent: Vec<_> = s
            .recent_tools
            .iter()
            .map(|t| t.tool_use_id.as_str())
            .collect();
        assert_eq!(recent, ["t2", "t1", "t-vieja"]);
        assert_eq!(s.recent_tools[1].duration_ms, Some(500));

        assert_eq!(store.session_summary("no-existe", 0).unwrap(), None);
        assert_eq!(store.sessions_active_since(T0 + 10_500).unwrap(), ["s1"]);
        assert!(store.sessions_active_since(T0 + 10_501).unwrap().is_empty());
    }
}
