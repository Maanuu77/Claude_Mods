//! Esquema y migraciones (SPEC §6.4). La versión vive en `PRAGMA user_version`.
//!
//! La migración 1 crea las tablas que alimenta el transcript. Las tablas de las
//! demás fuentes (`agents`, `turns`, `usage_snapshots`, `tasks`, `file_events`,
//! agregados y `aggregation_ledger`) llegan con sus fases en migraciones nuevas.

use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[
    // 1: F1, ingesta de transcripts.
    r#"
    CREATE TABLE schema_meta(
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    CREATE TABLE projects(
        id          INTEGER PRIMARY KEY,
        root        TEXT NOT NULL UNIQUE,
        git_dir     TEXT NULL,
        is_worktree INTEGER NOT NULL DEFAULT 0,
        main_root   TEXT NULL,
        last_active INTEGER NULL
    );

    CREATE TABLE sessions(
        id              TEXT PRIMARY KEY,
        root_id         INTEGER NULL REFERENCES projects(id),
        cwd             TEXT NULL,                 -- solo con privacy.record_paths
        entrypoint      TEXT NULL,
        engine_version  TEXT NULL,
        started_at      INTEGER NOT NULL,          -- ms
        last_seen_at    INTEGER NOT NULL,
        file_created_at INTEGER NULL,              -- creación del JSONL: desempate de R-4
        source_mask     INTEGER NOT NULL,
        -- última línea `cost-state` del transcript (totales del propio cliente)
        cost_state_usd            REAL NULL,
        cost_state_input          INTEGER NULL,
        cost_state_output         INTEGER NULL,
        cost_state_cache_read     INTEGER NULL,
        cost_state_cache_creation INTEGER NULL
    );

    CREATE TABLE api_messages(
        message_id            TEXT PRIMARY KEY,    -- deduplicación de tokens (M7)
        session_id            TEXT NOT NULL REFERENCES sessions(id),  -- sesión canónica (R-4)
        agent_id              TEXT NULL,
        model                 TEXT NOT NULL,
        ts                    INTEGER NOT NULL,
        day                   TEXT NOT NULL,
        input_tokens          INTEGER NOT NULL,
        output_tokens         INTEGER NOT NULL,
        cache_read_tokens     INTEGER NOT NULL,
        cache_creation_tokens INTEGER NOT NULL,
        stop_reason           TEXT NULL,
        engine_version        TEXT NULL
    );
    CREATE INDEX api_messages_session ON api_messages(session_id, ts);
    CREATE INDEX api_messages_day ON api_messages(day);

    CREATE TABLE tool_calls(
        session_id   TEXT NOT NULL REFERENCES sessions(id),
        tool_use_id  TEXT NOT NULL,                -- no es único entre sesiones (M8)
        is_copy      INTEGER NOT NULL DEFAULT 0,   -- 1 = copia de historial de otra sesión (R-4)
        agent_id     TEXT NULL,
        root_id      INTEGER NULL REFERENCES projects(id),
        tool         TEXT NOT NULL,
        status       TEXT NOT NULL,                -- running|ok|error|interrupted|unknown
        started_at   INTEGER NOT NULL,
        finished_at  INTEGER NULL,
        duration_ms  INTEGER NULL,
        day          TEXT NOT NULL,                -- fecha civil local de started_at
        rel_path     TEXT NULL,                    -- retención de 30 días (§6.4.1)
        host         TEXT NULL,                    -- retención de 30 días (§6.4.1)
        meta_json    TEXT NULL,                    -- lista blanca de §10, nunca rutas ni host
        source_mask  INTEGER NOT NULL,
        PRIMARY KEY (session_id, tool_use_id)
    );
    CREATE INDEX tool_calls_tool_use ON tool_calls(tool_use_id);
    CREATE INDEX tool_calls_session ON tool_calls(session_id, started_at);
    CREATE INDEX tool_calls_day ON tool_calls(day);
    CREATE INDEX tool_calls_paths ON tool_calls(started_at)
        WHERE rel_path IS NOT NULL OR host IS NOT NULL;

    CREATE TABLE api_errors(
        id         INTEGER PRIMARY KEY,
        session_id TEXT NOT NULL REFERENCES sessions(id),
        agent_id   TEXT NULL,
        ts         INTEGER NOT NULL,
        status     INTEGER NULL,                   -- estado HTTP
        error_type TEXT NULL                       -- código corto, nunca el mensaje
    );
    CREATE UNIQUE INDEX api_errors_identity ON api_errors(
        session_id, ts, COALESCE(status, 0), COALESCE(error_type, ''), COALESCE(agent_id, '')
    );

    CREATE TABLE file_offsets(
        path       TEXT PRIMARY KEY,
        size       INTEGER NOT NULL,
        offset     INTEGER NOT NULL,
        mtime_ms   INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );

    CREATE TABLE ingest_errors(
        id     INTEGER PRIMARY KEY,
        ts     INTEGER NOT NULL,
        path   TEXT NOT NULL,                      -- retención de 30 días (§6.4.1)
        kind   TEXT NOT NULL,                      -- corrupt|drift|unknown_type|truncated|orphan_result
        detail TEXT NOT NULL,                      -- nombre de tipo; nunca contenido
        count  INTEGER NOT NULL,
        UNIQUE (path, kind, detail)
    );

    CREATE TABLE pricing(
        model_prefix         TEXT NOT NULL,
        valid_from           TEXT NOT NULL,
        input_per_mtok       REAL NOT NULL,
        output_per_mtok      REAL NOT NULL,
        cache_read_per_mtok  REAL NOT NULL,
        cache_write_per_mtok REAL NOT NULL,
        PRIMARY KEY (model_prefix, valid_from)
    );
    "#,
    // 2: F3, eventos del mod.
    r#"
    CREATE TABLE mod_instances(
        instance_id    TEXT PRIMARY KEY,           -- uuid por carga del módulo
        session_id     TEXT NOT NULL,
        hello_seq      INTEGER NOT NULL,
        ack_seq        INTEGER NOT NULL DEFAULT 0, -- marca de agua de /v1/events
        engine_version TEXT NULL,
        mod_version    TEXT NULL,
        first_hello_at INTEGER NOT NULL,
        last_hello_at  INTEGER NOT NULL
    );

    CREATE TABLE turns(
        session_id            TEXT NOT NULL REFERENCES sessions(id),
        turn_id               TEXT NOT NULL,
        agent_id              TEXT NULL,
        completed_at          INTEGER NOT NULL,
        duration_ms           INTEGER NOT NULL,
        is_aborted            INTEGER NOT NULL,
        model                 TEXT NULL,
        input_tokens          INTEGER NULL,
        output_tokens         INTEGER NULL,
        cache_read_tokens     INTEGER NULL,
        cache_creation_tokens INTEGER NULL,
        PRIMARY KEY (session_id, turn_id)
    );

    CREATE TABLE usage_snapshots(
        id               INTEGER PRIMARY KEY,
        session_id       TEXT NOT NULL REFERENCES sessions(id),
        ts               INTEGER NOT NULL,
        context_tokens   INTEGER NULL,
        context_window   INTEGER NULL,
        context_percent  REAL NULL,
        rate_limits_json TEXT NOT NULL,
        cost_usd         REAL NULL
    );
    CREATE INDEX usage_snapshots_session ON usage_snapshots(session_id, ts);

    ALTER TABLE sessions ADD COLUMN ended_at INTEGER NULL;
    "#,
];

/// Versión de esquema que produce este binario.
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

pub(crate) fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as u32)?;
        tx.commit()?;
    }
    Ok(())
}
