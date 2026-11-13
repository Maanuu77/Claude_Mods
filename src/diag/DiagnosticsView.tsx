import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";
import { View } from "../app/Shell";
import { formatAgo, formatInt } from "../live/format";
import { sortSessions, useLive } from "../live/store";
import { Badge, Button, Card, EmptyState, Notice, Property, PropertyList, Skeleton } from "../ui/components";
import type { Tone } from "../ui/tone";
import { PrivacySettings } from "./PrivacySettings";
import "./diag.css";

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

const WATCH: Record<string, { label: string; tone: Tone }> = {
  active: { label: "vigilado", tone: "positive" },
  network: { label: "unidad de red, sin vigilar", tone: "attention" },
  too_broad: { label: "demasiado amplio, sin vigilar", tone: "attention" },
  error: { label: "error al vigilar", tone: "critical" },
};

/** Tarjeta sin datos porque el diagnóstico no se ha podido leer (el aviso de arriba lo explica). */
function Unavailable() {
  return <p className="dg-note">Sin datos: no se ha podido leer el diagnóstico.</p>;
}

function formatBytes(n: number): string {
  if (n < 1024 * 1024) return `${formatInt(Math.round(n / 1024))} KiB`;
  return `${(n / 1024 / 1024).toLocaleString("es-ES", { maximumFractionDigits: 1 })} MiB`;
}

/**
 * Vista «Diagnóstico» (SPEC §7, vista 5): fuentes activas, mod por sesión, motores,
 * ingesta, agregación diaria (§6.4.2), retención de rutas (§6.4.1) y ajustes de
 * privacidad (§10).
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
  const refresh = () => setRound((r) => r + 1);

  return (
    <View
      title="Diagnóstico"
      summary="Salud de las fuentes, la base de datos y la agregación; privacidad y retención."
      actions={
        <Button icon="refresh" onClick={refresh}>
          Actualizar
        </Button>
      }
    >
      {failure && (
        <Notice
          tone="critical"
          title="No se ha podido leer el diagnóstico"
          actions={
            <Button size="sm" onClick={refresh}>
              Reintentar
            </Button>
          }
        >
          {failure}
        </Notice>
      )}

      <div className="grid-cards dg-grid">
        <Card title="Fuentes">
          <PropertyList>
            <Property label="Núcleo">
              {connected ? (
                <Badge tone="positive" dot>
                  conectado
                </Badge>
              ) : error ? (
                <Badge tone="critical" dot>
                  sin conexión
                </Badge>
              ) : (
                <Badge tone="neutral" dot pulse>
                  conectando
                </Badge>
              )}
            </Property>
            <Property label="Transcripts">
              {backfill?.running ? (
                <Badge tone="accent" dot pulse>
                  leyendo el historial · {formatInt(backfill.files_done)} de {formatInt(backfill.files_total)}
                </Badge>
              ) : (
                <>
                  <Badge tone="positive">al día</Badge>
                  {diag && <span className="muted num">{formatInt(diag.transcript_files)} ficheros</span>}
                </>
              )}
            </Property>
            <Property label="Sesiones vivas">
              <span className="num">{formatInt(live.length)}</span>
              <span className="muted">· {formatInt(withMod.length)} con el mod conectado</span>
            </Property>
            <Property label="Proyectos vigilados">
              {roots.length === 0 ? (
                <span className="muted">ninguno</span>
              ) : (
                roots.map((w) => (
                  <Badge key={w.root_id} tone={WATCH[w.watch]?.tone ?? "neutral"} title={WATCH[w.watch]?.label}>
                    {w.name}
                  </Badge>
                ))
              )}
            </Property>
          </PropertyList>
          {live.length > 0 && (
            <ul className="dg-sessions">
              {live.map((s) => (
                <li key={s.id}>
                  <span className="mono">{s.id.slice(0, 8)}</span>
                  <span className="muted">{s.engine_version ? `motor ${s.engine_version}` : "motor desconocido"}</span>
                  <Badge tone={s.mod_connected ? "positive" : "attention"} icon="plug">
                    {s.mod_connected ? "mod conectado" : "sin mod"}
                  </Badge>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card title="Base de datos">
          {diag ? (
            <PropertyList>
              <Property label="Esquema">
                <span className="num">versión {diag.schema_version}</span>
              </Property>
              <Property label="Tamaño">
                <span className="num">{formatBytes(diag.db_bytes)}</span>
                <span className="muted num">+ {formatBytes(diag.wal_bytes)} de WAL</span>
              </Property>
              <Property label="Sesiones">
                <span className="num">{formatInt(diag.sessions)}</span>
              </Property>
              <Property label="Mensajes de la API">
                <span className="num">{formatInt(diag.api_messages)}</span>
              </Property>
              <Property label="Herramientas">
                <span className="num">{formatInt(diag.tool_calls)}</span>
                <span className="muted num">· {formatInt(diag.tool_call_copies)} copias de historial</span>
              </Property>
              <Property label="Errores de la API">
                {diag.api_errors > 0 ? (
                  <Badge tone="attention">{formatInt(diag.api_errors)}</Badge>
                ) : (
                  <span className="num">0</span>
                )}
              </Property>
            </PropertyList>
          ) : (
            failure ? <Unavailable /> : <Skeleton lines={5} />
          )}
        </Card>

        <Card title="Agregación diaria y retención">
          {diag ? (
            <>
              <PropertyList>
                {diag.aggregation.map((a) => (
                  <Property key={a.target} label={TARGET_LABEL[a.target]}>
                    {a.pending_days === 0 && a.mismatched_days === 0 ? (
                      <Badge tone="positive" icon="check">
                        al día
                      </Badge>
                    ) : (
                      <>
                        {a.pending_days > 0 && (
                          <Badge tone="attention">{formatInt(a.pending_days)} días por agregar</Badge>
                        )}
                        {a.mismatched_days > 0 && (
                          <Badge tone="critical">{formatInt(a.mismatched_days)} días no cuadran</Badge>
                        )}
                      </>
                    )}
                  </Property>
                ))}
                <Property label="Detalle de rutas">
                  <span>
                    <span className="num">{formatInt(diag.path_detail.tool_calls_with_path)}</span>
                    <span className="muted"> herramientas y </span>
                    <span className="num">{formatInt(diag.path_detail.file_events)}</span>
                    <span className="muted"> cambios de fichero</span>
                    {diag.path_detail.oldest_day && <span className="muted"> · desde {diag.path_detail.oldest_day}</span>}
                  </span>
                </Property>
              </PropertyList>
              <p className="dg-note">
                Las rutas se guardan los días fijados en «Privacidad y retención»; después solo quedan los totales por
                día (§6.4.1).
              </p>
            </>
          ) : (
            failure ? <Unavailable /> : <Skeleton lines={3} />
          )}
        </Card>

        <Card title="Motores con el mod">
          {!diag ? (
            failure ? <Unavailable /> : <Skeleton lines={2} />
          ) : diag.engines.length === 0 ? (
            <EmptyState icon="plug" title="Ningún mod se ha conectado todavía" compact>
              Las sesiones se ven igual, solo con los transcripts.
            </EmptyState>
          ) : (
            <PropertyList>
              {diag.engines.map((e) => (
                <Property key={`${e.engine_version}-${e.mod_version}`} label={`Claude Code ${e.engine_version ?? "?"}`}>
                  <span className="mono">mod {e.mod_version ?? "?"}</span>
                  <span className="muted">
                    · {formatInt(e.sessions)} sesiones · último {formatAgo(e.last_hello_at, now)}
                  </span>
                </Property>
              ))}
            </PropertyList>
          )}
        </Card>

        <PrivacySettings now={now} onCleared={refresh} />

        <Card title="Incidencias de la ingesta" className="grid-wide">
          {!diag ? (
            failure ? <Unavailable /> : <Skeleton lines={3} />
          ) : diag.ingest_errors.length === 0 ? (
            <EmptyState icon="check-circle" title="Ninguna incidencia" compact>
              Todas las líneas de los transcripts se han entendido.
            </EmptyState>
          ) : (
            <div className="ui-table-wrap">
              <table className="ui-table">
                <thead>
                  <tr>
                    <th>Tipo</th>
                    <th>Detalle</th>
                    <th className="n">Veces</th>
                    <th className="n">Ficheros</th>
                    <th className="n">Última</th>
                  </tr>
                </thead>
                <tbody>
                  {diag.ingest_errors.map((e) => (
                    <tr key={`${e.kind}-${e.detail}`}>
                      <td>{ERROR_LABEL[e.kind] ?? e.kind}</td>
                      <td className="mono">{e.detail || "—"}</td>
                      <td className="n">{formatInt(e.count)}</td>
                      <td className="n">{formatInt(e.files)}</td>
                      <td className="n">{formatAgo(e.last_ts, now)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </Card>
      </div>
    </View>
  );
}
