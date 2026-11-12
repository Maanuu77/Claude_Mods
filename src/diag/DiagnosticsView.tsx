import { invoke } from "@tauri-apps/api/core";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import { formatAgo, formatInt } from "../live/format";
import { sortSessions, useLive } from "../live/store";

/** Espejo de `DiagnosticsView` en `src-tauri` (SPEC §7, vista 5, y §6.4.2). */
interface Diagnostics {
  schema_version: number;
  sessions: number;
  api_messages: number;
  tool_calls: number;
  tool_call_copies: number;
  api_errors: number;
  transcript_files: number;
  ingest_errors: { kind: string; detail: string; count: number; files: number; last_ts: number }[];
  engines: { engine_version: string | null; mod_version: string | null; sessions: number; last_hello_at: number }[];
  aggregation: {
    target: "FileChangeDaily" | "ToolUsageDaily";
    pending_days: number;
    dirty_days: number;
    mismatched_days: number;
  }[];
  path_detail: { tool_calls_with_path: number; file_events: number; oldest_day: string | null };
  db_bytes: number;
  wal_bytes: number;
}

const TARGET_LABEL = {
  FileChangeDaily: "Cambios de fichero por día",
  ToolUsageDaily: "Uso de herramientas por día",
} as const;

const ERROR_LABEL: Record<string, string> = {
  corrupt: "línea corrupta",
  drift: "cambio de esquema",
  unknown_type: "tipo de línea desconocido",
  truncated: "fichero truncado",
  orphan_result: "resultado sin su herramienta",
};

const WATCH_LABEL = {
  active: "vigilada",
  network: "unidad de red (sin vigilar)",
  too_broad: "demasiado amplia (sin vigilar)",
  error: "error al vigilar",
} as const;

function formatBytes(n: number): string {
  if (n < 1024 * 1024) return `${formatInt(Math.round(n / 1024))} KiB`;
  return `${(n / 1024 / 1024).toLocaleString("es-ES", { maximumFractionDigits: 1 })} MiB`;
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="dg-row">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

/**
 * Vista «Diagnóstico» (SPEC §7, vista 5): fuentes activas, mod por sesión, motores,
 * ingesta, agregación diaria (§6.4.2) y retención de rutas (§6.4.1). Solo lectura.
 */
export function DiagnosticsView({ now }: { now: number }) {
  const { connected, error, sessions, backfill, workspaces } = useLive();
  const [diag, setDiag] = useState<Diagnostics | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [round, setRound] = useState(0);

  useEffect(() => {
    let alive = true;
    invoke<Diagnostics>("diagnostics")
      .then((d) => {
        if (!alive) return;
        setDiag(d);
        setFailure(null);
      })
      .catch((e: unknown) => alive && setFailure(String(e)));
    return () => {
      alive = false;
    };
  }, [round]);

  const recent = useMemo(() => sortSessions(sessions), [sessions]);
  const live = recent.filter((s) => s.live !== null);
  const withMod = live.filter((s) => s.mod_connected);
  const roots = Object.values(workspaces);

  return (
    <div className="dg">
      <header className="dg-head">
        <button type="button" className="tab" onClick={() => setRound((r) => r + 1)}>
          Actualizar
        </button>
      </header>
      {failure && (
        <p className="banner banner-error" role="alert">
          No se ha podido leer el diagnóstico: {failure}
        </p>
      )}

      <div className="grid">
        <section className="card">
          <h3>Fuentes</h3>
          <dl className="dg-list">
            <Row label="Núcleo">{connected ? "conectado" : error ? `error: ${error}` : "conectando…"}</Row>
            <Row label="Transcripts">
              {backfill?.running
                ? `leyendo el historial (${formatInt(backfill.files_done)} de ${formatInt(backfill.files_total)})`
                : `al día${diag ? ` · ${formatInt(diag.transcript_files)} ficheros` : ""}`}
            </Row>
            <Row label="Sesiones vivas">
              {formatInt(live.length)} · {formatInt(withMod.length)} con el mod conectado
            </Row>
            <Row label="Proyectos vigilados">
              {roots.length === 0
                ? "ninguno"
                : roots.map((w) => `${w.name} (${WATCH_LABEL[w.watch]})`).join(", ")}
            </Row>
          </dl>
          {live.length > 0 && (
            <ul className="dg-sessions">
              {live.map((s) => (
                <li key={s.id}>
                  <span className="dg-mono">{s.id.slice(0, 8)}</span>
                  <span>{s.engine_version ? `motor ${s.engine_version}` : "motor desconocido"}</span>
                  <span className={s.mod_connected ? "tag tag-wait" : "tag"}>
                    {s.mod_connected ? "mod conectado" : "sin mod"}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>

        {diag && (
          <section className="card">
            <h3>Base de datos</h3>
            <dl className="dg-list">
              <Row label="Esquema">versión {diag.schema_version}</Row>
              <Row label="Tamaño">
                {formatBytes(diag.db_bytes)} + {formatBytes(diag.wal_bytes)} de WAL
              </Row>
              <Row label="Sesiones">{formatInt(diag.sessions)}</Row>
              <Row label="Mensajes de la API">{formatInt(diag.api_messages)}</Row>
              <Row label="Herramientas">
                {formatInt(diag.tool_calls)} ({formatInt(diag.tool_call_copies)} copias de historial)
              </Row>
              <Row label="Errores de la API">{formatInt(diag.api_errors)}</Row>
            </dl>
          </section>
        )}

        {diag && (
          <section className="card">
            <h3>Agregación diaria y retención</h3>
            <dl className="dg-list">
              {diag.aggregation.map((a) => (
                <Row key={a.target} label={TARGET_LABEL[a.target]}>
                  {a.pending_days === 0 && a.mismatched_days === 0 ? (
                    "al día"
                  ) : (
                    <>
                      {a.pending_days > 0 && (
                        <span className="tag tag-wait">{formatInt(a.pending_days)} días por agregar</span>
                      )}{" "}
                      {a.mismatched_days > 0 && (
                        <span className="tag tag-error">{formatInt(a.mismatched_days)} días no cuadran</span>
                      )}
                    </>
                  )}
                </Row>
              ))}
              <Row label="Detalle de rutas">
                {formatInt(diag.path_detail.tool_calls_with_path)} herramientas y{" "}
                {formatInt(diag.path_detail.file_events)} cambios de fichero
                {diag.path_detail.oldest_day && ` · desde ${diag.path_detail.oldest_day}`}
              </Row>
            </dl>
            <p className="card-meta">
              Las rutas se guardan 30 días; después solo quedan los totales por día (§6.4.1).
            </p>
          </section>
        )}

        {diag && (
          <section className="card">
            <h3>Motores con el mod</h3>
            {diag.engines.length === 0 ? (
              <p className="card-meta">Ningún mod se ha conectado todavía.</p>
            ) : (
              <dl className="dg-list">
                {diag.engines.map((e) => (
                  <Row
                    key={`${e.engine_version}-${e.mod_version}`}
                    label={`Claude Code ${e.engine_version ?? "?"}`}
                  >
                    mod {e.mod_version ?? "?"} · {formatInt(e.sessions)} sesiones · último {formatAgo(e.last_hello_at, now)}
                  </Row>
                ))}
              </dl>
            )}
          </section>
        )}

        {diag && (
          <section className="card dg-wide">
            <h3>Incidencias de la ingesta</h3>
            {diag.ingest_errors.length === 0 ? (
              <p className="card-meta">Ninguna.</p>
            ) : (
              <table className="dg-table">
                <thead>
                  <tr>
                    <th>Tipo</th>
                    <th>Detalle</th>
                    <th>Veces</th>
                    <th>Ficheros</th>
                    <th>Última</th>
                  </tr>
                </thead>
                <tbody>
                  {diag.ingest_errors.map((e) => (
                    <tr key={`${e.kind}-${e.detail}`}>
                      <td>{ERROR_LABEL[e.kind] ?? e.kind}</td>
                      <td className="dg-mono">{e.detail || "—"}</td>
                      <td>{formatInt(e.count)}</td>
                      <td>{formatInt(e.files)}</td>
                      <td>{formatAgo(e.last_ts, now)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </section>
        )}
      </div>
    </div>
  );
}
