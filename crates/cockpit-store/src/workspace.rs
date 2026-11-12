//! Proyectos, cambios de fichero y candidatas de atribución (SPEC §6.4, §7).

use cockpit_core::attribution::{Actor, Attribution, Confidence, DELTA_MS, EPSILON_MS};
use cockpit_core::privacy::extension;
use cockpit_core::time::local_day;
use cockpit_core::workspace::FileChange;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::{Result, Store};

/// Una herramienta en `running` desde hace más de esto no es candidata (sesión rota).
const STALE_RUNNING_MS: i64 = 60 * 60 * 1000;

/// Raíz de proyecto resuelta con Git (o el `cwd`, si no hay Git).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProject<'a> {
    pub root: &'a str,
    pub git_dir: Option<&'a str>,
    pub is_worktree: bool,
    pub main_root: Option<&'a str>,
}

/// Fila de herramienta tal como la necesita la atribución.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInterval {
    pub session_id: String,
    pub tool_use_id: String,
    pub tool: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    /// Relativa al `cwd` de la sesión, como la guardan el mod y el parser.
    pub rel_path: Option<String>,
}

/// Un cambio de fichero ya guardado, para reevaluar su atribución.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredChange {
    pub id: i64,
    pub rel_path: Option<String>,
    pub from_rel_path: Option<String>,
    pub ts: i64,
    pub settled: bool,
    pub attribution: Attribution,
}

/// Un cambio de fichero tal como lo pinta la vista «Cambios en directo».
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileEventView {
    pub id: i64,
    pub root_id: i64,
    pub rel_path: Option<String>,
    pub from_rel_path: Option<String>,
    pub change: String,
    pub size: Option<i64>,
    pub ts: i64,
    pub actor: String,
    pub session_id: Option<String>,
    pub tool_use_id: Option<String>,
    /// Nombre de la herramienta que lo hizo, si se conoce.
    pub tool: Option<String>,
    pub confidence: String,
    pub settled: bool,
}

const VIEW_COLUMNS: &str = "fe.id, fe.root_id, fe.rel_path, fe.from_rel_path, fe.change, fe.size,
    fe.ts, fe.actor, fe.actor_session, fe.actor_tool_use_id, tc.tool, fe.confidence, fe.settled";
const VIEW_FROM: &str = "FROM file_events fe LEFT JOIN tool_calls tc
    ON tc.session_id = fe.actor_session AND tc.tool_use_id = fe.actor_tool_use_id";

fn read_view(r: &rusqlite::Row<'_>) -> rusqlite::Result<FileEventView> {
    Ok(FileEventView {
        id: r.get(0)?,
        root_id: r.get(1)?,
        rel_path: r.get(2)?,
        from_rel_path: r.get(3)?,
        change: r.get(4)?,
        size: r.get(5)?,
        ts: r.get(6)?,
        actor: r.get(7)?,
        session_id: r.get(8)?,
        tool_use_id: r.get(9)?,
        tool: r.get(10)?,
        confidence: r.get(11)?,
        settled: r.get(12)?,
    })
}

fn read_attribution(
    actor: &str,
    session: Option<String>,
    tool: Option<String>,
    confidence: &str,
) -> Attribution {
    use cockpit_core::attribution::Rule;
    let actor = Actor::parse(actor).unwrap_or(Actor::Unknown);
    // La regla no se guarda; se reconstruye la más probable para comparar.
    let rule = match actor {
        Actor::Claude if confidence == "high" => Rule::R1,
        Actor::Claude => Rule::R2,
        Actor::ClaudeShell => Rule::R3,
        Actor::Unknown => Rule::R4,
        Actor::External => Rule::R5,
    };
    Attribution {
        actor,
        session_id: session,
        tool_use_id: tool,
        confidence: Confidence::parse(confidence).unwrap_or(Confidence::Low),
        rule,
    }
}

impl Store {
    /// Crea o actualiza la raíz y devuelve su id.
    pub fn upsert_project(&mut self, p: &NewProject<'_>, now_ms: i64) -> Result<i64> {
        let conn = &self.conn;
        conn.prepare_cached(
            "INSERT INTO projects(root, git_dir, is_worktree, main_root, last_active)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(root) DO UPDATE SET
                 git_dir = excluded.git_dir, is_worktree = excluded.is_worktree,
                 main_root = excluded.main_root,
                 last_active = MAX(COALESCE(last_active, 0), excluded.last_active)",
        )?
        .execute(params![
            p.root,
            p.git_dir,
            p.is_worktree,
            p.main_root,
            now_ms
        ])?;
        Ok(conn
            .prepare_cached("SELECT id FROM projects WHERE root = ?1")?
            .query_row([p.root], |r| r.get(0))?)
    }

    pub fn project_root(&self, id: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .prepare_cached("SELECT root FROM projects WHERE id = ?1")?
            .query_row([id], |r| r.get(0))
            .optional()?)
    }

    /// Asigna la raíz de una sesión y la de sus herramientas que aún no tenían.
    ///
    /// Las herramientas que ya tenían raíz conservan la suya: una sesión que cambia
    /// de worktree no reescribe dónde trabajó antes. El trigger de §6.4.2 marca
    /// como sucios el agregado anterior (`0`) y el nuevo.
    pub fn assign_session_root(&mut self, session_id: &str, root_id: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.prepare_cached("UPDATE sessions SET root_id = ?1 WHERE id = ?2 AND root_id IS NOT ?1")?
            .execute(params![root_id, session_id])?;
        tx.prepare_cached(
            "UPDATE tool_calls SET root_id = ?1 WHERE session_id = ?2 AND root_id IS NULL",
        )?
        .execute(params![root_id, session_id])?;
        tx.commit()?;
        Ok(())
    }

    /// `cwd` guardado de las sesiones (solo con `record_paths`), para resolver su raíz.
    pub fn session_cwds(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, cwd FROM sessions WHERE cwd IS NOT NULL ORDER BY last_seen_at DESC",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Herramientas de las sesiones de la raíz cuyo intervalo puede cubrir
    /// `[from_ts, to_ts]`. Sin copias de historial (R-4).
    pub fn attribution_candidates(
        &self,
        root_id: i64,
        from_ts: i64,
        to_ts: i64,
        now_ms: i64,
    ) -> Result<Vec<ToolInterval>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT session_id, tool_use_id, tool, started_at, finished_at, rel_path
             FROM tool_calls
             WHERE root_id = ?1 AND is_copy = 0
               AND started_at <= ?2 AND started_at >= ?4
               AND (finished_at >= ?3
                    OR (finished_at IS NULL AND status IN ('running', 'awaiting_permission')))",
        )?;
        let rows = stmt.query_map(
            params![
                root_id,
                to_ts + EPSILON_MS,
                from_ts - DELTA_MS,
                now_ms - STALE_RUNNING_MS
            ],
            |r| {
                Ok(ToolInterval {
                    session_id: r.get(0)?,
                    tool_use_id: r.get(1)?,
                    tool: r.get(2)?,
                    started_at: r.get(3)?,
                    finished_at: r.get(4)?,
                    rel_path: r.get(5)?,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Guarda cambios con su atribución provisional, en una transacción.
    ///
    /// Rechaza (`None`) los de un día ya purgado, `day < cutoff_day`: su agregado es
    /// definitivo y recalcularlo desde la tabla cruda lo vaciaría (§6.4.2).
    /// Con `record_paths = false` no se escribe ninguna ruta.
    pub fn insert_file_events(
        &mut self,
        root_id: i64,
        changes: &[(FileChange, Attribution)],
        record_paths: bool,
        cutoff_day: &str,
    ) -> Result<Vec<Option<i64>>> {
        let tx = self.conn.transaction()?;
        let mut ids = Vec::with_capacity(changes.len());
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO file_events(root_id, rel_path, change, from_rel_path, ext, size, ts, day,
                     actor, actor_session, actor_tool_use_id, confidence, settled)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0)",
            )?;
            for (c, a) in changes {
                let day = local_day(c.ts_ms);
                if day.as_str() < cutoff_day {
                    ids.push(None);
                    continue;
                }
                let paired = a.session_id.is_some() && a.tool_use_id.is_some();
                stmt.execute(params![
                    root_id,
                    record_paths.then_some(c.rel_path.as_str()),
                    c.change.as_str(),
                    c.from_rel_path.as_deref().filter(|_| record_paths),
                    extension(&c.rel_path),
                    c.size.map(|s| s as i64),
                    c.ts_ms,
                    day,
                    a.actor.as_str(),
                    a.session_id.as_deref().filter(|_| paired),
                    a.tool_use_id.as_deref().filter(|_| paired),
                    a.confidence.as_str()
                ])?;
                ids.push(Some(tx.last_insert_rowid()));
            }
        }
        tx.commit()?;
        Ok(ids)
    }

    /// Cambia la atribución de un cambio (provisional o una subida tardía).
    pub fn update_file_event(&mut self, id: i64, a: &Attribution, settled: bool) -> Result<()> {
        let paired = a.session_id.is_some() && a.tool_use_id.is_some();
        self.conn
            .prepare_cached(
                "UPDATE file_events SET actor = ?1, actor_session = ?2, actor_tool_use_id = ?3,
                     confidence = ?4, settled = MAX(settled, ?5)
                 WHERE id = ?6",
            )?
            .execute(params![
                a.actor.as_str(),
                a.session_id.as_deref().filter(|_| paired),
                a.tool_use_id.as_deref().filter(|_| paired),
                a.confidence.as_str(),
                settled,
                id
            ])?;
        Ok(())
    }

    /// Marca como definitivas las atribuciones de estos cambios.
    pub fn settle_file_events(&mut self, ids: &[i64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached("UPDATE file_events SET settled = 1 WHERE id = ?1")?;
            for id in ids {
                stmt.execute([id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Cambios que se pueden reevaluar: los provisionales y los `unknown` desde `since_ts`.
    pub fn changes_to_reevaluate(&self, root_id: i64, since_ts: i64) -> Result<Vec<StoredChange>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, rel_path, from_rel_path, ts, settled, actor, actor_session,
                    actor_tool_use_id, confidence
             FROM file_events
             WHERE root_id = ?1 AND ts >= ?2 AND (settled = 0 OR actor = 'unknown')
             ORDER BY id",
        )?;
        let rows = stmt.query_map(params![root_id, since_ts], |r| {
            let actor: String = r.get(5)?;
            let confidence: String = r.get(8)?;
            Ok(StoredChange {
                id: r.get(0)?,
                rel_path: r.get(1)?,
                from_rel_path: r.get(2)?,
                ts: r.get(3)?,
                settled: r.get(4)?,
                attribution: read_attribution(&actor, r.get(6)?, r.get(7)?, &confidence),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Cambios provisionales de cualquier raíz (tras un reinicio se asientan sin más).
    pub fn unsettled_file_events(&self) -> Result<Vec<(i64, i64, i64)>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, root_id, ts FROM file_events WHERE settled = 0")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Cambios de una raíz, del más reciente al más antiguo, para la vista.
    pub fn file_events_page(
        &self,
        root_id: i64,
        before_id: Option<i64>,
        limit: u32,
    ) -> Result<Vec<FileEventView>> {
        let sql = format!(
            "SELECT {VIEW_COLUMNS} {VIEW_FROM}
             WHERE fe.root_id = ?1 AND fe.id < ?2 ORDER BY fe.id DESC LIMIT ?3"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(
            params![root_id, before_id.unwrap_or(i64::MAX), limit],
            read_view,
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn file_events_by_id(&self, ids: &[i64]) -> Result<Vec<FileEventView>> {
        let sql = format!("SELECT {VIEW_COLUMNS} {VIEW_FROM} WHERE fe.id = ?1");
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(v) = stmt.query_row([id], read_view).optional()? {
                out.push(v);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Batch;
    use cockpit_core::attribution::Rule;
    use cockpit_core::model::{CockpitEvent, EventKind, Source};
    use cockpit_core::privacy::ToolMeta;
    use cockpit_core::workspace::ChangeKind;

    const T0: i64 = 1_762_770_000_000;

    fn project(store: &mut Store, root: &str) -> i64 {
        store
            .upsert_project(
                &NewProject {
                    root,
                    git_dir: None,
                    is_worktree: false,
                    main_root: None,
                },
                T0,
            )
            .unwrap()
    }

    fn started(session: &str, id: &str, tool: &str, ts: i64) -> CockpitEvent {
        CockpitEvent {
            source: Source::Mod,
            session_id: session.to_owned(),
            agent_id: None,
            ts_ms: ts,
            engine_version: None,
            kind: EventKind::ToolStarted {
                tool_use_id: id.to_owned(),
                tool: tool.to_owned(),
                meta: ToolMeta {
                    rel_path: Some("a.rs".to_owned()),
                    ..Default::default()
                },
            },
        }
    }

    fn change(path: &str, ts: i64) -> FileChange {
        FileChange {
            rel_path: path.to_owned(),
            change: ChangeKind::Modified,
            from_rel_path: None,
            size: Some(10),
            ts_ms: ts,
        }
    }

    fn unknown() -> Attribution {
        Attribution {
            actor: Actor::Unknown,
            session_id: None,
            tool_use_id: None,
            confidence: Confidence::Low,
            rule: Rule::R4,
        }
    }

    #[test]
    fn projects_are_unique_by_root() {
        let mut store = Store::open_in_memory().unwrap();
        let a = project(&mut store, r"C:\w\repo");
        assert_eq!(project(&mut store, r"C:\w\repo"), a);
        assert_ne!(project(&mut store, r"C:\w\otro"), a);
        assert_eq!(
            store.project_root(a).unwrap().as_deref(),
            Some(r"C:\w\repo")
        );
    }

    #[test]
    fn tools_inherit_the_session_root_and_backfill_it() {
        let mut store = Store::open_in_memory().unwrap();
        let root = project(&mut store, r"C:\w\repo");
        let apply = |store: &mut Store, e: CockpitEvent| {
            store
                .apply(&Batch {
                    events: &[e],
                    now_ms: T0,
                    ..Default::default()
                })
                .unwrap();
        };
        apply(&mut store, started("s1", "t1", "Edit", T0));
        store.assign_session_root("s1", root).unwrap();
        apply(&mut store, started("s1", "t2", "Write", T0 + 10));
        let roots: Vec<Option<i64>> = store
            .connection()
            .prepare("SELECT root_id FROM tool_calls ORDER BY tool_use_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(roots, [Some(root), Some(root)]);

        let found = store
            .attribution_candidates(root, T0 + 5, T0 + 5, T0 + 100)
            .unwrap();
        assert_eq!(found.len(), 2, "las dos siguen en curso");
    }

    #[test]
    fn file_events_respect_paths_policy_and_the_purge_cutoff() {
        let mut store = Store::open_in_memory().unwrap();
        let root = project(&mut store, r"C:\w\repo");
        let today = local_day(T0);
        let ids = store
            .insert_file_events(
                root,
                &[
                    (change("src/a.rs", T0), unknown()),
                    (change("src/b.RS", T0 - 40 * 86_400_000), unknown()),
                ],
                true,
                &local_day(T0 - 30 * 86_400_000),
            )
            .unwrap();
        assert!(ids[0].is_some());
        assert_eq!(ids[1], None, "un día ya purgado no admite filas nuevas");

        let ids = store
            .insert_file_events(
                root,
                &[(change("x/secreto.TXT", T0), unknown())],
                false,
                &today,
            )
            .unwrap();
        let (path, ext): (Option<String>, String) = store
            .connection()
            .query_row(
                "SELECT rel_path, ext FROM file_events WHERE id = ?1",
                [ids[0].unwrap()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(path, None);
        assert_eq!(ext, ".txt");
    }

    #[test]
    fn attribution_updates_are_visible_in_the_view() {
        let mut store = Store::open_in_memory().unwrap();
        let root = project(&mut store, r"C:\w\repo");
        store
            .apply(&Batch {
                events: &[started("s1", "t1", "Edit", T0)],
                now_ms: T0,
                ..Default::default()
            })
            .unwrap();
        let id = store
            .insert_file_events(root, &[(change("a.rs", T0 + 5), unknown())], true, "")
            .unwrap()[0]
            .unwrap();
        let claude = Attribution {
            actor: Actor::Claude,
            session_id: Some("s1".to_owned()),
            tool_use_id: Some("t1".to_owned()),
            confidence: Confidence::High,
            rule: Rule::R1,
        };
        store.update_file_event(id, &claude, false).unwrap();
        let view = &store.file_events_page(root, None, 10).unwrap()[0];
        assert_eq!(view.actor, "claude");
        assert_eq!(view.tool.as_deref(), Some("Edit"));
        assert!(!view.settled);

        let pending = store.changes_to_reevaluate(root, 0).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attribution, claude);
        store.settle_file_events(&[id]).unwrap();
        assert!(store.changes_to_reevaluate(root, 0).unwrap().is_empty());
        assert!(store.unsettled_file_events().unwrap().is_empty());
    }
}
