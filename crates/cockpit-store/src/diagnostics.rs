//! Diagnóstico (SPEC §7, vista 5, y avisos de §6.4.2): salud de la ingesta y de la
//! base. **Solo lectura.** Sin contenido ni rutas: las incidencias se agrupan por
//! tipo y se cuenta en cuántos ficheros aparecen.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::params;
use serde::Serialize;

use crate::compact::Target;
use crate::{Result, Store};

/// Incidencias de la ingesta de un mismo tipo. `detail` es un nombre de tipo de
/// línea o de herramienta, nunca contenido.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorGroup {
    pub kind: String,
    pub detail: String,
    pub count: u64,
    /// En cuántos ficheros aparece (las rutas no salen de la base).
    pub files: u64,
    pub last_ts: i64,
}

/// Motores y versiones del mod que han hecho `hello`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineSeen {
    pub engine_version: Option<String>,
    pub mod_version: Option<String>,
    pub sessions: u64,
    pub last_hello_at: i64,
}

/// Estado de la agregación diaria de un target (avisos de §6.4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AggregationHealth {
    pub target: Target,
    /// Días cerrados pendientes de agregar (nunca agregados o marcados `dirty`).
    pub pending_days: u64,
    pub dirty_days: u64,
    /// Días `done` cuyo agregado no cuadra con la tabla cruda (reconciliación).
    pub mismatched_days: u64,
}

/// Detalle de rutas que queda en la base (retención de 30 días, §6.4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathDetail {
    pub tool_calls_with_path: u64,
    pub file_events: u64,
    /// Día más antiguo con alguna ruta guardada.
    pub oldest_day: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostics {
    pub schema_version: u32,
    pub sessions: u64,
    pub api_messages: u64,
    pub tool_calls: u64,
    pub tool_call_copies: u64,
    pub api_errors: u64,
    pub transcript_files: u64,
    pub ingest_errors: Vec<ErrorGroup>,
    pub engines: Vec<EngineSeen>,
    pub aggregation: Vec<AggregationHealth>,
    pub path_detail: PathDetail,
}

/// Tope de grupos de incidencias que se devuelven.
const MAX_ERROR_GROUPS: u32 = 50;

impl Store {
    /// Diagnóstico de la base. `closed_before` y `purge_before` son los cortes de la
    /// compactación (`time::closed_before` y el de §6.4.1).
    pub fn diagnostics(&self, closed_before: &str, purge_before: &str) -> Result<Diagnostics> {
        let totals = self.totals()?;
        let count = |sql: &str| -> rusqlite::Result<u64> {
            self.conn
                .query_row(sql, [], |r| r.get::<_, i64>(0))
                .map(|n| n.max(0) as u64)
        };
        let schema_version = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;

        let mut stmt = self.conn.prepare(
            "SELECT kind, detail, SUM(count), COUNT(DISTINCT path), MAX(ts) FROM ingest_errors
             GROUP BY kind, detail ORDER BY SUM(count) DESC, kind, detail LIMIT ?1",
        )?;
        let ingest_errors = stmt
            .query_map([MAX_ERROR_GROUPS], |r| {
                Ok(ErrorGroup {
                    kind: r.get(0)?,
                    detail: r.get(1)?,
                    count: r.get::<_, i64>(2)?.max(0) as u64,
                    files: r.get::<_, i64>(3)?.max(0) as u64,
                    last_ts: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        let mut stmt = self.conn.prepare(
            "SELECT engine_version, mod_version, COUNT(DISTINCT session_id), MAX(last_hello_at)
             FROM mod_instances GROUP BY engine_version, mod_version
             ORDER BY MAX(last_hello_at) DESC",
        )?;
        let engines = stmt
            .query_map([], |r| {
                Ok(EngineSeen {
                    engine_version: r.get(0)?,
                    mod_version: r.get(1)?,
                    sessions: r.get::<_, i64>(2)?.max(0) as u64,
                    last_hello_at: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        let mismatches = self.aggregate_mismatches(purge_before)?;
        let mut aggregation = Vec::new();
        for target in Target::ALL {
            let dirty_days = self.conn.query_row(
                "SELECT COUNT(*) FROM aggregation_ledger WHERE target = ?1 AND state = 'dirty'",
                [target.name()],
                |r| r.get::<_, i64>(0),
            )?;
            aggregation.push(AggregationHealth {
                target,
                pending_days: self.aggregation_candidates(target, closed_before)?.len() as u64,
                dirty_days: dirty_days.max(0) as u64,
                mismatched_days: mismatches.iter().filter(|(t, ..)| *t == target).count() as u64,
            });
        }

        let oldest_day: Option<String> = self.conn.query_row(
            "SELECT MIN(day) FROM (
                 SELECT MIN(day) AS day FROM tool_calls WHERE rel_path IS NOT NULL OR host IS NOT NULL
                 UNION ALL SELECT MIN(day) FROM file_events)",
            [],
            |r| r.get(0),
        )?;

        Ok(Diagnostics {
            schema_version,
            sessions: totals.sessions,
            api_messages: totals.api_messages,
            tool_calls: totals.tool_calls,
            tool_call_copies: totals.tool_call_copies,
            api_errors: totals.api_errors,
            transcript_files: count("SELECT COUNT(*) FROM file_offsets")?,
            ingest_errors,
            engines,
            aggregation,
            path_detail: PathDetail {
                tool_calls_with_path: count(
                    "SELECT COUNT(*) FROM tool_calls WHERE rel_path IS NOT NULL OR host IS NOT NULL",
                )?,
                file_events: count("SELECT COUNT(*) FROM file_events")?,
                oldest_day,
            },
        })
    }

    /// Días `done` (de `purge_before` en adelante) cuyo agregado no cuadra con su
    /// tabla cruda, con la misma normalización que la agregación (§6.4.2).
    pub(crate) fn aggregate_mismatches(
        &self,
        purge_before: &str,
    ) -> Result<Vec<(Target, i64, String)>> {
        let pairs = [
            (
                Target::ToolUsageDaily,
                "SELECT COALESCE(root_id, 0), day, COUNT(*) FROM tool_calls
                 WHERE is_copy = 0 AND day >= ?1 GROUP BY 1, 2",
                "SELECT root_id, day, SUM(calls) FROM tool_usage_daily WHERE day >= ?1 GROUP BY 1, 2",
            ),
            (
                Target::FileChangeDaily,
                "SELECT COALESCE(root_id, 0), day, COUNT(*) FROM file_events
                 WHERE day >= ?1 GROUP BY 1, 2",
                "SELECT root_id, day, SUM(events) FROM file_change_daily WHERE day >= ?1 GROUP BY 1, 2",
            ),
        ];
        let mut out = Vec::new();
        for (target, raw_sql, agg_sql) in pairs {
            let read = |sql: &str| -> Result<BTreeMap<(i64, String), i64>> {
                let mut stmt = self.conn.prepare(sql)?;
                let rows =
                    stmt.query_map([purge_before], |r| Ok(((r.get(0)?, r.get(1)?), r.get(2)?)))?;
                Ok(rows.collect::<rusqlite::Result<_>>()?)
            };
            let raw = read(raw_sql)?;
            let agg = read(agg_sql)?;
            let mut done = self.conn.prepare(
                "SELECT 1 FROM aggregation_ledger
                 WHERE target = ?1 AND root_id = ?2 AND day = ?3 AND state = 'done'",
            )?;
            let keys: BTreeSet<&(i64, String)> = raw.keys().chain(agg.keys()).collect();
            for key in keys {
                if raw.get(key).copied().unwrap_or(0) != agg.get(key).copied().unwrap_or(0)
                    && done.exists(params![target.name(), key.0, key.1])?
                {
                    out.push((target, key.0, key.1.clone()));
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Batch, CompactOptions, IngestError};
    use cockpit_core::model::{CockpitEvent, EventKind, Source, ToolStatus};
    use cockpit_core::privacy::ToolMeta;
    use cockpit_core::time::local_day;

    /// Mediodía UTC: el día local es el mismo en cualquier zona de Europa.
    const T0: i64 = 1_762_776_000_000;
    const DAY: i64 = 86_400_000;

    fn tool(store: &mut Store, id: &str, ts: i64) {
        let ev = |ts, kind| CockpitEvent {
            source: Source::Transcript,
            session_id: "s1".to_owned(),
            agent_id: None,
            ts_ms: ts,
            engine_version: None,
            kind,
        };
        let events = [
            ev(
                ts,
                EventKind::ToolStarted {
                    tool_use_id: id.to_owned(),
                    tool: "Edit".to_owned(),
                    meta: ToolMeta {
                        rel_path: Some("src/a.rs".to_owned()),
                        ..Default::default()
                    },
                },
            ),
            ev(
                ts + 10,
                EventKind::ToolFinished {
                    tool_use_id: id.to_owned(),
                    status: ToolStatus::Ok,
                },
            ),
        ];
        store
            .apply(&Batch {
                events: &events,
                now_ms: ts,
                ..Default::default()
            })
            .unwrap();
    }

    #[test]
    fn an_empty_base_is_healthy() {
        let store = Store::open_in_memory().unwrap();
        let d = store.diagnostics("2026-11-10", "2026-10-11").unwrap();
        assert_eq!(d.schema_version, crate::SCHEMA_VERSION);
        assert_eq!((d.sessions, d.tool_calls, d.transcript_files), (0, 0, 0));
        assert!(d.ingest_errors.is_empty() && d.engines.is_empty());
        assert!(
            d.aggregation
                .iter()
                .all(|a| a.pending_days == 0 && a.mismatched_days == 0)
        );
        assert_eq!(d.path_detail.oldest_day, None);
    }

    #[test]
    fn errors_are_grouped_by_type_without_paths() {
        let mut store = Store::open_in_memory().unwrap();
        let errors = [
            IngestError {
                path: "C:\\Users\\alguien\\a.jsonl".to_owned(),
                kind: "unknown_type",
                detail: "tipo-nuevo".to_owned(),
                count: 3,
            },
            IngestError {
                path: "C:\\Users\\alguien\\b.jsonl".to_owned(),
                kind: "unknown_type",
                detail: "tipo-nuevo".to_owned(),
                count: 2,
            },
        ];
        store
            .apply(&Batch {
                errors: &errors,
                now_ms: T0,
                ..Default::default()
            })
            .unwrap();
        let d = store.diagnostics("2026-11-10", "2026-10-11").unwrap();
        assert_eq!(
            d.ingest_errors,
            [ErrorGroup {
                kind: "unknown_type".to_owned(),
                detail: "tipo-nuevo".to_owned(),
                count: 5,
                files: 2,
                last_ts: T0,
            }]
        );
        assert!(!serde_json::to_string(&d).unwrap().contains("alguien"));
    }

    #[test]
    fn pending_days_and_mismatches_are_reported_and_reconcile_marks_them() {
        let mut store = Store::open_in_memory().unwrap();
        tool(&mut store, "t1", T0);
        tool(&mut store, "t2", T0 + DAY);
        let closed = local_day(T0 + 3 * DAY);
        let purge = local_day(T0 - 30 * DAY);
        let d = store.diagnostics(&closed, &purge).unwrap();
        let tools = |d: &Diagnostics| {
            d.aggregation
                .iter()
                .find(|a| a.target == Target::ToolUsageDaily)
                .cloned()
                .unwrap()
        };
        assert_eq!(tools(&d).pending_days, 2);
        assert_eq!(d.path_detail.tool_calls_with_path, 2);
        assert_eq!(d.path_detail.oldest_day, Some(local_day(T0)));

        store
            .compact(&CompactOptions {
                closed_before: &closed,
                purge_before: &purge,
                errors_before_ms: 0,
                now_ms: T0 + 3 * DAY,
            })
            .unwrap();
        let d = store.diagnostics(&closed, &purge).unwrap();
        assert_eq!((tools(&d).pending_days, tools(&d).mismatched_days), (0, 0));

        // Un agregado que deja de cuadrar: el diagnóstico lo ve sin tocar nada.
        store
            .connection()
            .execute("UPDATE tool_usage_daily SET calls = calls + 1", [])
            .unwrap();
        assert_eq!(
            tools(&store.diagnostics(&closed, &purge).unwrap()).mismatched_days,
            2
        );
        assert_eq!(
            tools(&store.diagnostics(&closed, &purge).unwrap()).dirty_days,
            0
        );
        // La reconciliación los marca como `dirty`, y entonces pasan a pendientes.
        assert_eq!(store.reconcile(&purge).unwrap(), 2);
        let d = store.diagnostics(&closed, &purge).unwrap();
        assert_eq!(
            (
                tools(&d).mismatched_days,
                tools(&d).dirty_days,
                tools(&d).pending_days
            ),
            (0, 2, 2)
        );
    }
}
