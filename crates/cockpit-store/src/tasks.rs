//! Task Monitor (SPEC §8): operaciones declaradas, estado de cada lista y subagentes.
//!
//! `task_ops` es el registro: una fila por operación, idempotente por
//! `(session_id, op_key)`. `tasks` es su reproducción ([`cockpit_core::tasks::replay`]),
//! que se reescribe entera para cada lista que recibe una operación nueva.

use std::collections::{BTreeMap, BTreeSet};

use cockpit_core::model::{CockpitEvent, EventKind};
use cockpit_core::tasks::{
    self, AgentState, Progress, TOOL_TASK_CREATE, TaskOp, TaskSource, TaskStatus,
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::Serialize;

use crate::{Result, Store};

/// Listas que hay que reconstruir al final del lote.
pub(crate) type Touched = BTreeSet<(String, TaskSource)>;

/// Minutos de la sparkline de actividad.
pub const ACTIVITY_MINUTES: usize = 30;

/// Guarda una operación. Si ya existía (otra fuente la entregó antes), solo
/// rellena los títulos que le faltaban y suma la fuente.
pub(crate) fn record_op(
    tx: &Transaction<'_>,
    e: &CockpitEvent,
    source: TaskSource,
    op_key: &str,
    op: &TaskOp,
    touched: &mut Touched,
) -> Result<()> {
    let existing: Option<String> = tx
        .prepare_cached("SELECT op_json FROM task_ops WHERE session_id = ?1 AND op_key = ?2")?
        .query_row(params![e.session_id, op_key], |r| r.get(0))
        .optional()?;
    match existing {
        None => {
            let json = serde_json::to_string(op).unwrap_or_default();
            tx.prepare_cached(
                "INSERT INTO task_ops(session_id, op_key, source, ts, agent_id, op_json, source_mask)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![
                e.session_id,
                op_key,
                source.as_str(),
                e.ts_ms,
                e.agent_id,
                json,
                e.source.mask()
            ])?;
            touched.insert((e.session_id.clone(), source));
        }
        Some(json) => {
            let mut stored: TaskOp = match serde_json::from_str(&json) {
                Ok(op) => op,
                Err(_) => op.clone(),
            };
            let before = stored.clone();
            stored.fill_subjects(op);
            tx.prepare_cached(
                "UPDATE task_ops SET op_json = ?1, source_mask = source_mask | ?2
                 WHERE session_id = ?3 AND op_key = ?4",
            )?
            .execute(params![
                serde_json::to_string(&stored).unwrap_or(json),
                e.source.mask(),
                e.session_id,
                op_key
            ])?;
            if stored != before {
                touched.insert((e.session_id.clone(), source));
            }
        }
    }
    Ok(())
}

/// Una llamada que terminó en error o se interrumpió no cambió la lista.
pub(crate) fn drop_failed_op(
    tx: &Transaction<'_>,
    session_id: &str,
    tool_use_id: &str,
    touched: &mut Touched,
) -> Result<()> {
    let source: Option<String> = tx
        .prepare_cached(
            "DELETE FROM task_ops WHERE session_id = ?1 AND op_key = 'tool:' || ?2 RETURNING source",
        )?
        .query_row(params![session_id, tool_use_id], |r| r.get(0))
        .optional()?;
    if let Some(source) = source.as_deref().and_then(TaskSource::parse) {
        touched.insert((session_id.to_owned(), source));
    }
    Ok(())
}

/// Resultado de una herramienta del transcript: el store sabe de qué herramienta es.
pub(crate) fn apply_result_meta(
    tx: &Transaction<'_>,
    e: &CockpitEvent,
    touched: &mut Touched,
) -> Result<()> {
    let EventKind::ToolResultMeta {
        tool_use_id,
        is_error,
        task_id,
        task_subject,
        agent,
        agent_type,
        agent_status,
        duration_ms,
    } = &e.kind
    else {
        return Ok(());
    };
    let tool: Option<String> = tx
        .prepare_cached("SELECT tool FROM tool_calls WHERE session_id = ?1 AND tool_use_id = ?2")?
        .query_row(params![e.session_id, tool_use_id], |r| r.get(0))
        .optional()?;
    match tool.as_deref() {
        Some(TOOL_TASK_CREATE) if !is_error => {
            if let Some(task_id) = task_id {
                let op = TaskOp::Create {
                    task_id: task_id.clone(),
                    subject: task_subject.clone(),
                };
                if let Some(key) = op.key(None) {
                    record_op(tx, e, TaskSource::TaskTools, &key, &op, touched)?;
                }
            }
        }
        Some("Agent") => {
            if let Some(agent) = agent {
                upsert_agent(
                    tx,
                    &AgentUpdate {
                        session_id: &e.session_id,
                        agent,
                        agent_type: agent_type.as_deref(),
                        status: *agent_status,
                        parent_id: e.agent_id.as_deref(),
                        ts: e.ts_ms,
                        duration_ms: *duration_ms,
                        mask: e.source.mask(),
                    },
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) struct AgentUpdate<'a> {
    pub session_id: &'a str,
    pub agent: &'a str,
    pub agent_type: Option<&'a str>,
    pub status: Option<AgentState>,
    pub parent_id: Option<&'a str>,
    pub ts: i64,
    pub duration_ms: Option<u64>,
    pub mask: i64,
}

pub(crate) fn upsert_agent(tx: &Transaction<'_>, u: &AgentUpdate<'_>) -> Result<()> {
    // Un agente no es su propio padre (el motor no lo permite; un dato raro, tampoco).
    let parent = u.parent_id.filter(|p| *p != u.agent);
    tx.prepare_cached(
        "INSERT INTO agents(session_id, agent_id, agent_type, status, status_ts, parent_id,
                            first_seen, last_seen, duration_ms, source_mask)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?5, ?5, ?7, ?8)
         ON CONFLICT(session_id, agent_id) DO UPDATE SET
             agent_type = COALESCE(excluded.agent_type, agent_type),
             status = CASE WHEN excluded.status IS NOT NULL AND excluded.status_ts >= status_ts
                           THEN excluded.status ELSE status END,
             status_ts = CASE WHEN excluded.status IS NOT NULL AND excluded.status_ts >= status_ts
                              THEN excluded.status_ts ELSE status_ts END,
             parent_id = COALESCE(excluded.parent_id, parent_id),
             first_seen = MIN(first_seen, excluded.first_seen),
             last_seen = MAX(last_seen, excluded.last_seen),
             duration_ms = COALESCE(excluded.duration_ms, duration_ms),
             source_mask = source_mask | excluded.source_mask",
    )?
    .execute(params![
        u.session_id,
        u.agent,
        u.agent_type,
        u.status.map(AgentState::as_str),
        u.ts,
        parent,
        u.duration_ms.map(|d| d as i64),
        u.mask
    ])?;
    Ok(())
}

/// Cada evento de un subagente prueba que existe: primera y última actividad.
pub(crate) fn touch_agents(tx: &Transaction<'_>, events: &[CockpitEvent]) -> Result<()> {
    let mut seen: BTreeMap<(&str, &str), (i64, i64, i64)> = BTreeMap::new();
    for e in events {
        // En `AgentStatus` y `ToolResultMeta`, `agent_id` es el padre, no el agente.
        if matches!(
            e.kind,
            EventKind::AgentStatus { .. }
                | EventKind::ToolResultMeta { .. }
                | EventKind::CostState { .. }
                | EventKind::TranscriptFile { .. }
        ) {
            continue;
        }
        let Some(agent) = e.agent_id.as_deref() else {
            continue;
        };
        let entry = seen
            .entry((e.session_id.as_str(), agent))
            .or_insert((e.ts_ms, e.ts_ms, 0));
        entry.0 = entry.0.min(e.ts_ms);
        entry.1 = entry.1.max(e.ts_ms);
        entry.2 |= e.source.mask();
    }
    let mut stmt = tx.prepare_cached(
        "INSERT INTO agents(session_id, agent_id, status_ts, first_seen, last_seen, source_mask)
         SELECT ?1, ?2, 0, ?3, ?4, ?5 WHERE EXISTS(SELECT 1 FROM sessions WHERE id = ?1)
         ON CONFLICT(session_id, agent_id) DO UPDATE SET
             first_seen = MIN(first_seen, excluded.first_seen),
             last_seen = MAX(last_seen, excluded.last_seen),
             source_mask = source_mask | excluded.source_mask",
    )?;
    for ((session, agent), (first, last, mask)) in seen {
        stmt.execute(params![session, agent, first, last, mask])?;
    }
    Ok(())
}

/// Reproduce las operaciones de una lista y reescribe su estado.
pub(crate) fn rebuild(tx: &Transaction<'_>, session_id: &str, source: TaskSource) -> Result<()> {
    let ops: Vec<(i64, TaskOp)> = tx
        .prepare_cached(
            "SELECT ts, op_json FROM task_ops WHERE session_id = ?1 AND source = ?2
             ORDER BY ts, rowid",
        )?
        .query_map(params![session_id, source.as_str()], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .filter_map(|row| {
            let (ts, json) = row.ok()?;
            Some((ts, serde_json::from_str(&json).ok()?))
        })
        .collect();
    let state = tasks::replay(source, &ops);
    tx.prepare_cached("DELETE FROM tasks WHERE session_id = ?1 AND source = ?2")?
        .execute(params![session_id, source.as_str()])?;
    let mut insert = tx.prepare_cached(
        "INSERT INTO tasks(session_id, source, task_id, subject, status, owner, blocked_by_json,
                           position, created_at, updated_at, completed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;
    for t in state {
        insert.execute(params![
            session_id,
            source.as_str(),
            t.task_id,
            t.subject,
            t.status.as_str(),
            t.owner,
            serde_json::to_string(&t.blocked_by).unwrap_or_else(|_| "[]".to_owned()),
            t.position,
            t.created_at,
            t.updated_at,
            t.completed_at
        ])?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Vistas
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskItemView {
    pub task_id: String,
    /// `None` sin `capture_task_titles` (D1): la interfaz muestra «Tarea n.º k».
    pub subject: Option<String>,
    pub status: TaskStatus,
    pub owner: Option<String>,
    pub blocked_by: Vec<String>,
    pub completed_at: Option<i64>,
}

/// Una lista declarada por la sesión, con la fuente indicada (SPEC §8).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskListView {
    pub source: TaskSource,
    /// Sin las tareas borradas.
    pub items: Vec<TaskItemView>,
    pub progress: Progress,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentView {
    pub agent_id: String,
    pub agent_type: Option<String>,
    pub status: Option<AgentState>,
    pub parent_id: Option<String>,
    pub first_seen: i64,
    pub last_seen: i64,
    pub duration_ms: Option<i64>,
}

/// Herramientas por minuto: **actividad**, nunca progreso (SPEC §8).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ActivityView {
    /// Inicio (ms) del primer minuto de `per_minute`.
    pub since: i64,
    pub per_minute: Vec<u32>,
}

/// Tope de subagentes por sesión en la vista.
const MAX_AGENTS: i64 = 100;

impl Store {
    /// Desactiva adaptadores (A3 es desactivable, SPEC §8). Sus operaciones se
    /// siguen registrando, pero sus listas no se muestran.
    pub fn set_disabled_task_sources(&mut self, sources: &[TaskSource]) {
        self.disabled_sources = sources.to_vec();
    }

    pub(crate) fn source_enabled(&self, source: TaskSource) -> bool {
        !self.disabled_sources.contains(&source)
    }

    /// Listas de tareas de una sesión, de la más reciente a la más antigua.
    pub fn session_tasks(&self, session_id: &str) -> Result<Vec<TaskListView>> {
        let mut stmt = self.connection().prepare_cached(
            "SELECT source, task_id, subject, status, owner, blocked_by_json, completed_at, updated_at
             FROM tasks WHERE session_id = ?1 ORDER BY source, position",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<i64>>(6)?,
                r.get::<_, i64>(7)?,
            ))
        })?;
        let mut lists: BTreeMap<TaskSource, TaskListView> = BTreeMap::new();
        for row in rows {
            let (source, task_id, subject, status, owner, blocked, completed_at, updated_at) = row?;
            let (Some(source), Some(status)) =
                (TaskSource::parse(&source), TaskStatus::parse(&status))
            else {
                continue;
            };
            if !self.source_enabled(source) {
                continue;
            }
            let list = lists.entry(source).or_insert_with(|| TaskListView {
                source,
                items: Vec::new(),
                progress: Progress::default(),
                updated_at,
            });
            list.updated_at = list.updated_at.max(updated_at);
            if status == TaskStatus::Deleted {
                continue;
            }
            list.items.push(TaskItemView {
                task_id,
                subject,
                status,
                owner,
                blocked_by: serde_json::from_str(&blocked).unwrap_or_default(),
                completed_at,
            });
        }
        let mut out: Vec<TaskListView> = lists
            .into_values()
            .filter(|l| !l.items.is_empty())
            .map(|mut l| {
                for item in &l.items {
                    l.progress.total += 1;
                    match item.status {
                        TaskStatus::Completed => l.progress.completed += 1,
                        TaskStatus::InProgress => l.progress.in_progress += 1,
                        _ => {}
                    }
                }
                l
            })
            .collect();
        out.sort_by_key(|l| std::cmp::Reverse(l.updated_at));
        Ok(out)
    }

    /// Subagentes de una sesión, por orden de aparición.
    pub fn session_agents(&self, session_id: &str) -> Result<Vec<AgentView>> {
        let mut stmt = self.connection().prepare_cached(
            "SELECT agent_id, agent_type, status, parent_id, first_seen, last_seen, duration_ms
             FROM agents WHERE session_id = ?1 ORDER BY first_seen, agent_id LIMIT ?2",
        )?;
        let agents = stmt
            .query_map(params![session_id, MAX_AGENTS], |r| {
                Ok(AgentView {
                    agent_id: r.get(0)?,
                    agent_type: r.get(1)?,
                    status: r
                        .get::<_, Option<String>>(2)?
                        .as_deref()
                        .and_then(AgentState::parse),
                    parent_id: r.get(3)?,
                    first_seen: r.get(4)?,
                    last_seen: r.get(5)?,
                    duration_ms: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(agents)
    }

    /// Herramientas por minuto en los últimos [`ACTIVITY_MINUTES`] minutos hasta `now_ms`.
    pub fn session_activity(&self, session_id: &str, now_ms: i64) -> Result<ActivityView> {
        let minute = 60_000;
        let current = now_ms.div_euclid(minute);
        let first = current - ACTIVITY_MINUTES as i64 + 1;
        let mut per_minute = vec![0u32; ACTIVITY_MINUTES];
        let mut stmt = self.connection().prepare_cached(
            "SELECT started_at / 60000 AS m, COUNT(*) FROM tool_calls
             WHERE session_id = ?1 AND is_copy = 0 AND started_at >= ?2 AND started_at < ?3
             GROUP BY m",
        )?;
        let rows = stmt.query_map(
            params![session_id, first * minute, (current + 1) * minute],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )?;
        for row in rows {
            let (m, n) = row?;
            if let Some(slot) = usize::try_from(m - first)
                .ok()
                .and_then(|i| per_minute.get_mut(i))
            {
                *slot = n as u32;
            }
        }
        Ok(ActivityView {
            since: first * minute,
            per_minute,
        })
    }

    /// `capture_task_titles = false` borra los títulos ya guardados (SPEC §10).
    pub fn clear_task_subjects(&mut self) -> Result<u64> {
        let tx = self.conn.transaction()?;
        let cleared = tx.execute(
            "UPDATE tasks SET subject = NULL WHERE subject IS NOT NULL",
            [],
        )?;
        let ops: Vec<(String, String, String)> = tx
            .prepare("SELECT session_id, op_key, op_json FROM task_ops")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (session_id, op_key, json) in ops {
            let Ok(mut op) = serde_json::from_str::<TaskOp>(&json) else {
                continue;
            };
            op.clear_subjects();
            let clean = serde_json::to_string(&op).unwrap_or_default();
            if clean != json {
                tx.execute(
                    "UPDATE task_ops SET op_json = ?1 WHERE session_id = ?2 AND op_key = ?3",
                    params![clean, session_id, op_key],
                )?;
            }
        }
        tx.commit()?;
        Ok(cleared as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Batch;
    use cockpit_core::model::{Source, ToolStatus};
    use cockpit_core::privacy::{PrivacyPolicy, ToolMeta};
    use cockpit_core::tasks::{ListItem, create_op};

    const T0: i64 = 1_762_790_000_000;

    fn ev(source: Source, agent: Option<&str>, ts: i64, kind: EventKind) -> CockpitEvent {
        CockpitEvent {
            source,
            session_id: "s1".to_owned(),
            agent_id: agent.map(str::to_owned),
            ts_ms: ts,
            engine_version: None,
            kind,
        }
    }

    fn apply(store: &mut Store, events: &[CockpitEvent]) {
        store
            .apply(&Batch {
                events,
                now_ms: T0,
                ..Default::default()
            })
            .unwrap();
    }

    fn started(source: Source, ts: i64, id: &str, tool: &str) -> CockpitEvent {
        ev(
            source,
            None,
            ts,
            EventKind::ToolStarted {
                tool_use_id: id.to_owned(),
                tool: tool.to_owned(),
                meta: ToolMeta::default(),
            },
        )
    }

    fn finished(source: Source, ts: i64, id: &str, status: ToolStatus) -> CockpitEvent {
        ev(
            source,
            None,
            ts,
            EventKind::ToolFinished {
                tool_use_id: id.to_owned(),
                status,
            },
        )
    }

    fn plan_op(source: Source, ts: i64, id: &str, op: TaskOp) -> CockpitEvent {
        ev(
            source,
            None,
            ts,
            EventKind::TaskOp {
                source: TaskSource::DesktopPlan,
                op_key: op.key(Some(id)).unwrap(),
                op,
            },
        )
    }

    fn items(subjects: &[Option<&str>], statuses: &[TaskStatus]) -> TaskOp {
        TaskOp::Replace {
            items: subjects
                .iter()
                .zip(statuses)
                .map(|(s, st)| ListItem {
                    subject: s.map(str::to_owned),
                    status: *st,
                })
                .collect(),
        }
    }

    fn statuses(store: &Store) -> Vec<TaskStatus> {
        store.session_tasks("s1").unwrap()[0]
            .items
            .iter()
            .map(|i| i.status)
            .collect()
    }

    use TaskStatus::{Completed, InProgress, Pending};

    #[test]
    fn the_same_advance_from_mod_and_transcript_counts_once() {
        let mut store = Store::open_in_memory().unwrap();
        let plan = items(
            &[Some("Uno"), Some("Dos"), Some("Tres")],
            &[InProgress, Pending, Pending],
        );
        apply(
            &mut store,
            &[
                started(Source::Transcript, T0, "t1", "mcp__ccd_turn__update_plan"),
                plan_op(Source::Transcript, T0, "t1", plan),
            ],
        );
        // El mod entrega el `advance` al terminar; el transcript, después, el mismo.
        apply(
            &mut store,
            &[plan_op(Source::Mod, T0 + 2_000, "t2", TaskOp::Advance)],
        );
        apply(
            &mut store,
            &[plan_op(
                Source::Transcript,
                T0 + 1_500,
                "t2",
                TaskOp::Advance,
            )],
        );
        assert_eq!(statuses(&store), [Completed, InProgress, Pending]);
        let lists = store.session_tasks("s1").unwrap();
        assert_eq!(lists[0].source, TaskSource::DesktopPlan);
        assert_eq!(
            lists[0].progress,
            Progress {
                total: 3,
                completed: 1,
                in_progress: 1
            }
        );
    }

    #[test]
    fn titles_missing_before_the_handshake_are_filled_by_the_transcript() {
        let mut store = Store::open_in_memory().unwrap();
        let blind = items(&[None, None], &[InProgress, Pending]);
        let seen = items(&[Some("Uno"), Some("Dos")], &[InProgress, Pending]);
        apply(&mut store, &[plan_op(Source::Mod, T0, "t1", blind)]);
        assert_eq!(store.session_tasks("s1").unwrap()[0].items[0].subject, None);
        apply(
            &mut store,
            &[plan_op(Source::Transcript, T0 - 10, "t1", seen)],
        );
        let lists = store.session_tasks("s1").unwrap();
        assert_eq!(lists[0].items[0].subject.as_deref(), Some("Uno"));

        // Desactivar los títulos los borra también del registro.
        store.clear_task_subjects().unwrap();
        assert_eq!(store.session_tasks("s1").unwrap()[0].items[0].subject, None);
        let json: String = store
            .connection()
            .query_row("SELECT op_json FROM task_ops", [], |r| r.get(0))
            .unwrap();
        assert!(!json.contains("Uno"), "{json}");
    }

    #[test]
    fn a_failed_call_does_not_change_the_list() {
        let mut store = Store::open_in_memory().unwrap();
        let plan = items(&[Some("Uno"), Some("Dos")], &[InProgress, Pending]);
        apply(
            &mut store,
            &[
                started(Source::Transcript, T0, "t1", "mcp__ccd_turn__update_plan"),
                plan_op(Source::Transcript, T0, "t1", plan),
                started(
                    Source::Transcript,
                    T0 + 1,
                    "t2",
                    "mcp__ccd_turn__update_plan",
                ),
                plan_op(Source::Transcript, T0 + 1, "t2", TaskOp::Advance),
            ],
        );
        assert_eq!(statuses(&store), [Completed, InProgress]);
        apply(
            &mut store,
            &[finished(
                Source::Transcript,
                T0 + 2,
                "t2",
                ToolStatus::Error,
            )],
        );
        assert_eq!(statuses(&store), [InProgress, Pending]);
    }

    #[test]
    fn task_create_result_is_resolved_by_the_stored_tool() {
        let mut store = Store::open_in_memory().unwrap();
        let result = |id: &str, task: &str, subject: &str| {
            ev(
                Source::Transcript,
                None,
                T0 + 5,
                EventKind::ToolResultMeta {
                    tool_use_id: id.to_owned(),
                    is_error: false,
                    task_id: Some(task.to_owned()),
                    task_subject: Some(subject.to_owned()),
                    agent: None,
                    agent_type: None,
                    agent_status: None,
                    duration_ms: None,
                },
            )
        };
        apply(
            &mut store,
            &[
                started(Source::Transcript, T0, "t1", "TaskCreate"),
                result("t1", "1", "Primera"),
                // `TaskGet` devuelve una forma parecida: no crea nada.
                started(Source::Transcript, T0, "t2", "TaskGet"),
                result("t2", "9", "No es una creación"),
            ],
        );
        // El hook `TaskCreated` del mod repite la creación: misma clave.
        let hook = create_op("1", Some("Primera"), &PrivacyPolicy::APP_DEFAULT).unwrap();
        apply(
            &mut store,
            &[ev(
                Source::Mod,
                None,
                T0 + 6,
                EventKind::TaskOp {
                    source: TaskSource::TaskTools,
                    op_key: hook.key(None).unwrap(),
                    op: hook,
                },
            )],
        );
        let lists = store.session_tasks("s1").unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].items.len(), 1);
        assert_eq!(lists[0].items[0].subject.as_deref(), Some("Primera"));
        let ops: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM task_ops", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ops, 1);
    }

    #[test]
    fn subagents_build_a_tree_with_the_latest_status() {
        let mut store = Store::open_in_memory().unwrap();
        let agent_line = |agent: &str, ts: i64| {
            let mut e = started(Source::Transcript, ts, &format!("x{ts}"), "Read");
            e.agent_id = Some(agent.to_owned());
            e
        };
        let status = |agent: &str, parent: Option<&str>, ts: i64, st: AgentState| {
            ev(
                Source::Mod,
                parent,
                ts,
                EventKind::AgentStatus {
                    agent: agent.to_owned(),
                    agent_type: Some("Explore".to_owned()),
                    status: Some(st),
                    duration_ms: None,
                },
            )
        };
        apply(
            &mut store,
            &[
                started(Source::Transcript, T0, "t0", "Agent"),
                agent_line("a1", T0 + 10),
                agent_line("a2", T0 + 20),
                status("a1", None, T0 + 30, AgentState::Running),
                status("a2", Some("a1"), T0 + 31, AgentState::Running),
                agent_line("a1", T0 + 40),
            ],
        );
        // Un estado más antiguo que llega tarde no pisa al último.
        apply(
            &mut store,
            &[
                status("a1", None, T0 + 50, AgentState::Completed),
                status("a1", None, T0 + 35, AgentState::Running),
            ],
        );
        let agents = store.session_agents("s1").unwrap();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].agent_id, "a1");
        assert_eq!(agents[0].status, Some(AgentState::Completed));
        assert_eq!(
            (agents[0].first_seen, agents[0].last_seen),
            (T0 + 10, T0 + 50)
        );
        assert_eq!(agents[1].parent_id.as_deref(), Some("a1"));
        assert_eq!(agents[1].agent_type.as_deref(), Some("Explore"));
    }

    #[test]
    fn activity_counts_tools_per_minute_and_never_copies() {
        let mut store = Store::open_in_memory().unwrap();
        let now = T0 + 10 * 60_000;
        apply(
            &mut store,
            &[
                started(Source::Transcript, now - 5_000, "a", "Read"),
                started(Source::Transcript, now - 6_000, "b", "Edit"),
                started(Source::Transcript, now - 3 * 60_000, "c", "Bash"),
                started(Source::Transcript, now - 40 * 60_000, "viejo", "Bash"),
            ],
        );
        let activity = store.session_activity("s1", now).unwrap();
        assert_eq!(activity.per_minute.len(), ACTIVITY_MINUTES);
        assert_eq!(activity.per_minute.iter().sum::<u32>(), 3);
        assert_eq!(*activity.per_minute.last().unwrap(), 2);
    }

    #[test]
    fn a_disabled_adapter_is_hidden_but_still_recorded() {
        let mut store = Store::open_in_memory().unwrap();
        store.set_disabled_task_sources(&[TaskSource::DesktopPlan]);
        apply(
            &mut store,
            &[plan_op(
                Source::Transcript,
                T0,
                "t1",
                items(&[Some("Uno")], &[InProgress]),
            )],
        );
        assert!(store.session_tasks("s1").unwrap().is_empty());
        store.set_disabled_task_sources(&[]);
        assert_eq!(store.session_tasks("s1").unwrap().len(), 1);
    }
}
