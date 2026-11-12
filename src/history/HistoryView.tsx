import { invoke } from "@tauri-apps/api/core";
import { useEffect, useMemo, useState } from "react";
import { formatCompact, formatDuration, formatInt, formatUsd, toolLabel } from "../live/format";

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

/** Tokens de salida por día: una serie, sin leyenda (el título la nombra). */
function DailyChart({ history, now }: { history: History; now: number }) {
  const byDay = useMemo(() => new Map(history.days.map((d) => [d.day, d])), [history.days]);
  const days = useMemo(() => daysFrom(history.since_day, now), [history.since_day, now]);
  const max = Math.max(1, ...days.map((d) => byDay.get(d)?.output ?? 0));
  const W = 720;
  const H = 160;
  const top = 12;
  const bottom = 22;
  const plot = H - top - bottom;
  const step = W / days.length;
  const bar = Math.max(1, step - 2);
  return (
    <figure className="hs-figure">
      <figcaption>Tokens de salida por día</figcaption>
      <svg viewBox={`0 0 ${W} ${H}`} className="hs-chart" role="img" aria-label="Tokens de salida por día">
        <line x1={0} x2={W} y1={top + plot} y2={top + plot} className="hs-axis" />
        <text x={0} y={top - 2} className="hs-tick">
          {formatCompact(max)}
        </text>
        {days.map((day, i) => {
          const v = byDay.get(day)?.output ?? 0;
          const h = v === 0 ? 0 : Math.max(2, (v / max) * plot);
          const x = i * step + 1;
          const r = Math.min(4, bar / 2, h);
          const y = top + plot - h;
          return (
            <g key={day}>
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
      <details className="pm-ext">
        <summary>Ver como tabla</summary>
        <table className="dg-table">
          <thead>
            <tr>
              <th>Día</th>
              <th>Mensajes</th>
              <th>Entrada</th>
              <th>Salida</th>
              <th>Caché leída</th>
            </tr>
          </thead>
          <tbody>
            {history.days.map((d) => (
              <tr key={d.day}>
                <td>{dayLabel(d.day)}</td>
                <td>{formatInt(d.messages)}</td>
                <td>{formatInt(d.input)}</td>
                <td>{formatInt(d.output)}</td>
                <td>{formatInt(d.cache_read)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </details>
    </figure>
  );
}

/** Uso de los límites (0–100 %), una línea escalonada por ventana, con leyenda. */
function LimitsChart({ history, now, days }: { history: History; now: number; days: number }) {
  const series = useMemo(() => {
    const out = new Map<string, History["limits"]>();
    for (const p of history.limits) out.set(p.kind, [...(out.get(p.kind) ?? []), p]);
    return [...out.entries()].slice(0, 2);
  }, [history.limits]);
  if (series.length === 0) {
    return (
      <p className="card-meta">
        Sin lecturas de límites en este periodo: solo llegan con el mod conectado y en cuentas de suscripción.
      </p>
    );
  }
  const W = 720;
  const H = 140;
  const top = 10;
  const plot = H - top - 20;
  const from = now - days * 86_400_000;
  const x = (ts: number) => ((Math.max(ts, from) - from) / (now - from)) * (W - 70);
  const y = (pct: number) => top + plot - (Math.min(100, Math.max(0, pct)) / 100) * plot;
  return (
    <figure className="hs-figure">
      <figcaption>Uso de los límites</figcaption>
      <div className="hs-legend">
        {series.map(([kind], i) => (
          <span key={kind}>
            <i className={`hs-swatch hs-s${i + 1}`} /> {LIMIT_LABEL[kind] ?? kind}
          </span>
        ))}
      </div>
      <svg viewBox={`0 0 ${W} ${H}`} className="hs-chart" role="img" aria-label="Uso de los límites de 5 horas y 7 días">
        <line x1={0} x2={W - 70} y1={y(0)} y2={y(0)} className="hs-axis" />
        <line x1={0} x2={W - 70} y1={y(100)} y2={y(100)} className="hs-gridline" />
        <text x={0} y={y(100) - 2} className="hs-tick">
          100 %
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
              <text x={x(now) + 6} y={y(last.percent_used) + 4} className="hs-tick">
                {LIMIT_LABEL[kind] ?? kind} {Math.round(last.percent_used)} %
              </text>
              {points.map((p) => (
                <circle key={p.ts} cx={x(p.ts)} cy={y(p.percent_used)} r={6} className="hs-hit">
                  <title>{`${LIMIT_LABEL[kind] ?? kind}: ${Math.round(p.percent_used)} %${p.resets_at ? `, se restablece ${p.resets_at}` : ""}`}</title>
                </circle>
              ))}
            </g>
          );
        })}
      </svg>
    </figure>
  );
}

function GroupTable({ rows, label, empty }: { rows: History["projects"]; label: string; empty: string }) {
  return (
    <table className="dg-table">
      <thead>
        <tr>
          <th>{label}</th>
          <th>Sesiones</th>
          <th>Mensajes</th>
          <th>Salida</th>
          <th>Caché leída</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.key}>
            <td>{r.key || empty}</td>
            <td>{formatInt(r.sessions)}</td>
            <td>{formatInt(r.messages)}</td>
            <td>{formatInt(r.output)}</td>
            <td>{formatCompact(r.cache_read)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/**
 * Vista «Histórico» (SPEC §7, vistas 3 y 4): tokens por día, proyecto y modelo,
 * herramientas y límites de uso. El coste es una estimación del cliente.
 */
export function HistoryView({ now }: { now: number }) {
  const [days, setDays] = useState<(typeof PERIODS)[number]>(30);
  const [history, setHistory] = useState<History | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

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
  }, [days]);

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
    <div className="dg">
      <nav className="roots" aria-label="Periodo">
        {PERIODS.map((p) => (
          <button
            key={p}
            type="button"
            className={`root-chip${p === days ? " root-chip-on" : ""}`}
            onClick={() => setDays(p)}
          >
            Últimos {p} días
          </button>
        ))}
      </nav>
      {failure && (
        <p className="banner banner-error" role="alert">
          No se ha podido leer el histórico: {failure}
        </p>
      )}
      {history && (
        <>
          <div className="hs-tiles">
            <div className="hs-tile">
              <span className="hs-value">{formatInt(totals.messages)}</span>
              <span className="card-meta">mensajes de la API</span>
            </div>
            <div className="hs-tile">
              <span className="hs-value">{formatCompact(totals.output)}</span>
              <span className="card-meta">tokens de salida</span>
            </div>
            <div className="hs-tile">
              <span className="hs-value">{formatCompact(totals.cache_read)}</span>
              <span className="card-meta">tokens leídos de caché</span>
            </div>
            <div className="hs-tile">
              <span className="hs-value">{formatUsd(history.cost_usd)}</span>
              <span className="card-meta">
                coste estimado ({formatInt(history.cost_sessions)} sesiones
                {history.sessions_without_cost > 0 && `; ${formatInt(history.sessions_without_cost)} sin dato`})
              </span>
            </div>
          </div>

          <section className="card hs-section">
            <DailyChart history={history} now={now} />
          </section>
          <section className="card hs-section">
            <LimitsChart history={history} now={now} days={days} />
          </section>

          <div className="grid hs-grid">
            <section className="card">
              <h3>Por proyecto</h3>
              <GroupTable rows={history.projects} label="Proyecto" empty="Sin proyecto" />
            </section>
            <section className="card">
              <h3>Por modelo</h3>
              <GroupTable rows={history.models} label="Modelo" empty="?" />
            </section>
            <section className="card dg-wide">
              <h3>Herramientas</h3>
              {history.tools.length === 0 ? (
                <p className="card-meta">Ninguna en este periodo.</p>
              ) : (
                <table className="dg-table">
                  <thead>
                    <tr>
                      <th>Herramienta</th>
                      <th>Llamadas</th>
                      <th>Con error</th>
                      <th>p50</th>
                      <th>p95</th>
                    </tr>
                  </thead>
                  <tbody>
                    {history.tools.map((t) => (
                      <tr key={t.tool}>
                        <td>{toolLabel(t.tool)}</td>
                        <td>{formatInt(t.calls)}</td>
                        <td>
                          {formatInt(t.errors)} ({Math.round((t.errors / Math.max(1, t.calls)) * 100)} %)
                        </td>
                        <td>{t.p50_ms === null ? "—" : formatDuration(t.p50_ms)}</td>
                        <td>{t.p95_ms === null ? "—" : formatDuration(t.p95_ms)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </section>
          </div>
        </>
      )}
    </div>
  );
}
