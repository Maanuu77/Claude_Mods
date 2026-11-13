import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";
import { View } from "../app/Shell";
import { formatCompact, formatDuration, formatInt, formatUsd, toolLabel } from "../live/format";
import { Button, Card, EmptyState, Metric, Notice, Segmented, Skeleton } from "../ui/components";
import { useWidth } from "../ui/useWidth";
import "./history.css";

/** Espejo de `cockpit_store::History` (SPEC §7, vistas 3 y 4). */
interface Tokens {
  messages: number;
  input: number;
  output: number;
  cache_read: number;
  cache_creation: number;
}
interface History {
  since_day: string;
  days: ({ day: string } & Tokens)[];
  projects: ({ key: string; sessions: number } & Tokens)[];
  models: ({ key: string; sessions: number } & Tokens)[];
  tools: { tool: string; calls: number; errors: number; interrupted: number; p50_ms: number | null; p95_ms: number | null }[];
  limits: { ts: number; kind: string; percent_used: number; resets_at: string | null }[];
  cost_usd: number;
  cost_sessions: number;
  sessions_without_cost: number;
}

const PERIODS = [7, 30, 90] as const;
type Period = (typeof PERIODS)[number];
const LIMIT_LABEL: Record<string, string> = { five_hour: "5 horas", seven_day: "7 días" };
const dayFormat = new Intl.DateTimeFormat("es-ES", { day: "numeric", month: "short" });

/** Días del periodo, del primero a hoy, aunque no tengan datos. */
function daysFrom(since: string, now: number): string[] {
  const out: string[] = [];
  const [y, m, d] = since.split("-").map(Number);
  const cursor = new Date(y, m - 1, d);
  const end = new Date(now);
  while (cursor <= end && out.length < 400) {
    const key = `${cursor.getFullYear()}-${String(cursor.getMonth() + 1).padStart(2, "0")}-${String(cursor.getDate()).padStart(2, "0")}`;
    out.push(key);
    cursor.setDate(cursor.getDate() + 1);
  }
  return out;
}

function dayLabel(day: string): string {
  const [y, m, d] = day.split("-").map(Number);
  return dayFormat.format(new Date(y, m - 1, d));
}

/** Tokens de salida por día: una serie (`--series-1`), sin leyenda (el título la nombra). */
function DailyChart({ history, now }: { history: History; now: number }) {
  const [asTable, setAsTable] = useState(false);
  const [plotRef, W] = useWidth<HTMLDivElement>();
  const byDay = useMemo(() => new Map(history.days.map((d) => [d.day, d])), [history.days]);
  const days = useMemo(() => daysFrom(history.since_day, now), [history.since_day, now]);
  const peak = Math.max(0, ...days.map((d) => byDay.get(d)?.output ?? 0));
  const max = Math.max(1, peak);
  const H = 200;
  const top = 16;
  const bottom = 24;
  const plot = H - top - bottom;
  const step = W / days.length;
  // Marcas finas: como mucho 28 px, con al menos 2 px de separación.
  const bar = Math.max(1, Math.min(step - 2, 28));
  if (peak === 0) {
    return (
      <Card title="Tokens de salida por día">
        <EmptyState icon="history" title="Sin actividad en este periodo" compact>
          No hay mensajes de la API en los días elegidos.
        </EmptyState>
      </Card>
    );
  }
  return (
    <Card
      title="Tokens de salida por día"
      subtitle={`Máximo ${formatInt(peak)} en un día`}
      actions={
        <Button size="sm" variant="ghost" icon={asTable ? "history" : "layers"} onClick={() => setAsTable((v) => !v)}>
          {asTable ? "Ver como gráfico" : "Ver como tabla"}
        </Button>
      }
    >
      <div ref={plotRef}>
        {asTable ? (
          <div className="ui-table-wrap hs-table-scroll">
            <table className="ui-table">
              <thead>
                <tr>
                  <th>Día</th>
                  <th className="n">Mensajes</th>
                  <th className="n">Entrada</th>
                  <th className="n">Salida</th>
                  <th className="n">Caché leída</th>
                </tr>
              </thead>
              <tbody>
                {history.days.map((d) => (
                  <tr key={d.day}>
                    <td>{dayLabel(d.day)}</td>
                    <td className="n">{formatInt(d.messages)}</td>
                    <td className="n">{formatInt(d.input)}</td>
                    <td className="n">{formatInt(d.output)}</td>
                    <td className="n">{formatInt(d.cache_read)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <svg viewBox={`0 0 ${W} ${H}`} className="hs-chart" role="img" aria-label="Tokens de salida por día">
            <line x1={0} x2={W} y1={top} y2={top} className="hs-grid" />
            <line x1={0} x2={W} y1={top + plot / 2} y2={top + plot / 2} className="hs-grid" />
            <text x={0} y={top - 4} className="hs-tick">
              {formatCompact(max)}
            </text>
            {days.map((day, i) => {
              const v = byDay.get(day)?.output ?? 0;
              const h = v === 0 ? 0 : Math.max(2, (v / max) * plot);
              const x = i * step + (step - bar) / 2;
              const r = Math.min(4, bar / 2, h);
              const y = top + plot - h;
              return (
                <g key={day} className="hs-col">
                  {h > 0 && (
                    <path
                      className="hs-bar"
                      d={`M${x},${top + plot} V${y + r} Q${x},${y} ${x + r},${y} H${x + bar - r} Q${x + bar},${y} ${x + bar},${y + r} V${top + plot} Z`}
                    />
                  )}
                  {/* Zona de hover más grande que la barra. */}
                  <rect x={i * step} y={top} width={step} height={plot} className="hs-hit">
                    <title>
                      {`${dayLabel(day)}: ${formatInt(v)} tokens de salida, ${formatInt(byDay.get(day)?.messages ?? 0)} mensajes`}
                    </title>
                  </rect>
                </g>
              );
            })}
            <line x1={0} x2={W} y1={top + plot} y2={top + plot} className="hs-axis" />
            {[0, Math.floor((days.length - 1) / 2), days.length - 1]
              .filter((v, i, a) => a.indexOf(v) === i)
              .map((i) => (
                <text
                  key={i}
                  x={i * step + step / 2}
                  y={H - 6}
                  textAnchor={i === 0 ? "start" : i === days.length - 1 ? "end" : "middle"}
                  className="hs-tick"
                >
                  {dayLabel(days[i])}
                </text>
              ))}
          </svg>
        )}
      </div>
    </Card>
  );
}

/** Uso de los límites (0–100 %): una línea escalonada por ventana, con leyenda y etiqueta directa. */
function LimitsChart({ history, now, days }: { history: History; now: number; days: number }) {
  const series = useMemo(() => {
    const out = new Map<string, History["limits"]>();
    for (const p of history.limits) out.set(p.kind, [...(out.get(p.kind) ?? []), p]);
    return [...out.entries()].slice(0, 2);
  }, [history.limits]);
  const [plotRef, W] = useWidth<HTMLDivElement>();
  if (series.length === 0) {
    return (
      <Card title="Uso de los límites">
        <EmptyState icon="gauge" title="Sin lecturas de límites en este periodo" compact>
          Solo llegan con el mod conectado y en cuentas de suscripción.
        </EmptyState>
      </Card>
    );
  }
  const H = 180;
  const top = 16;
  const plot = H - top - 24;
  const right = 112;
  const from = now - days * 86_400_000;
  const x = (ts: number) => ((Math.max(ts, from) - from) / (now - from)) * (W - right);
  const y = (pct: number) => top + plot - (Math.min(100, Math.max(0, pct)) / 100) * plot;
  return (
    <Card
      title="Uso de los límites"
      subtitle="Porcentaje usado de cada ventana, según el mod"
      actions={
        <div className="hs-legend">
          {series.map(([kind], i) => (
            <span key={kind}>
              <i className={`hs-swatch hs-s${i + 1}`} /> {LIMIT_LABEL[kind] ?? kind}
            </span>
          ))}
        </div>
      }
    >
      <div ref={plotRef}>
        <svg viewBox={`0 0 ${W} ${H}`} className="hs-chart" role="img" aria-label="Uso de los límites de 5 horas y 7 días">
          <line x1={0} x2={W - right} y1={y(100)} y2={y(100)} className="hs-grid" />
          <line x1={0} x2={W - right} y1={y(50)} y2={y(50)} className="hs-grid" />
          <text x={0} y={y(100) - 4} className="hs-tick">
            100 %
          </text>
          <text x={0} y={y(50) - 4} className="hs-tick">
            50 %
          </text>
          <line x1={0} x2={W - right} y1={y(0)} y2={y(0)} className="hs-axis" />
          <text x={0} y={H - 6} className="hs-tick">
            {dayFormat.format(new Date(from))}
          </text>
          <text x={(W - right) / 2} y={H - 6} textAnchor="middle" className="hs-tick">
            {dayFormat.format(new Date((from + now) / 2))}
          </text>
          <text x={W - right} y={H - 6} textAnchor="end" className="hs-tick">
            hoy
          </text>
          {series.map(([kind, points], i) => {
            let d = "";
            points.forEach((p, k) => {
              d += k === 0 ? `M${x(p.ts)},${y(p.percent_used)}` : ` H${x(p.ts)} V${y(p.percent_used)}`;
            });
            const last = points[points.length - 1];
            d += ` H${x(now)}`;
            return (
              <g key={kind}>
                <path d={d} className={`hs-line hs-s${i + 1}`} />
                <text x={x(now) + 8} y={y(last.percent_used) + 4} className="hs-direct">
                  {LIMIT_LABEL[kind] ?? kind} {Math.round(last.percent_used)} %
                </text>
                {points.map((p) => (
                  <circle key={p.ts} cx={x(p.ts)} cy={y(p.percent_used)} r={7} className="hs-hit">
                    <title>{`${LIMIT_LABEL[kind] ?? kind}: ${Math.round(p.percent_used)} %${p.resets_at ? `, se restablece ${p.resets_at}` : ""}`}</title>
                  </circle>
                ))}
              </g>
            );
          })}
        </svg>
      </div>
    </Card>
  );
}

function GroupTable({ rows, label, empty }: { rows: History["projects"]; label: string; empty: string }) {
  if (rows.length === 0) {
    return <EmptyState icon="history" title="Sin datos en este periodo" compact />;
  }
  return (
    <div className="ui-table-wrap">
      <table className="ui-table">
        <thead>
          <tr>
            <th>{label}</th>
            <th className="n">Sesiones</th>
            <th className="n">Mensajes</th>
            <th className="n">Salida</th>
            <th className="n">Caché leída</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={r.key}>
              <td className="hs-key">{r.key || <span className="faint">{empty}</span>}</td>
              <td className="n">{formatInt(r.sessions)}</td>
              <td className="n">{formatInt(r.messages)}</td>
              <td className="n">{formatInt(r.output)}</td>
              <td className="n">{formatCompact(r.cache_read)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/**
 * Vista «Histórico» (SPEC §7, vistas 3 y 4): tokens por día, proyecto y modelo,
 * herramientas y límites de uso. El coste es una estimación del cliente.
 */
export function HistoryView({ now }: { now: number }) {
  const [days, setDays] = useState<Period>(30);
  const [history, setHistory] = useState<History | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [round, setRound] = useState(0);

  useEffect(() => {
    let alive = true;
    invoke<History>("history", { days })
      .then((h) => {
        if (!alive) return;
        setHistory(h);
        setFailure(null);
      })
      .catch((e: unknown) => alive && setFailure(String(e)));
    return () => {
      alive = false;
    };
  }, [days, round]);

  const totals = useMemo(() => {
    const t = { messages: 0, output: 0, input: 0, cache_read: 0 };
    for (const d of history?.days ?? []) {
      t.messages += d.messages;
      t.output += d.output;
      t.input += d.input;
      t.cache_read += d.cache_read;
    }
    return t;
  }, [history]);

  return (
    <View
      title="Histórico"
      summary="Consumo, herramientas y límites. El coste es una estimación del propio cliente."
      actions={
        <Segmented
          label="Periodo"
          value={days}
          onChange={setDays}
          options={PERIODS.map((p) => ({ value: p, label: `${p} días` }))}
        />
      }
    >
      {failure && (
        <Notice
          tone="critical"
          title="No se ha podido leer el histórico"
          actions={
            <Button size="sm" onClick={() => setRound((r) => r + 1)}>
              Reintentar
            </Button>
          }
        >
          {failure}
        </Notice>
      )}
      {!history && !failure && (
        <div className="hs-tiles">
          {[0, 1, 2, 3].map((i) => (
            <Card key={i}>
              <Skeleton lines={2} />
            </Card>
          ))}
        </div>
      )}
      {history && (
        <div className="hs-stack">
          <div className="hs-tiles">
            <Card>
              <Metric size="lg" icon="activity" label="Mensajes de la API" value={formatInt(totals.messages)} />
            </Card>
            <Card>
              <Metric size="lg" icon="history" label="Tokens de salida" value={formatCompact(totals.output)} />
            </Card>
            <Card>
              <Metric size="lg" icon="database" label="Tokens leídos de caché" value={formatCompact(totals.cache_read)} />
            </Card>
            <Card>
              <Metric
                size="lg"
                icon="coin"
                label="Coste estimado"
                value={formatUsd(history.cost_usd)}
                hint={`${formatInt(history.cost_sessions)} sesiones${history.sessions_without_cost > 0 ? ` · ${formatInt(history.sessions_without_cost)} sin dato` : ""}`}
              />
            </Card>
          </div>

          <DailyChart history={history} now={now} />
          <LimitsChart history={history} now={now} days={days} />

          <div className="hs-pair">
            <Card title="Por proyecto">
              <GroupTable rows={history.projects} label="Proyecto" empty="Sin proyecto" />
            </Card>
            <Card title="Por modelo">
              <GroupTable rows={history.models} label="Modelo" empty="?" />
            </Card>
          </div>
          <Card title="Herramientas">
            {history.tools.length === 0 ? (
              <EmptyState icon="terminal" title="Ninguna herramienta en este periodo" compact />
            ) : (
              <div className="ui-table-wrap">
                <table className="ui-table">
                  <thead>
                    <tr>
                      <th>Herramienta</th>
                      <th className="n">Llamadas</th>
                      <th className="n">Con error</th>
                      <th className="n">Interrumpidas</th>
                      <th className="n">p50</th>
                      <th className="n">p95</th>
                    </tr>
                  </thead>
                  <tbody>
                    {history.tools.map((t) => {
                      const pct = Math.round((t.errors / Math.max(1, t.calls)) * 100);
                      return (
                        <tr key={t.tool}>
                          <td className="hs-key">{toolLabel(t.tool)}</td>
                          <td className="n">{formatInt(t.calls)}</td>
                          <td className="n">
                            {formatInt(t.errors)} <span className={pct >= 20 ? "ui-text-critical" : "faint"}>({pct} %)</span>
                          </td>
                          <td className="n">{formatInt(t.interrupted)}</td>
                          <td className="n">{t.p50_ms === null ? "—" : formatDuration(t.p50_ms)}</td>
                          <td className="n">{t.p95_ms === null ? "—" : formatDuration(t.p95_ms)}</td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </Card>
        </div>
      )}
    </View>
  );
}
