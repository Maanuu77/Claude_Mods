//! Almacenamiento de DEV COCKPIT: SQLite con `rusqlite` en WAL (SPEC §6.4).
//!
//! `Store` es el único escritor. Todas las escrituras son upserts idempotentes:
//! volver a aplicar los mismos eventos no cambia nada.
//!
//! El hilo único que escribe es el de la canalización en vivo (`cockpit-ingest`);
//! las consultas de la interfaz abren conexiones de solo lectura.

mod compact;
mod diagnostics;
mod history;
mod project_cache;
mod schema;
mod summary;
mod tasks;
mod workspace;

use std::collections::HashMap;
use std::path::Path;

use cockpit_core::model::{CockpitEvent, EventKind, TokenUsage, ToolStatus};
use cockpit_core::tasks::TaskSource;
use cockpit_core::time::local_day;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

pub use compact::{CompactOptions, CompactReport, Target};
pub use diagnostics::{AggregationHealth, Diagnostics, EngineSeen, ErrorGroup, PathDetail};
pub use history::{DayTokens, GroupTokens, History, LimitPoint, Tokens, ToolStats};
pub use schema::SCHEMA_VERSION;
pub use summary::{SessionSummary, ToolView, UsageView};
pub use tasks::{ACTIVITY_MINUTES, ActivityView, AgentView, TaskItemView, TaskListView};
pub use workspace::{FileEventView, NewProject, StoredChange, ToolInterval};

/// Nombre del fichero de la base de datos dentro de `%APPDATA%\dev-cockpit\`.
pub const DB_FILE_NAME: &str = "cockpit.db";

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("error de SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Una unidad de la compactación se abandonó antes de su `COMMIT`.
    #[error("compactación interrumpida")]
    Interrupted,
}

/// Estado guardado de una instancia del mod (SPEC §6.2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModInstance {
    pub session_id: String,
    pub hello_seq: i64,
    pub ack_seq: i64,
}

/// Posición de lectura de un fichero de transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOffset {
    pub path: String,
    pub size: u64,
    pub offset: u64,
    pub mtime_ms: i64,
    /// Versión del parser con la que se leyó hasta `offset`
    /// (`cockpit_core::transcript::PARSER_VERSION`).
    pub parser_version: u32,
}

/// Incidencia de la ingesta. `detail` es un nombre de tipo, nunca contenido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestError {
    pub path: String,
    pub kind: &'static str,
    pub detail: String,
    pub count: u64,
}

/// Lo que se escribe en una sola transacción.
#[derive(Debug, Default)]
pub struct Batch<'a> {
    pub events: &'a [CockpitEvent],
    /// Posiciones alcanzadas **después** de estos eventos.
    pub offsets: &'a [FileOffset],
    pub errors: &'a [IngestError],
    /// Instante de la escritura (ms), para las columnas de auditoría.
    pub now_ms: i64,
    /// Marca de agua de `/v1/events`: `(instance_id, último seq guardado)`.
    pub mod_ack: Option<(&'a str, i64)>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BatchStats {
    /// Resultados de herramienta sin su `tool_use` en la base.
    pub orphan_results: u64,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Totals {
    pub sessions: u64,
    pub api_messages: u64,
    pub tool_calls: u64,
    pub tool_call_copies: u64,
    pub api_errors: u64,
    pub usage: TokenUsage,
}

/// Comparación de una sesión con su línea `cost-state`.
#[derive(Debug, Clone, PartialEq)]
pub struct CostStateCheck {
    pub session_id: String,
    pub deduplicated: TokenUsage,
    pub cost_state: TokenUsage,
}

impl CostStateCheck {
    pub fn matches(&self) -> bool {
        self.deduplicated == self.cost_state
    }
}

pub struct Store {
    conn: Connection,
    /// Adaptadores de tareas desactivados (A3 es desactivable, SPEC §8).
    disabled_sources: Vec<TaskSource>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    /// Conexión de solo lectura para las consultas de la interfaz. No migra ni escribe.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Self {
            conn,
            disabled_sources: Vec::new(),
        })
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        schema::migrate(&mut conn)?;
        Ok(Self {
            conn,
            disabled_sources: Vec::new(),
        })
    }

    /// Aplica un lote en una transacción: o entra entero, o no entra nada.
    pub fn apply(&mut self, batch: &Batch<'_>) -> Result<BatchStats> {
        let tx = self.conn.transaction()?;
        let mut stats = BatchStats::default();
        let mut touched = tasks::Touched::new();
        upsert_sessions(&tx, batch.events)?;
        tasks::touch_agents(&tx, batch.events)?;
        for event in batch.events {
            apply_event(&tx, event, &mut stats, &mut touched)?;
        }
        // Cada lista con operaciones nuevas se reproduce una sola vez por lote.
        for (session_id, source) in &touched {
            tasks::rebuild(&tx, session_id, *source)?;
        }
        for o in batch.offsets {
            tx.prepare_cached(
                "INSERT INTO file_offsets(path, size, offset, mtime_ms, updated_at, parser_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(path) DO UPDATE SET size = excluded.size, offset = excluded.offset,
                     mtime_ms = excluded.mtime_ms, updated_at = excluded.updated_at,
                     parser_version = excluded.parser_version",
            )?
            .execute(params![
                o.path,
                o.size as i64,
                o.offset as i64,
                o.mtime_ms,
                batch.now_ms,
                o.parser_version
            ])?;
        }
        for e in batch.errors {
            tx.prepare_cached(
                "INSERT INTO ingest_errors(ts, path, kind, detail, count) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(path, kind, detail) DO UPDATE SET
                     count = count + excluded.count, ts = excluded.ts",
            )?
            .execute(params![batch.now_ms, e.path, e.kind, e.detail, e.count as i64])?;
        }
        if let Some((instance_id, seq)) = batch.mod_ack {
            tx.prepare_cached(
                "UPDATE mod_instances SET ack_seq = MAX(ack_seq, ?1) WHERE instance_id = ?2",
            )?
            .execute(params![seq, instance_id])?;
        }
        tx.commit()?;
        Ok(stats)
    }

    pub fn file_offset(&self, path: &str) -> Result<Option<FileOffset>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT size, offset, mtime_ms, parser_version FROM file_offsets WHERE path = ?1",
            )?
            .query_row([path], |r| {
                Ok(FileOffset {
                    path: path.to_owned(),
                    size: r.get::<_, i64>(0)? as u64,
                    offset: r.get::<_, i64>(1)? as u64,
                    mtime_ms: r.get(2)?,
                    parser_version: r.get(3)?,
                })
            })
            .optional()?)
    }

    pub fn totals(&self) -> Result<Totals> {
        let count = |sql: &str| -> rusqlite::Result<u64> {
            self.conn
                .query_row(sql, [], |r| r.get::<_, i64>(0))
                .map(|n| n as u64)
        };
        let usage = self.conn.query_row(
            "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cache_read_tokens), 0), COALESCE(SUM(cache_creation_tokens), 0)
             FROM api_messages",
            [],
            read_usage,
        )?;
        Ok(Totals {
            sessions: count("SELECT COUNT(*) FROM sessions")?,
            api_messages: count("SELECT COUNT(*) FROM api_messages")?,
            tool_calls: count("SELECT COUNT(*) FROM tool_calls WHERE is_copy = 0")?,
            tool_call_copies: count("SELECT COUNT(*) FROM tool_calls WHERE is_copy = 1")?,
            api_errors: count("SELECT COUNT(*) FROM api_errors")?,
            usage,
        })
    }

    /// Tokens deduplicados de cada sesión frente a su última línea `cost-state`.
    pub fn cost_state_checks(&self) -> Result<Vec<CostStateCheck>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id,
                    COALESCE(SUM(m.input_tokens), 0), COALESCE(SUM(m.output_tokens), 0),
                    COALESCE(SUM(m.cache_read_tokens), 0), COALESCE(SUM(m.cache_creation_tokens), 0),
                    s.cost_state_input, s.cost_state_output,
                    s.cost_state_cache_read, s.cost_state_cache_creation
             FROM sessions s LEFT JOIN api_messages m ON m.session_id = s.id
             WHERE s.cost_state_usd IS NOT NULL
             GROUP BY s.id ORDER BY s.started_at, s.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(CostStateCheck {
                session_id: r.get(0)?,
                deduplicated: TokenUsage {
                    input: r.get::<_, i64>(1)? as u64,
                    output: r.get::<_, i64>(2)? as u64,
                    cache_read: r.get::<_, i64>(3)? as u64,
                    cache_creation: r.get::<_, i64>(4)? as u64,
                },
                cost_state: TokenUsage {
                    input: r.get::<_, i64>(5)? as u64,
                    output: r.get::<_, i64>(6)? as u64,
                    cache_read: r.get::<_, i64>(7)? as u64,
                    cache_creation: r.get::<_, i64>(8)? as u64,
                },
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Instancia del mod registrada con `/v1/hello`.
    pub fn mod_instance(&self, instance_id: &str) -> Result<Option<ModInstance>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT session_id, hello_seq, ack_seq FROM mod_instances WHERE instance_id = ?1",
            )?
            .query_row([instance_id], |r| {
                Ok(ModInstance {
                    session_id: r.get(0)?,
                    hello_seq: r.get(1)?,
                    ack_seq: r.get(2)?,
                })
            })
            .optional()?)
    }

    /// Registra un `hello`. Repetirlo no crea filas ni retrocede la marca de agua.
    pub fn record_hello(
        &mut self,
        instance_id: &str,
        session_id: &str,
        hello_seq: i64,
        engine_version: Option<&str>,
        mod_version: Option<&str>,
        now_ms: i64,
    ) -> Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO mod_instances(instance_id, session_id, hello_seq, engine_version,
                                           mod_version, first_hello_at, last_hello_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 ON CONFLICT(instance_id) DO UPDATE SET
                     hello_seq = MAX(hello_seq, excluded.hello_seq),
                     last_hello_at = excluded.last_hello_at",
            )?
            .execute(params![
                instance_id,
                session_id,
                hello_seq,
                engine_version,
                mod_version,
                now_ms
            ])?;
        Ok(())
    }

    /// Acceso de solo lectura para consultas y tests.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

fn read_usage(r: &rusqlite::Row<'_>) -> rusqlite::Result<TokenUsage> {
    Ok(TokenUsage {
        input: r.get::<_, i64>(0)? as u64,
        output: r.get::<_, i64>(1)? as u64,
        cache_read: r.get::<_, i64>(2)? as u64,
        cache_creation: r.get::<_, i64>(3)? as u64,
    })
}

#[derive(Default)]
struct SessionAgg<'a> {
    first: Option<i64>,
    last: Option<i64>,
    cwd: Option<&'a str>,
    entrypoint: Option<&'a str>,
    version: Option<&'a str>,
    file_created_at: Option<i64>,
    mask: i64,
}

/// Las sesiones se escriben antes que el resto del lote: son el destino de las
/// claves foráneas y su orden decide cuál es la sesión canónica (R-4).
fn upsert_sessions(tx: &Transaction<'_>, events: &[CockpitEvent]) -> Result<()> {
    let mut sessions: HashMap<&str, SessionAgg<'_>> = HashMap::new();
    for e in events {
        let agg = sessions.entry(e.session_id.as_str()).or_default();
        agg.mask |= e.source.mask();
        match &e.kind {
            // `cost-state` no lleva marca de tiempo propia.
            EventKind::CostState { .. } => continue,
            EventKind::TranscriptFile { created_at_ms } => {
                agg.file_created_at = Some(match agg.file_created_at {
                    Some(prev) => prev.min(*created_at_ms),
                    None => *created_at_ms,
                });
                // La creación del fichero puede adelantar el inicio de la sesión,
                // pero no es actividad: no mueve `last_seen_at`.
                agg.first = Some(agg.first.map_or(e.ts_ms, |t| t.min(e.ts_ms)));
                continue;
            }
            EventKind::SessionSeen { cwd, entrypoint } => {
                if cwd.is_some() {
                    agg.cwd = cwd.as_deref();
                }
                if entrypoint.is_some() {
                    agg.entrypoint = entrypoint.as_deref();
                }
                if e.engine_version.is_some() {
                    agg.version = e.engine_version.as_deref();
                }
            }
            _ => {}
        }
        agg.first = Some(agg.first.map_or(e.ts_ms, |t| t.min(e.ts_ms)));
        agg.last = Some(agg.last.map_or(e.ts_ms, |t| t.max(e.ts_ms)));
    }
    let mut stmt = tx.prepare_cached(
        "INSERT INTO sessions(id, cwd, entrypoint, engine_version, started_at, last_seen_at,
                              file_created_at, source_mask)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
             cwd = COALESCE(excluded.cwd, cwd),
             entrypoint = COALESCE(excluded.entrypoint, entrypoint),
             engine_version = COALESCE(excluded.engine_version, engine_version),
             started_at = MIN(started_at, excluded.started_at),
             last_seen_at = MAX(last_seen_at, excluded.last_seen_at),
             file_created_at = CASE
                 WHEN excluded.file_created_at IS NULL THEN file_created_at
                 WHEN file_created_at IS NULL THEN excluded.file_created_at
                 ELSE MIN(file_created_at, excluded.file_created_at) END,
             source_mask = source_mask | excluded.source_mask",
    )?;
    for (id, agg) in sessions {
        // Un lote con solo `cost-state` no basta para crear la sesión.
        let Some(first) = agg.first else {
            continue;
        };
        let last = agg.last.unwrap_or(first);
        stmt.execute(params![
            id,
            agg.cwd,
            agg.entrypoint,
            agg.version,
            first,
            last,
            agg.file_created_at,
            agg.mask
        ])?;
    }
    Ok(())
}

/// Orden de antigüedad de una sesión: la menor es la canónica (R-4).
///
/// Una sesión reanudada o bifurcada copia el historial con sus marcas de tiempo
/// originales, así que `started_at` empata con el de la sesión de origen. La fecha
/// de creación del fichero deshace el empate: el original se creó antes.
fn session_rank(tx: &Transaction<'_>, id: &str) -> Result<(i64, i64)> {
    Ok(tx
        .prepare_cached(
            "SELECT started_at, COALESCE(file_created_at, 9223372036854775807)
             FROM sessions WHERE id = ?1",
        )?
        .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))?)
}

fn apply_event(
    tx: &Transaction<'_>,
    e: &CockpitEvent,
    stats: &mut BatchStats,
    touched: &mut tasks::Touched,
) -> Result<()> {
    match &e.kind {
        EventKind::TaskOp { source, op_key, op } => {
            tasks::record_op(tx, e, *source, op_key, op, touched)?;
        }
        EventKind::ToolResultMeta { .. } => tasks::apply_result_meta(tx, e, touched)?,
        EventKind::AgentStatus {
            agent,
            agent_type,
            status,
            duration_ms,
        } => tasks::upsert_agent(
            tx,
            &tasks::AgentUpdate {
                session_id: &e.session_id,
                agent,
                agent_type: agent_type.as_deref(),
                status: *status,
                parent_id: e.agent_id.as_deref(),
                ts: e.ts_ms,
                duration_ms: *duration_ms,
                mask: e.source.mask(),
            },
        )?,
        EventKind::SessionSeen { .. } | EventKind::TranscriptFile { .. } => {}
        EventKind::ApiMessage {
            message_id,
            model,
            usage,
            stop_reason,
        } => {
            let existing: Option<String> = tx
                .prepare_cached("SELECT session_id FROM api_messages WHERE message_id = ?1")?
                .query_row([message_id], |r| r.get(0))
                .optional()?;
            match existing {
                None => {
                    tx.prepare_cached(
                        "INSERT INTO api_messages(message_id, session_id, agent_id, model, ts, day,
                             input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                             stop_reason, engine_version)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    )?
                    .execute(params![
                        message_id,
                        e.session_id,
                        e.agent_id,
                        model,
                        e.ts_ms,
                        local_day(e.ts_ms),
                        usage.input as i64,
                        usage.output as i64,
                        usage.cache_read as i64,
                        usage.cache_creation as i64,
                        stop_reason,
                        e.engine_version
                    ])?;
                }
                Some(current) => {
                    // La misma respuesta sale en varias filas (una por bloque). El uso
                    // final es el de la fila con más tokens de salida.
                    tx.prepare_cached(
                        "UPDATE api_messages SET
                             input_tokens = CASE WHEN ?2 >= output_tokens THEN ?1 ELSE input_tokens END,
                             cache_read_tokens = CASE WHEN ?2 >= output_tokens THEN ?3 ELSE cache_read_tokens END,
                             cache_creation_tokens = CASE WHEN ?2 >= output_tokens THEN ?4 ELSE cache_creation_tokens END,
                             output_tokens = MAX(output_tokens, ?2),
                             stop_reason = COALESCE(?5, stop_reason)
                         WHERE message_id = ?6",
                    )?
                    .execute(params![
                        usage.input as i64,
                        usage.output as i64,
                        usage.cache_read as i64,
                        usage.cache_creation as i64,
                        stop_reason,
                        message_id
                    ])?;
                    if current != e.session_id
                        && (session_rank(tx, &e.session_id)?, e.session_id.as_str())
                            < (session_rank(tx, &current)?, current.as_str())
                    {
                        tx.prepare_cached(
                            "UPDATE api_messages SET session_id = ?1, agent_id = ?2 WHERE message_id = ?3",
                        )?
                        .execute(params![e.session_id, e.agent_id, message_id])?;
                    }
                }
            }
        }
        EventKind::ToolStarted {
            tool_use_id,
            tool,
            meta,
        } => {
            tx.prepare_cached(
                "INSERT INTO tool_calls(session_id, tool_use_id, agent_id, tool, status, started_at,
                                        day, rel_path, host, meta_json, source_mask, root_id)
                 VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6, ?7, ?8, ?9, ?10,
                         (SELECT root_id FROM sessions WHERE id = ?1))
                 ON CONFLICT(session_id, tool_use_id) DO UPDATE SET
                     root_id = COALESCE(root_id, excluded.root_id),
                     -- El mod mide el inicio real; el transcript solo lo aproxima.
                     started_at = CASE WHEN excluded.source_mask = 1 THEN excluded.started_at ELSE started_at END,
                     day = CASE WHEN excluded.source_mask = 1 THEN excluded.day ELSE day END,
                     duration_ms = CASE WHEN excluded.source_mask = 1 AND finished_at IS NOT NULL
                                        THEN MAX(0, finished_at - excluded.started_at) ELSE duration_ms END,
                     agent_id = COALESCE(agent_id, excluded.agent_id),
                     rel_path = COALESCE(rel_path, excluded.rel_path),
                     host = COALESCE(host, excluded.host),
                     meta_json = COALESCE(meta_json, excluded.meta_json),
                     source_mask = source_mask | excluded.source_mask",
            )?
            .execute(params![
                e.session_id,
                tool_use_id,
                e.agent_id,
                tool,
                e.ts_ms,
                local_day(e.ts_ms),
                meta.rel_path,
                meta.host,
                meta.meta_json(),
                e.source.mask()
            ])?;
            mark_copies(tx, tool_use_id)?;
        }
        EventKind::ToolFinished {
            tool_use_id,
            status,
        } => {
            let changed = tx
                .prepare_cached(
                    "UPDATE tool_calls SET status = ?1, finished_at = ?2,
                            duration_ms = MAX(0, ?2 - started_at)
                     WHERE session_id = ?3 AND tool_use_id = ?4
                       AND (status IN ('running', 'awaiting_permission', 'unknown') OR ?5 = 1)",
                )?
                // El fin que da el mod es exacto y sustituye al del transcript.
                .execute(params![
                    status.as_str(),
                    e.ts_ms,
                    e.session_id,
                    tool_use_id,
                    e.source.mask()
                ])?;
            if changed == 0 {
                let exists: bool = tx
                    .prepare_cached(
                        "SELECT EXISTS(SELECT 1 FROM tool_calls WHERE session_id = ?1 AND tool_use_id = ?2)",
                    )?
                    .query_row(params![e.session_id, tool_use_id], |r| r.get(0))?;
                if !exists {
                    stats.orphan_results += 1;
                }
            }
            if matches!(status, ToolStatus::Error | ToolStatus::Interrupted) {
                tasks::drop_failed_op(tx, &e.session_id, tool_use_id, touched)?;
            }
        }
        EventKind::ToolPermission {
            tool_use_id,
            is_waiting,
        } => {
            // Solo se mueve entre los dos estados «en curso»: nunca reabre una
            // herramienta que ya terminó.
            let (from, to) = if *is_waiting {
                ("running", "awaiting_permission")
            } else {
                ("awaiting_permission", "running")
            };
            tx.prepare_cached(
                "UPDATE tool_calls SET status = ?1
                 WHERE session_id = ?2 AND tool_use_id = ?3 AND status = ?4",
            )?
            .execute(params![to, e.session_id, tool_use_id, from])?;
        }
        EventKind::TurnCompleted {
            turn_id,
            duration_ms,
            is_aborted,
            model,
            usage,
        } => {
            tx.prepare_cached(
                "INSERT OR REPLACE INTO turns(session_id, turn_id, agent_id, completed_at, duration_ms,
                     is_aborted, model, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            )?
            .execute(params![
                e.session_id,
                turn_id,
                e.agent_id,
                e.ts_ms,
                *duration_ms as i64,
                is_aborted,
                model,
                usage.map(|u| u.input as i64),
                usage.map(|u| u.output as i64),
                usage.map(|u| u.cache_read as i64),
                usage.map(|u| u.cache_creation as i64)
            ])?;
        }
        EventKind::UsageSnapshot {
            context_tokens,
            context_window,
            context_percent,
            rate_limits,
            cost_usd,
        } => {
            let limits = serde_json::to_string(rate_limits).unwrap_or_else(|_| "[]".to_owned());
            // Solo se guarda si cambia respecto al último de la sesión.
            let same: bool = tx
                .prepare_cached(
                    "SELECT COALESCE((SELECT context_tokens IS ?2 AND context_window IS ?3
                                AND context_percent IS ?4 AND rate_limits_json = ?5 AND cost_usd IS ?6
                         FROM usage_snapshots WHERE session_id = ?1 ORDER BY id DESC LIMIT 1), 0)",
                )?
                .query_row(
                    params![
                        e.session_id,
                        context_tokens.map(|v| v as i64),
                        context_window.map(|v| v as i64),
                        context_percent,
                        limits,
                        cost_usd
                    ],
                    |r| r.get(0),
                )?;
            if !same {
                tx.prepare_cached(
                    "INSERT INTO usage_snapshots(session_id, ts, context_tokens, context_window,
                         context_percent, rate_limits_json, cost_usd)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )?
                .execute(params![
                    e.session_id,
                    e.ts_ms,
                    context_tokens.map(|v| v as i64),
                    context_window.map(|v| v as i64),
                    context_percent,
                    limits,
                    cost_usd
                ])?;
            }
        }
        EventKind::SessionEnded { .. } => {
            tx.prepare_cached("UPDATE sessions SET ended_at = ?1 WHERE id = ?2")?
                .execute(params![e.ts_ms, e.session_id])?;
            // Lo que seguía en curso queda sin resultado conocido (SPEC §5.2).
            tx.prepare_cached(
                "UPDATE tool_calls SET status = 'unknown'
                 WHERE session_id = ?1 AND status IN ('running', 'awaiting_permission')",
            )?
            .execute(params![e.session_id])?;
        }
        EventKind::ApiError { status, error_type } => {
            tx.prepare_cached(
                "INSERT OR IGNORE INTO api_errors(session_id, agent_id, ts, status, error_type)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![
                e.session_id,
                e.agent_id,
                e.ts_ms,
                status,
                error_type
            ])?;
        }
        EventKind::CostState {
            total_cost_usd,
            usage,
        } => {
            tx.prepare_cached(
                "UPDATE sessions SET cost_state_usd = ?1, cost_state_input = ?2, cost_state_output = ?3,
                        cost_state_cache_read = ?4, cost_state_cache_creation = ?5
                 WHERE id = ?6",
            )?
            .execute(params![
                total_cost_usd,
                usage.input as i64,
                usage.output as i64,
                usage.cache_read as i64,
                usage.cache_creation as i64,
                e.session_id
            ])?;
        }
    }
    Ok(())
}

/// Si la herramienta aparece en varias sesiones, deja como canónica la de la sesión
/// más antigua y marca el resto como copias de historial (R-4).
fn mark_copies(tx: &Transaction<'_>, tool_use_id: &str) -> Result<()> {
    let occurrences: i64 = tx
        .prepare_cached("SELECT COUNT(*) FROM tool_calls WHERE tool_use_id = ?1")?
        .query_row([tool_use_id], |r| r.get(0))?;
    if occurrences < 2 {
        return Ok(());
    }
    tx.prepare_cached(
        "UPDATE tool_calls SET is_copy = (session_id <> (
             SELECT tc.session_id FROM tool_calls tc JOIN sessions s ON s.id = tc.session_id
             WHERE tc.tool_use_id = ?1
             ORDER BY s.started_at, COALESCE(s.file_created_at, 9223372036854775807), s.id
             LIMIT 1))
         WHERE tool_use_id = ?1",
    )?
    .execute([tool_use_id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cockpit_core::model::{Source, ToolStatus};
    use cockpit_core::privacy::ToolMeta;

    const T0: i64 = 1_762_770_000_000;

    fn ev(session: &str, ts: i64, kind: EventKind) -> CockpitEvent {
        CockpitEvent {
            source: Source::Transcript,
            session_id: session.to_owned(),
            agent_id: None,
            ts_ms: ts,
            engine_version: Some("2.1.321".to_owned()),
            kind,
        }
    }

    fn file(session: &str, created: i64) -> CockpitEvent {
        ev(
            session,
            created,
            EventKind::TranscriptFile {
                created_at_ms: created,
            },
        )
    }

    fn msg(session: &str, ts: i64, id: &str, output: u64) -> CockpitEvent {
        ev(
            session,
            ts,
            EventKind::ApiMessage {
                message_id: id.to_owned(),
                model: "claude-test".to_owned(),
                usage: TokenUsage {
                    input: 10,
                    output,
                    cache_read: 100,
                    cache_creation: 5,
                },
                stop_reason: None,
            },
        )
    }

    fn tool(session: &str, ts: i64, id: &str) -> CockpitEvent {
        ev(
            session,
            ts,
            EventKind::ToolStarted {
                tool_use_id: id.to_owned(),
                tool: "Edit".to_owned(),
                meta: ToolMeta {
                    rel_path: Some("src/main.rs".to_owned()),
                    ..Default::default()
                },
            },
        )
    }

    fn done(session: &str, ts: i64, id: &str, status: ToolStatus) -> CockpitEvent {
        ev(
            session,
            ts,
            EventKind::ToolFinished {
                tool_use_id: id.to_owned(),
                status,
            },
        )
    }

    fn apply(store: &mut Store, events: &[CockpitEvent]) -> BatchStats {
        store
            .apply(&Batch {
                events,
                now_ms: T0,
                ..Default::default()
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

    #[test]
    fn schema_is_at_the_latest_version() {
        let store = Store::open_in_memory().unwrap();
        let v: u32 = store
            .connection()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    /// M7: la misma respuesta en varias filas cuenta una sola vez.
    #[test]
    fn duplicated_message_rows_count_once_with_the_final_usage() {
        let mut store = Store::open_in_memory().unwrap();
        apply(
            &mut store,
            &[
                msg("s1", T0, "m1", 3),
                msg("s1", T0 + 1, "m1", 250),
                msg("s1", T0 + 2, "m1", 250),
                msg("s1", T0 + 3, "m2", 40),
            ],
        );
        let t = store.totals().unwrap();
        assert_eq!(t.api_messages, 2);
        assert_eq!(t.usage.output, 290);
        assert_eq!(t.usage.input, 20);
        assert_eq!(t.usage.cache_read, 200);
    }

    #[test]
    fn applying_the_same_events_twice_changes_nothing() {
        let events = [
            file("s1", T0),
            msg("s1", T0 + 10, "m1", 50),
            tool("s1", T0 + 20, "t1"),
            done("s1", T0 + 900, "t1", ToolStatus::Ok),
            ev(
                "s1",
                T0 + 950,
                EventKind::ApiError {
                    status: Some(429),
                    error_type: Some("rate_limit".to_owned()),
                },
            ),
        ];
        let mut store = Store::open_in_memory().unwrap();
        apply(&mut store, &events);
        let tables = [
            "SELECT * FROM sessions",
            "SELECT * FROM api_messages",
            "SELECT * FROM tool_calls",
            "SELECT session_id, ts, status, error_type FROM api_errors",
        ];
        let before: Vec<_> = tables.iter().map(|q| dump(&store, q)).collect();
        apply(&mut store, &events);
        let after: Vec<_> = tables.iter().map(|q| dump(&store, q)).collect();
        assert_eq!(before, after);
        assert_eq!(store.totals().unwrap().api_errors, 1);
    }

    #[test]
    fn tool_lifecycle_records_status_and_duration() {
        let mut store = Store::open_in_memory().unwrap();
        let stats = apply(
            &mut store,
            &[
                tool("s1", T0, "t1"),
                tool("s1", T0, "t2"),
                done("s1", T0 + 1500, "t1", ToolStatus::Error),
                done("s1", T0 + 99, "huérfana", ToolStatus::Ok),
            ],
        );
        assert_eq!(stats.orphan_results, 1);
        let mut stmt = store
            .connection()
            .prepare("SELECT tool_use_id, status, duration_ms, rel_path FROM tool_calls ORDER BY 1")
            .unwrap();
        let rows: Vec<(String, String, Option<i64>, Option<String>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        let path = Some("src/main.rs".to_owned());
        assert_eq!(
            rows,
            [
                (
                    "t1".to_owned(),
                    "error".to_owned(),
                    Some(1500),
                    path.clone()
                ),
                ("t2".to_owned(), "running".to_owned(), None, path),
            ]
        );
    }

    fn copies(store: &Store) -> Vec<(String, i64)> {
        let mut stmt = store
            .connection()
            .prepare("SELECT session_id, is_copy FROM tool_calls ORDER BY session_id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    /// R-4: la sesión reanudada copia el historial con las marcas originales.
    /// La canónica es la original, se inserte primero la que se inserte.
    #[test]
    fn history_copies_are_flagged_whatever_the_insertion_order() {
        // El fichero original se crea justo después de su primera línea: las dos
        // sesiones empatan en `started_at` y decide la fecha de creación.
        let original = [file("orig", T0 + 20), tool("orig", T0 + 10, "t1")];
        let resumed = [
            file("zcopy", T0 + 3_600_000),
            tool("zcopy", T0 + 10, "t1"),
            tool("zcopy", T0 + 3_600_500, "t2"),
        ];
        let expected = vec![
            ("orig".to_owned(), 0),
            ("zcopy".to_owned(), 1),
            ("zcopy".to_owned(), 0),
        ];

        let mut a = Store::open_in_memory().unwrap();
        apply(&mut a, &original);
        apply(&mut a, &resumed);
        assert_eq!(copies(&a), expected);

        // El backfill va de lo más reciente a lo más antiguo: primero llega la copia.
        let mut b = Store::open_in_memory().unwrap();
        apply(&mut b, &resumed);
        assert_eq!(
            copies(&b),
            vec![("zcopy".to_owned(), 0), ("zcopy".to_owned(), 0)]
        );
        apply(&mut b, &original);
        assert_eq!(copies(&b), expected);

        assert_eq!(a.totals().unwrap().tool_calls, 2);
        assert_eq!(a.totals().unwrap().tool_call_copies, 1);
    }

    /// R-4: un mismo `message_id` en dos sesiones se cuenta una vez, en la más antigua.
    #[test]
    fn a_message_copied_to_another_session_belongs_to_the_oldest_one() {
        let original = [file("orig", T0 + 20), msg("orig", T0 + 10, "m1", 70)];
        let resumed = [
            file("zcopy", T0 + 3_600_000),
            msg("zcopy", T0 + 10, "m1", 70),
        ];
        for order in [[&original, &resumed], [&resumed, &original]] {
            let mut store = Store::open_in_memory().unwrap();
            for events in order {
                apply(&mut store, events.as_slice());
            }
            let owner: String = store
                .connection()
                .query_row("SELECT session_id FROM api_messages", [], |r| r.get(0))
                .unwrap();
            assert_eq!(owner, "orig");
            assert_eq!(store.totals().unwrap().usage.output, 70);
        }
    }

    #[test]
    fn cost_state_check_compares_deduplicated_tokens() {
        let mut store = Store::open_in_memory().unwrap();
        apply(
            &mut store,
            &[
                msg("s1", T0, "m1", 30),
                msg("s1", T0 + 1, "m1", 30),
                msg("s1", T0 + 2, "m2", 12),
                ev(
                    "s1",
                    T0 + 2,
                    EventKind::CostState {
                        total_cost_usd: 0.25,
                        usage: TokenUsage {
                            input: 20,
                            output: 42,
                            cache_read: 200,
                            cache_creation: 10,
                        },
                    },
                ),
            ],
        );
        let checks = store.cost_state_checks().unwrap();
        assert_eq!(checks.len(), 1);
        assert!(checks[0].matches(), "{:?}", checks[0]);
    }

    #[test]
    fn offsets_and_errors_are_written_with_the_batch() {
        let mut store = Store::open_in_memory().unwrap();
        let offset = FileOffset {
            path: "a.jsonl".to_owned(),
            size: 100,
            offset: 80,
            mtime_ms: T0,
            parser_version: 2,
        };
        let error = IngestError {
            path: String::new(),
            kind: "unknown_type",
            detail: "tipo-nuevo".to_owned(),
            count: 2,
        };
        for _ in 0..2 {
            store
                .apply(&Batch {
                    offsets: std::slice::from_ref(&offset),
                    errors: std::slice::from_ref(&error),
                    now_ms: T0,
                    ..Default::default()
                })
                .unwrap();
        }
        assert_eq!(store.file_offset("a.jsonl").unwrap(), Some(offset));
        assert_eq!(store.file_offset("b.jsonl").unwrap(), None);
        let count: i64 = store
            .connection()
            .query_row("SELECT count FROM ingest_errors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 4);
    }
}
