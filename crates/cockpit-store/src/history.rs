//! Histórico y límites (SPEC §7, vistas 3 y 4): tokens por día, proyecto y modelo,
//! uso de herramientas (frecuencia, latencia p50/p95 y fallos) y evolución de los
//! límites de uso. **Solo lectura.** De cada proyecto solo sale el nombre de su
//! carpeta, nunca la ruta.

use std::collections::BTreeMap;

use cockpit_core::model::RateLimit;
use serde::Serialize;

use crate::{Result, Store};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Tokens {
    pub messages: u64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_creation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DayTokens {
    pub day: String,
    #[serde(flatten)]
    pub tokens: Tokens,
}

/// Tokens de un proyecto o de un modelo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroupTokens {
    /// Nombre de la carpeta del proyecto (`""` sin proyecto) o id del modelo.
    pub key: String,
    pub sessions: u64,
    #[serde(flatten)]
    pub tokens: Tokens,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolStats {
    pub tool: String,
    pub calls: u64,
    pub errors: u64,
    pub interrupted: u64,
    pub p50_ms: Option<u64>,
    pub p95_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LimitPoint {
    pub ts: i64,
    pub kind: String,
    pub percent_used: f64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct History {
    pub since_day: String,
    pub days: Vec<DayTokens>,
    pub projects: Vec<GroupTokens>,
    pub models: Vec<GroupTokens>,
    pub tools: Vec<ToolStats>,
    pub limits: Vec<LimitPoint>,
    /// Coste estimado de las sesiones del periodo (SPEC §3): el del mod o, si no
    /// hay, el de `cost-state`. Es una estimación del cliente, no la factura.
    pub cost_usd: f64,
    /// Sesiones del periodo con coste conocido y sin él.
    pub cost_sessions: u64,
    pub sessions_without_cost: u64,
}

const MAX_TOOLS: usize = 30;
const MAX_GROUPS: usize = 30;

/// `n`-ésimo percentil (0–100) de una lista ordenada, por el método del rango más cercano.
fn percentile(sorted: &[u64], p: u64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (p * sorted.len() as u64).div_ceil(100).max(1) as usize;
    sorted.get(rank - 1).copied()
}

fn folder_name(root: &str) -> String {
    root.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_owned()
}

impl Store {
    /// Histórico desde el día `since_day` (incluido) y el instante `since_ms`.
    pub fn history(&self, since_day: &str, since_ms: i64) -> Result<History> {
        let tokens = |r: &rusqlite::Row<'_>, at: usize| -> rusqlite::Result<Tokens> {
            let n = |i: usize| {
                r.get::<_, Option<i64>>(at + i)
                    .map(|v| v.unwrap_or(0).max(0) as u64)
            };
            Ok(Tokens {
                messages: n(0)?,
                input: n(1)?,
                output: n(2)?,
                cache_read: n(3)?,
                cache_creation: n(4)?,
            })
        };
        const SUMS: &str = "COUNT(*), SUM(m.input_tokens), SUM(m.output_tokens),
                            SUM(m.cache_read_tokens), SUM(m.cache_creation_tokens)";

        let mut stmt = self.conn.prepare(&format!(
            "SELECT m.day, {SUMS} FROM api_messages m WHERE m.day >= ?1 GROUP BY m.day ORDER BY m.day"
        ))?;
        let days = stmt
            .query_map([since_day], |r| {
                Ok(DayTokens {
                    day: r.get(0)?,
                    tokens: tokens(r, 1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        let mut stmt = self.conn.prepare(&format!(
            "SELECT p.root, COUNT(DISTINCT m.session_id), {SUMS}
             FROM api_messages m JOIN sessions s ON s.id = m.session_id
             LEFT JOIN projects p ON p.id = s.root_id
             WHERE m.day >= ?1 GROUP BY s.root_id ORDER BY SUM(m.output_tokens) DESC"
        ))?;
        let mut by_name: BTreeMap<String, GroupTokens> = BTreeMap::new();
        for row in stmt.query_map([since_day], |r| {
            let root: Option<String> = r.get(0)?;
            Ok((
                root.map(|r| folder_name(&r)).unwrap_or_default(),
                r.get::<_, i64>(1)?.max(0) as u64,
                tokens(r, 2)?,
            ))
        })? {
            // Dos raíces con el mismo nombre de carpeta se suman: solo sale el nombre.
            let (key, sessions, t) = row?;
            let entry = by_name.entry(key.clone()).or_insert(GroupTokens {
                key,
                sessions: 0,
                tokens: Tokens::default(),
            });
            entry.sessions += sessions;
            entry.tokens.messages += t.messages;
            entry.tokens.input += t.input;
            entry.tokens.output += t.output;
            entry.tokens.cache_read += t.cache_read;
            entry.tokens.cache_creation += t.cache_creation;
        }
        let mut projects: Vec<GroupTokens> = by_name.into_values().collect();
        projects.sort_by(|a, b| {
            b.tokens
                .output
                .cmp(&a.tokens.output)
                .then(a.key.cmp(&b.key))
        });
        projects.truncate(MAX_GROUPS);

        let mut stmt = self.conn.prepare(&format!(
            "SELECT m.model, COUNT(DISTINCT m.session_id), {SUMS} FROM api_messages m
             WHERE m.day >= ?1 GROUP BY m.model ORDER BY SUM(m.output_tokens) DESC LIMIT ?2"
        ))?;
        let models = stmt
            .query_map(rusqlite::params![since_day, MAX_GROUPS as i64], |r| {
                Ok(GroupTokens {
                    key: r.get(0)?,
                    sessions: r.get::<_, i64>(1)?.max(0) as u64,
                    tokens: tokens(r, 2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        // Herramientas: las filas se conservan siempre (sin rutas pasados 30 días),
        // así que los percentiles salen exactos de las duraciones.
        let mut stmt = self.conn.prepare(
            "SELECT tool, status, duration_ms FROM tool_calls WHERE is_copy = 0 AND started_at >= ?1",
        )?;
        let mut acc: BTreeMap<String, (ToolStats, Vec<u64>)> = BTreeMap::new();
        for row in stmt.query_map([since_ms], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
            ))
        })? {
            let (tool, status, ms) = row?;
            let (stats, durations) = acc.entry(tool.clone()).or_insert_with(|| {
                (
                    ToolStats {
                        tool,
                        calls: 0,
                        errors: 0,
                        interrupted: 0,
                        p50_ms: None,
                        p95_ms: None,
                    },
                    Vec::new(),
                )
            });
            stats.calls += 1;
            match status.as_str() {
                "error" => stats.errors += 1,
                "interrupted" => stats.interrupted += 1,
                _ => {}
            }
            if let Some(ms) = ms.filter(|ms| *ms >= 0) {
                durations.push(ms as u64);
            }
        }
        let mut tools: Vec<ToolStats> = acc
            .into_values()
            .map(|(mut stats, mut durations)| {
                durations.sort_unstable();
                stats.p50_ms = percentile(&durations, 50);
                stats.p95_ms = percentile(&durations, 95);
                stats
            })
            .collect();
        tools.sort_by(|a, b| b.calls.cmp(&a.calls).then(a.tool.cmp(&b.tool)));
        tools.truncate(MAX_TOOLS);

        let mut stmt = self.conn.prepare(
            "SELECT ts, rate_limits_json FROM usage_snapshots WHERE ts >= ?1 ORDER BY ts, id",
        )?;
        let mut limits: Vec<LimitPoint> = Vec::new();
        let mut last: BTreeMap<String, (f64, Option<String>)> = BTreeMap::new();
        for row in stmt.query_map([since_ms], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })? {
            let (ts, json) = row?;
            for limit in serde_json::from_str::<Vec<RateLimit>>(&json).unwrap_or_default() {
                // Solo los cambios: varias sesiones repiten la misma lectura.
                let now = (limit.percent_used, limit.resets_at.clone());
                if last.get(&limit.kind) == Some(&now) {
                    continue;
                }
                last.insert(limit.kind.clone(), now);
                limits.push(LimitPoint {
                    ts,
                    kind: limit.kind,
                    percent_used: limit.percent_used,
                    resets_at: limit.resets_at,
                });
            }
        }

        let mut stmt = self.conn.prepare(
            "SELECT COALESCE((SELECT MAX(u.cost_usd) FROM usage_snapshots u WHERE u.session_id = s.id),
                             s.cost_state_usd)
             FROM sessions s WHERE s.last_seen_at >= ?1",
        )?;
        let (mut cost_usd, mut cost_sessions, mut sessions_without_cost) = (0.0, 0, 0);
        for cost in stmt.query_map([since_ms], |r| r.get::<_, Option<f64>>(0))? {
            match cost? {
                Some(c) if c.is_finite() && c >= 0.0 => {
                    cost_usd += c;
                    cost_sessions += 1;
                }
                _ => sessions_without_cost += 1,
            }
        }

        Ok(History {
            since_day: since_day.to_owned(),
            days,
            projects,
            models,
            tools,
            limits,
            cost_usd,
            cost_sessions,
            sessions_without_cost,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Batch;
    use cockpit_core::model::{CockpitEvent, EventKind, Source, TokenUsage, ToolStatus};
    use cockpit_core::privacy::ToolMeta;
    use cockpit_core::time::local_day;

    const T0: i64 = 1_762_776_000_000;

    fn ev(session: &str, ts: i64, kind: EventKind) -> CockpitEvent {
        CockpitEvent {
            source: Source::Transcript,
            session_id: session.to_owned(),
            agent_id: None,
            ts_ms: ts,
            engine_version: None,
            kind,
        }
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        let d: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile(&d, 50), Some(50));
        assert_eq!(percentile(&d, 95), Some(95));
        assert_eq!(percentile(&[7], 95), Some(7));
        assert_eq!(percentile(&[], 50), None);
        assert_eq!(
            folder_name("C:\\Users\\x\\Proyectos\\Benchify\\"),
            "Benchify"
        );
    }

    #[test]
    fn history_counts_tokens_tools_and_limits() {
        let mut store = Store::open_in_memory().unwrap();
        let usage = |output| TokenUsage {
            input: 10,
            output,
            cache_read: 100,
            cache_creation: 5,
        };
        let mut events = Vec::new();
        for (i, (model, output)) in [("claude-a", 40), ("claude-a", 60), ("claude-b", 5)]
            .into_iter()
            .enumerate()
        {
            events.push(ev(
                "s1",
                T0 + i as i64,
                EventKind::ApiMessage {
                    message_id: format!("m{i}"),
                    model: model.to_owned(),
                    usage: usage(output),
                    stop_reason: None,
                },
            ));
        }
        for (i, (status, ms)) in [
            (ToolStatus::Ok, 100),
            (ToolStatus::Ok, 300),
            (ToolStatus::Error, 50),
        ]
        .into_iter()
        .enumerate()
        {
            let id = format!("t{i}");
            events.push(ev(
                "s1",
                T0 + 10,
                EventKind::ToolStarted {
                    tool_use_id: id.clone(),
                    tool: "Bash".to_owned(),
                    meta: ToolMeta::default(),
                },
            ));
            events.push(ev(
                "s1",
                T0 + 10 + ms,
                EventKind::ToolFinished {
                    tool_use_id: id,
                    status,
                },
            ));
        }
        store
            .apply(&Batch {
                events: &events,
                now_ms: T0,
                ..Default::default()
            })
            .unwrap();

        let h = store.history(&local_day(T0), T0 - 1).unwrap();
        assert_eq!(h.days.len(), 1);
        assert_eq!(h.days[0].tokens.messages, 3);
        assert_eq!(h.days[0].tokens.output, 105);
        assert_eq!(h.models[0].key, "claude-a");
        assert_eq!(h.models[0].tokens.output, 100);
        assert_eq!(h.projects.len(), 1);
        assert_eq!(h.projects[0].key, "", "sin proyecto resuelto");
        let bash = &h.tools[0];
        assert_eq!(
            (bash.calls, bash.errors, bash.p50_ms, bash.p95_ms),
            (3, 1, Some(100), Some(300))
        );
        assert_eq!(h.sessions_without_cost, 1);
        assert!(h.limits.is_empty());

        // Fuera del periodo no cuenta nada.
        let later = store.history("2099-01-01", i64::MAX).unwrap();
        assert!(later.days.is_empty() && later.tools.is_empty());
    }
}
