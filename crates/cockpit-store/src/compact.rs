//! Agregación transaccional e idempotente, purga y reconciliación (SPEC §6.4.1,
//! §6.4.2).
//!
//! - Una transacción `BEGIN IMMEDIATE … COMMIT` por `(target, rk, día)`: borrar el
//!   agregado, recalcularlo entero desde la tabla cruda, insertarlo y marcar el
//!   registro como `done`. Si algo falla a mitad, SQLite deshace la transacción y
//!   el día vuelve a ser candidato en la siguiente ejecución.
//! - La purga es otra transacción, posterior, y solo toca días `done`.
//! - `rk = COALESCE(root_id, 0)` en todas partes (R-2).

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{TransactionBehavior, params};
use serde::Serialize;

use crate::{Result, Store, StoreError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Target {
    FileChangeDaily,
    ToolUsageDaily,
}

impl Target {
    pub const ALL: [Target; 2] = [Target::FileChangeDaily, Target::ToolUsageDaily];

    pub const fn name(self) -> &'static str {
        match self {
            Target::FileChangeDaily => "file_change_daily",
            Target::ToolUsageDaily => "tool_usage_daily",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CompactOptions<'a> {
    /// Solo se agregan los días `< closed_before` (`time::closed_before`).
    pub closed_before: &'a str,
    /// Se purga el detalle de rutas de los días `< purge_before` ya agregados.
    pub purge_before: &'a str,
    /// `ingest_errors` con `ts < errors_before_ms` se borran (no tienen agregado).
    pub errors_before_ms: i64,
    pub now_ms: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct CompactReport {
    /// Unidades `(target, rk, día)` agregadas.
    pub aggregated: usize,
    /// Unidades que fallaron: siguen pendientes y su detalle no se purga.
    pub failed: usize,
    pub tool_paths_purged: usize,
    pub file_events_purged: usize,
    pub errors_purged: usize,
}

/// Clave de `file_change_daily`: `(actor, change, ext)`.
type FileGroup = (String, String, String);
/// `(eventos, rutas distintas, eventos sin ruta)` de un grupo.
type FileCounts = (i64, BTreeSet<String>, i64);

/// Percentil por rango más cercano sobre valores ya ordenados.
fn percentile(sorted: &[i64], p: f64) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((p * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    Some(sorted[rank - 1])
}

impl Store {
    /// Ciclo completo: agregación de los días cerrados, purga y checkpoint.
    pub fn compact(&mut self, opts: &CompactOptions<'_>) -> Result<CompactReport> {
        self.compact_with(opts, &|_, _, _| false)
    }

    /// `fail` decide, por unidad, si se inyecta un fallo entre los pasos 3 y 5.
    pub(crate) fn compact_with(
        &mut self,
        opts: &CompactOptions<'_>,
        fail: &dyn Fn(Target, i64, &str) -> bool,
    ) -> Result<CompactReport> {
        let mut report = CompactReport::default();
        for target in Target::ALL {
            for (rk, day) in self.aggregation_candidates(target, opts.closed_before)? {
                match self.aggregate_day(target, rk, &day, opts.now_ms, fail(target, rk, &day)) {
                    Ok(()) => report.aggregated += 1,
                    Err(_) => report.failed += 1,
                }
            }
        }
        let (tool_paths, file_events, errors) = self.purge(opts)?;
        report.tool_paths_purged = tool_paths;
        report.file_events_purged = file_events;
        report.errors_purged = errors;
        self.conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        Ok(report)
    }

    /// Días candidatos del target: los cerrados sin `done`, más los `dirty`.
    pub fn aggregation_candidates(
        &self,
        target: Target,
        closed_before: &str,
    ) -> Result<Vec<(i64, String)>> {
        let raw = match target {
            Target::FileChangeDaily => {
                "SELECT COALESCE(root_id, 0) AS rk, day FROM file_events WHERE day < ?1 GROUP BY rk, day"
            }
            Target::ToolUsageDaily => {
                "SELECT COALESCE(root_id, 0) AS rk, day FROM tool_calls
                 WHERE day < ?1 AND is_copy = 0 GROUP BY rk, day"
            }
        };
        // `A EXCEPT B UNION C` se evalúa de izquierda a derecha: (A − done) ∪ dirty.
        let sql = format!(
            "{raw}
             EXCEPT SELECT root_id, day FROM aggregation_ledger WHERE target = ?2 AND state = 'done'
             UNION SELECT root_id, day FROM aggregation_ledger WHERE target = ?2 AND state = 'dirty'
             ORDER BY 2, 1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![closed_before, target.name()], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Unidad de trabajo de §6.4.2, pasos 1–5, en una transacción `IMMEDIATE`.
    fn aggregate_day(
        &mut self,
        target: Target,
        rk: i64,
        day: &str,
        now_ms: i64,
        inject_failure: bool,
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // 1. Recalcular entero, no de forma incremental: repetirlo no duplica nada.
        tx.execute(
            &format!(
                "DELETE FROM {} WHERE root_id = ?1 AND day = ?2",
                target.name()
            ),
            params![rk, day],
        )?;
        // 2 y 3. Leer las filas crudas del día, agrupar en Rust e insertar.
        let source_rows = match target {
            Target::ToolUsageDaily => {
                let mut by_tool: BTreeMap<String, (i64, i64, i64, Vec<i64>)> = BTreeMap::new();
                let mut n = 0i64;
                {
                    let mut stmt = tx.prepare_cached(
                        "SELECT tool, status, duration_ms FROM tool_calls
                         WHERE COALESCE(root_id, 0) = ?1 AND day = ?2 AND is_copy = 0",
                    )?;
                    let mut rows = stmt.query(params![rk, day])?;
                    while let Some(r) = rows.next()? {
                        n += 1;
                        let tool: String = r.get(0)?;
                        let status: String = r.get(1)?;
                        let duration: Option<i64> = r.get(2)?;
                        let e = by_tool.entry(tool).or_default();
                        e.0 += 1;
                        e.1 += i64::from(status == "error");
                        e.2 += i64::from(status == "interrupted");
                        e.3.extend(duration);
                    }
                }
                let mut insert = tx.prepare_cached(
                    "INSERT INTO tool_usage_daily(root_id, day, tool, calls, errors, interrupted,
                         p50_ms, p95_ms, total_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                )?;
                for (tool, (calls, errors, interrupted, mut durations)) in by_tool {
                    durations.sort_unstable();
                    insert.execute(params![
                        rk,
                        day,
                        tool,
                        calls,
                        errors,
                        interrupted,
                        percentile(&durations, 0.50),
                        percentile(&durations, 0.95),
                        durations.iter().sum::<i64>()
                    ])?;
                }
                n
            }
            Target::FileChangeDaily => {
                let mut groups: BTreeMap<FileGroup, FileCounts> = BTreeMap::new();
                let mut n = 0i64;
                {
                    let mut stmt = tx.prepare_cached(
                        "SELECT actor, change, ext, rel_path FROM file_events
                         WHERE COALESCE(root_id, 0) = ?1 AND day = ?2",
                    )?;
                    let mut rows = stmt.query(params![rk, day])?;
                    while let Some(r) = rows.next()? {
                        n += 1;
                        // `rel_path` solo se usa aquí, en memoria, para contar ficheros.
                        let path: Option<String> = r.get(3)?;
                        let e = groups.entry((r.get(0)?, r.get(1)?, r.get(2)?)).or_default();
                        e.0 += 1;
                        match path {
                            Some(p) => {
                                e.1.insert(p.to_lowercase());
                            }
                            None => e.2 += 1,
                        }
                    }
                }
                let mut insert = tx.prepare_cached(
                    "INSERT INTO file_change_daily(root_id, day, actor, change, ext, files_touched, events)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )?;
                for ((actor, change, ext), (events, paths, pathless)) in groups {
                    insert.execute(params![
                        rk,
                        day,
                        actor,
                        change,
                        ext,
                        paths.len() as i64 + pathless,
                        events
                    ])?;
                }
                n
            }
        };
        if inject_failure {
            // Sin `commit`: al soltar `tx`, SQLite deshace los pasos 1–3.
            return Err(StoreError::Interrupted);
        }
        // 4. Registro.
        tx.prepare_cached(
            "INSERT INTO aggregation_ledger(target, root_id, day, state, source_rows, aggregated_at)
             VALUES (?1, ?2, ?3, 'done', ?4, ?5)
             ON CONFLICT(target, root_id, day) DO UPDATE
               SET state = 'done', source_rows = excluded.source_rows,
                   aggregated_at = excluded.aggregated_at",
        )?
        .execute(params![target.name(), rk, day, source_rows, now_ms])?;
        // 5.
        tx.commit()?;
        Ok(())
    }

    /// Purga del detalle de rutas de los días ya agregados (§6.4.1, §6.4.2).
    ///
    /// No dispara triggers de suciedad: en `tool_calls` solo cambian `rel_path` y
    /// `host`, que no están en `UPDATE OF`, y `file_events` no tiene `AFTER DELETE`.
    fn purge(&mut self, opts: &CompactOptions<'_>) -> Result<(usize, usize, usize)> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut tool_paths = tx.execute(
            "UPDATE tool_calls SET rel_path = NULL, host = NULL
             WHERE day < ?1 AND (rel_path IS NOT NULL OR host IS NOT NULL)
               AND EXISTS (SELECT 1 FROM aggregation_ledger l
                            WHERE l.target = 'tool_usage_daily' AND l.state = 'done'
                              AND l.root_id = COALESCE(tool_calls.root_id, 0)
                              AND l.day = tool_calls.day)",
            [opts.purge_before],
        )?;
        // Las copias de historial no se agregan: su target es el de la fila canónica.
        tool_paths += tx.execute(
            "UPDATE tool_calls SET rel_path = NULL, host = NULL
             WHERE day < ?1 AND is_copy = 1 AND (rel_path IS NOT NULL OR host IS NOT NULL)",
            [opts.purge_before],
        )?;
        let file_events = tx.execute(
            "DELETE FROM file_events
             WHERE day < ?1
               AND EXISTS (SELECT 1 FROM aggregation_ledger l
                            WHERE l.target = 'file_change_daily' AND l.state = 'done'
                              AND l.root_id = COALESCE(file_events.root_id, 0)
                              AND l.day = file_events.day)",
            [opts.purge_before],
        )?;
        tx.execute(
            "UPDATE aggregation_ledger SET purged_at = ?2
             WHERE day < ?1 AND state = 'done' AND purged_at IS NULL",
            params![opts.purge_before, opts.now_ms],
        )?;
        let errors = tx.execute(
            "DELETE FROM ingest_errors WHERE ts < ?1",
            [opts.errors_before_ms],
        )?;
        tx.commit()?;
        Ok((tool_paths, file_events, errors))
    }

    /// Compara cada agregado con su tabla cruda en los días no purgados y marca
    /// como `dirty` los días `done` que no cuadran. Devuelve cuántos (Diagnóstico).
    pub fn reconcile(&mut self, purge_before: &str) -> Result<usize> {
        let mut marked = 0;
        for (target, root_id, day) in self.aggregate_mismatches(purge_before)? {
            marked += self.conn.execute(
                "UPDATE aggregation_ledger SET state = 'dirty'
                 WHERE target = ?1 AND root_id = ?2 AND day = ?3 AND state = 'done'",
                params![target.name(), root_id, day],
            )?;
        }
        Ok(marked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Batch;
    use crate::workspace::NewProject;
    use cockpit_core::attribution::{Actor, Attribution, Confidence, Rule};
    use cockpit_core::model::{CockpitEvent, EventKind, Source, ToolStatus};
    use cockpit_core::privacy::ToolMeta;
    use cockpit_core::time::local_day;
    use cockpit_core::workspace::{ChangeKind, FileChange};

    const DAY: i64 = 86_400_000;
    /// Mediodía UTC: el día local es el mismo en cualquier zona de Europa.
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

    /// Una herramienta terminada, con ruta, en la sesión dada.
    fn tool(
        store: &mut Store,
        session: &str,
        id: &str,
        name: &str,
        ts: i64,
        ms: i64,
        status: ToolStatus,
    ) {
        let events = [
            ev(session, ts, EventKind::TranscriptFile { created_at_ms: ts }),
            ev(
                session,
                ts,
                EventKind::ToolStarted {
                    tool_use_id: id.to_owned(),
                    tool: name.to_owned(),
                    meta: ToolMeta {
                        rel_path: Some(format!("src/{id}.rs")),
                        host: (name == "WebFetch").then(|| "example.com".to_owned()),
                        ..Default::default()
                    },
                },
            ),
            ev(
                session,
                ts + ms,
                EventKind::ToolFinished {
                    tool_use_id: id.to_owned(),
                    status,
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

    fn file(store: &mut Store, root: i64, path: &str, ts: i64, actor: Actor) {
        let a = Attribution {
            actor,
            session_id: None,
            tool_use_id: None,
            confidence: Confidence::High,
            rule: Rule::R5,
        };
        let c = FileChange {
            rel_path: path.to_owned(),
            change: ChangeKind::Modified,
            from_rel_path: None,
            size: None,
            ts_ms: ts,
        };
        store.insert_file_events(root, &[(c, a)], true, "").unwrap();
    }

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

    /// Opciones con `now` en `T0 + days` (a mediodía: el día anterior ya está cerrado).
    fn at(days: i64) -> (String, String, i64) {
        let now = T0 + days * DAY;
        (local_day(now), local_day(now - 30 * DAY), now)
    }

    fn compact(store: &mut Store, days: i64) -> CompactReport {
        let (closed, purge, now) = at(days);
        store
            .compact(&CompactOptions {
                closed_before: &closed,
                purge_before: &purge,
                errors_before_ms: now - 30 * DAY,
                now_ms: now,
            })
            .unwrap()
    }

    fn dump(store: &Store, sql: &str) -> Vec<String> {
        let mut stmt = store.connection().prepare(sql).unwrap();
        let n = stmt.column_count();
        stmt.query_map([], |r| {
            Ok((0..n)
                .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                .collect::<Vec<_>>()
                .join("|"))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
    }

    fn aggregates(store: &Store) -> Vec<Vec<String>> {
        vec![
            dump(store, "SELECT * FROM tool_usage_daily ORDER BY 1, 2, 3"),
            dump(
                store,
                "SELECT * FROM file_change_daily ORDER BY 1, 2, 3, 4, 5",
            ),
            dump(
                store,
                "SELECT target, root_id, day, state, source_rows FROM aggregation_ledger ORDER BY 1, 2, 3",
            ),
        ]
    }

    fn ledger_state(store: &Store, target: Target, rk: i64, day: &str) -> Option<String> {
        store
            .connection()
            .query_row(
                "SELECT state FROM aggregation_ledger WHERE target = ?1 AND root_id = ?2 AND day = ?3",
                params![target.name(), rk, day],
                |r| r.get(0),
            )
            .ok()
    }

    fn seeded() -> (Store, i64) {
        let mut store = Store::open_in_memory().unwrap();
        let root = project(&mut store, r"C:\w\repo");
        tool(&mut store, "s1", "a", "Edit", T0, 100, ToolStatus::Ok);
        tool(
            &mut store,
            "s1",
            "b",
            "Edit",
            T0 + 1000,
            300,
            ToolStatus::Error,
        );
        tool(
            &mut store,
            "s1",
            "c",
            "Bash",
            T0 + 2000,
            50,
            ToolStatus::Interrupted,
        );
        tool(&mut store, "s2", "d", "Edit", T0 + DAY, 700, ToolStatus::Ok);
        file(&mut store, root, "src/a.rs", T0, Actor::Claude);
        file(&mut store, root, "SRC/A.rs", T0 + 10, Actor::Claude);
        file(&mut store, root, "README", T0 + 20, Actor::External);
        (store, root)
    }

    #[test]
    fn aggregates_are_computed_from_the_raw_rows() {
        let (mut store, root) = seeded();
        let report = compact(&mut store, 2);
        assert_eq!(report.aggregated, 3, "{report:?}");
        let d0 = local_day(T0);
        let edit: (i64, i64, i64, Option<i64>, Option<i64>, i64) = store
            .connection()
            .query_row(
                "SELECT calls, errors, interrupted, p50_ms, p95_ms, total_ms FROM tool_usage_daily
                 WHERE root_id = 0 AND day = ?1 AND tool = 'Edit'",
                [&d0],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(edit, (2, 1, 0, Some(100), Some(300), 400));
        let files: Vec<(String, String, i64, i64)> = store
            .connection()
            .prepare("SELECT actor, ext, files_touched, events FROM file_change_daily WHERE root_id = ?1 ORDER BY 1")
            .unwrap()
            .query_map([root], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        // Las dos ediciones de `src/a.rs` (con distinta caja) son un solo fichero.
        assert_eq!(
            files,
            [
                ("claude".to_owned(), ".rs".to_owned(), 1, 2),
                ("external".to_owned(), String::new(), 1, 1)
            ]
        );
        // Hoy (T0 + 2 días) no está cerrado: no se agrega nada de hoy.
        assert!(
            store
                .aggregation_candidates(Target::ToolUsageDaily, &local_day(T0 + 2 * DAY))
                .unwrap()
                .is_empty()
        );
    }

    /// §6.4.2: ejecutar la compactación dos veces seguidas da un resultado idéntico.
    #[test]
    fn compacting_twice_is_idempotent() {
        let (mut store, _) = seeded();
        compact(&mut store, 2);
        let first = aggregates(&store);
        let report = compact(&mut store, 2);
        assert_eq!(report.aggregated, 0);
        assert_eq!(aggregates(&store), first);
    }

    /// §6.4.2: un fallo entre los pasos 3 y 5 deja el día pendiente, y la siguiente
    /// ejecución lo agrega igual que una ejecución limpia.
    #[test]
    fn an_interrupted_unit_is_redone_identically() {
        let (mut clean, _) = seeded();
        compact(&mut clean, 2);

        let (mut store, _) = seeded();
        let (closed, purge, now) = at(2);
        let d0 = local_day(T0);
        let report = store
            .compact_with(
                &CompactOptions {
                    closed_before: &closed,
                    purge_before: &purge,
                    errors_before_ms: now,
                    now_ms: now,
                },
                &|t, _, day| t == Target::ToolUsageDaily && day == d0,
            )
            .unwrap();
        assert_eq!(report.failed, 1);
        assert_eq!(ledger_state(&store, Target::ToolUsageDaily, 0, &d0), None);
        let partial: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM tool_usage_daily WHERE day = ?1",
                [&d0],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(partial, 0, "no quedan agregados parciales");

        compact(&mut store, 2);
        assert_eq!(aggregates(&store), aggregates(&clean));
    }

    /// §6.4.2: una inserción tardía marca el día como sucio y se recalcula.
    #[test]
    fn a_late_row_dirties_its_day_and_is_aggregated() {
        let (mut store, _) = seeded();
        compact(&mut store, 2);
        let d0 = local_day(T0);
        tool(
            &mut store,
            "s3",
            "tarde",
            "Edit",
            T0 + 5000,
            10,
            ToolStatus::Ok,
        );
        assert_eq!(
            ledger_state(&store, Target::ToolUsageDaily, 0, &d0).as_deref(),
            Some("dirty")
        );
        compact(&mut store, 2);
        let calls: i64 = store
            .connection()
            .query_row(
                "SELECT calls FROM tool_usage_daily WHERE day = ?1 AND tool = 'Edit'",
                [&d0],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(calls, 3);
    }

    /// §6.4.2: ningún día se purga sin estar en `done`.
    #[test]
    fn nothing_is_purged_unless_its_day_is_done() {
        let (mut store, root) = seeded();
        let d0 = local_day(T0);
        let (closed, purge, now) = at(40);
        let opts = CompactOptions {
            closed_before: &closed,
            purge_before: &purge,
            errors_before_ms: now - 30 * DAY,
            now_ms: now,
        };
        // Falla la agregación del día 0 en los dos targets.
        let report = store.compact_with(&opts, &|_, _, day| day == d0).unwrap();
        assert_eq!(report.failed, 2);
        let kept: i64 = store
            .connection()
            .query_row(
                "SELECT (SELECT COUNT(*) FROM tool_calls WHERE day = ?1 AND rel_path IS NOT NULL)
                      + (SELECT COUNT(*) FROM file_events WHERE day = ?1)",
                [&d0],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 3 + 3, "el detalle de un día sin agregar se conserva");
        // El día 1 sí se agregó y se purgó.
        let d1_paths: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM tool_calls WHERE day = ?1 AND rel_path IS NOT NULL",
                [local_day(T0 + DAY)],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(d1_paths, 0);

        store.compact(&opts).unwrap();
        let left: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM file_events WHERE root_id = ?1",
                [root],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
    }

    /// F4a: tras la compactación no queda ninguna ruta de más de 30 días.
    #[test]
    fn no_path_older_than_thirty_days_survives() {
        let (mut store, root) = seeded();
        tool(&mut store, "s4", "web", "WebFetch", T0, 5, ToolStatus::Ok);
        file(&mut store, root, "nuevo.rs", T0 + 39 * DAY, Actor::Claude);
        store
            .connection()
            .execute(
                "INSERT INTO ingest_errors(ts, path, kind, detail, count) VALUES (?1, 'C:\\x.jsonl', 'corrupt', '', 1)",
                [T0],
            )
            .unwrap();
        let report = compact(&mut store, 40);
        assert_eq!(report.errors_purged, 1);
        let cutoff = local_day(T0 + 10 * DAY);
        let old_paths: i64 = store
            .connection()
            .query_row(
                "SELECT (SELECT COUNT(*) FROM tool_calls WHERE day < ?1 AND (rel_path IS NOT NULL OR host IS NOT NULL))
                      + (SELECT COUNT(*) FROM file_events WHERE day < ?1)
                      + (SELECT COUNT(*) FROM ingest_errors)",
                [&cutoff],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_paths, 0);
        // Lo reciente sigue, y los agregados conservan lo purgado.
        let recent: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM file_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(recent, 1);
        let calls: i64 = store
            .connection()
            .query_row("SELECT SUM(calls) FROM tool_usage_daily", [], |r| r.get(0))
            .unwrap();
        assert_eq!(calls, 5);
    }

    /// R-2: reasignar `root_id` de NULL a 7 ensucia `(0, día)` y `(7, día)`.
    #[test]
    fn reassigning_a_root_dirties_both_sides() {
        let (mut store, _) = seeded();
        store
            .connection()
            .execute(
                "INSERT INTO projects(id, root) VALUES (7, 'C:\\w\\siete')",
                [],
            )
            .unwrap();
        tool(&mut store, "s7", "x", "Edit", T0 + 3000, 1, ToolStatus::Ok);
        compact(&mut store, 2);
        let d0 = local_day(T0);
        // La fila de (7, día) aún no existe: se crea al agregarla por primera vez.
        store.assign_session_root("s7", 7).unwrap();
        assert_eq!(
            ledger_state(&store, Target::ToolUsageDaily, 0, &d0).as_deref(),
            Some("dirty")
        );
        compact(&mut store, 2);
        assert_eq!(
            ledger_state(&store, Target::ToolUsageDaily, 7, &d0).as_deref(),
            Some("done")
        );
        let by_root = dump(
            &store,
            &format!(
                "SELECT root_id, SUM(calls) FROM tool_usage_daily WHERE day = '{d0}' GROUP BY 1 ORDER BY 1"
            ),
        );
        assert_eq!(by_root, ["Integer(0)|Integer(3)", "Integer(7)|Integer(1)"]);

        // Ahora que (7, día) existe en el registro, devolverla a otra raíz ensucia los dos.
        store
            .connection()
            .execute(
                "UPDATE tool_calls SET root_id = NULL WHERE session_id = 's7'",
                [],
            )
            .unwrap();
        assert_eq!(
            ledger_state(&store, Target::ToolUsageDaily, 0, &d0).as_deref(),
            Some("dirty")
        );
        assert_eq!(
            ledger_state(&store, Target::ToolUsageDaily, 7, &d0).as_deref(),
            Some("dirty")
        );
        compact(&mut store, 2);
        let by_root = dump(
            &store,
            &format!(
                "SELECT root_id, SUM(calls) FROM tool_usage_daily WHERE day = '{d0}' GROUP BY 1 ORDER BY 1"
            ),
        );
        assert_eq!(
            by_root,
            ["Integer(0)|Integer(4)"],
            "el agregado de 7 se vacía"
        );
    }

    /// R-2: cambiar `day` ensucia el día anterior y el nuevo.
    #[test]
    fn changing_the_day_dirties_both_days() {
        let (mut store, _) = seeded();
        compact(&mut store, 2);
        let (d0, d1) = (local_day(T0), local_day(T0 + DAY));
        store
            .connection()
            .execute(
                "UPDATE tool_calls SET day = ?1 WHERE tool_use_id = 'a'",
                [&d1],
            )
            .unwrap();
        assert_eq!(
            ledger_state(&store, Target::ToolUsageDaily, 0, &d0).as_deref(),
            Some("dirty")
        );
        assert_eq!(
            ledger_state(&store, Target::ToolUsageDaily, 0, &d1).as_deref(),
            Some("dirty")
        );
    }

    /// R-2: la purga no deja ningún día como `dirty`.
    #[test]
    fn purging_leaves_every_day_done() {
        let (mut store, _) = seeded();
        compact(&mut store, 40);
        let dirty: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM aggregation_ledger WHERE state <> 'done' OR purged_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dirty, 0);
    }

    /// R-4: la misma herramienta en la sesión original y en su copia cuenta 1,
    /// se inserte primero la que se inserte.
    #[test]
    fn a_history_copy_counts_once() {
        for copy_first in [false, true] {
            let mut store = Store::open_in_memory().unwrap();
            let original = |s: &mut Store| tool(s, "orig", "t1", "Edit", T0, 10, ToolStatus::Ok);
            let copy = |s: &mut Store| {
                // La copia es un fichero creado una hora después con las mismas marcas.
                s.apply(&Batch {
                    events: &[ev(
                        "zcopia",
                        T0 + 3_600_000,
                        EventKind::TranscriptFile {
                            created_at_ms: T0 + 3_600_000,
                        },
                    )],
                    now_ms: T0,
                    ..Default::default()
                })
                .unwrap();
                tool(s, "zcopia", "t1", "Edit", T0, 10, ToolStatus::Ok);
            };
            if copy_first {
                copy(&mut store);
                original(&mut store);
            } else {
                original(&mut store);
                copy(&mut store);
            }
            compact(&mut store, 2);
            let calls: i64 = store
                .connection()
                .query_row("SELECT SUM(calls) FROM tool_usage_daily", [], |r| r.get(0))
                .unwrap();
            assert_eq!(calls, 1, "copia primero: {copy_first}");
        }
    }

    #[test]
    fn reconciliation_flags_days_that_do_not_match() {
        let (mut store, _) = seeded();
        compact(&mut store, 2);
        let purge = local_day(T0 - 30 * DAY);
        assert_eq!(store.reconcile(&purge).unwrap(), 0);
        store
            .connection()
            .execute(
                "UPDATE tool_usage_daily SET calls = calls + 1 WHERE tool = 'Bash'",
                [],
            )
            .unwrap();
        assert_eq!(store.reconcile(&purge).unwrap(), 1);
        compact(&mut store, 2);
        assert_eq!(store.reconcile(&purge).unwrap(), 0);
    }

    #[test]
    fn nearest_rank_percentiles() {
        assert_eq!(percentile(&[], 0.5), None);
        assert_eq!(percentile(&[7], 0.95), Some(7));
        let v: Vec<i64> = (1..=20).collect();
        assert_eq!(percentile(&v, 0.50), Some(10));
        assert_eq!(percentile(&v, 0.95), Some(19));
    }
}
