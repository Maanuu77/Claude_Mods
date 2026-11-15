/**
 * Estructura de la ventana (docs/DESIGN.md §5): el espacio de fondo, el dock lateral
 * con la marca, la navegación y el estado del núcleo; cada vista con su cabecera HUD
 * y los avisos globales; y la secuencia de apertura.
 */
import { type ReactNode, useEffect, useState } from "react";
import { formatInt } from "../live/format";
import { useLive } from "../live/store";
import { Button, EmptyState, Loading, Notice, Reactor, StatusDot, ViewHeader } from "../ui/components";
import { Icon, type IconName, LogoMark } from "../ui/icons";

export type Tab = "live" | "tasks" | "changes" | "map" | "history" | "diag";

export const NAV: { id: Tab; label: string; icon: IconName; room: string }[] = [
  { id: "live", label: "En vivo", icon: "activity", room: "Puente de mando · 01" },
  { id: "tasks", label: "Tareas", icon: "tasks", room: "Plan de misión · 02" },
  { id: "changes", label: "Cambios en directo", icon: "changes", room: "Flujo de cambios · 03" },
  { id: "map", label: "Mapa", icon: "map", room: "Constelación · 04" },
  { id: "history", label: "Histórico", icon: "history", room: "Telemetría · 05" },
  { id: "diag", label: "Diagnóstico", icon: "gauge", room: "Sistemas · 06" },
];

function percentOf(done: number, total: number): number {
  return total > 0 ? Math.round((done / total) * 100) : 0;
}

/**
 * Suelo holográfico en perspectiva, calculado una vez: radios hacia un punto de fuga y
 * horizontales que se juntan en el horizonte. Es un SVG plano, no una capa 3D.
 */
const FLOOR = (() => {
  const vx = 500;
  const vy = -140;
  const rays = Array.from({ length: 33 }, (_, i) => -1500 + i * 125).map((x) => ({ x1: vx, y1: vy, x2: x, y2: 300 }));
  const rows = Array.from({ length: 12 }, (_, i) => 300 * (1 - 0.74 ** (i + 1)));
  return { rays, rows };
})();

/** Plano 0: halos de luz, partículas y suelo holográfico. Estático: no consume en reposo. */
export function Space() {
  return (
    <div className="sh-space" aria-hidden="true">
      <div className="sh-halos" />
      <div className="sh-stars" />
      <svg className="sh-floor" viewBox="0 0 1000 300" preserveAspectRatio="none">
        <defs>
          <linearGradient id="sh-floor-fade" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#fff" stopOpacity="0" />
            <stop offset="0.35" stopColor="#fff" stopOpacity="1" />
            <stop offset="1" stopColor="#fff" stopOpacity="0.35" />
          </linearGradient>
          <mask id="sh-floor-mask">
            <rect width="1000" height="300" fill="url(#sh-floor-fade)" />
          </mask>
        </defs>
        <g mask="url(#sh-floor-mask)">
          {FLOOR.rays.map((r) => (
            <line key={r.x2} x1={r.x1} y1={r.y1} x2={r.x2} y2={r.y2} vectorEffect="non-scaling-stroke" />
          ))}
          {FLOOR.rows.map((y) => (
            <line key={y} x1={0} y1={y} x2={1000} y2={y} vectorEffect="non-scaling-stroke" />
          ))}
        </g>
      </svg>
      <div className="sh-horizon" />
    </div>
  );
}

/** Estado del núcleo: conectado, conectando o sin conexión (DESIGN.md §7). */
function CoreStatus({ liveCount, recent }: { liveCount: number; recent: number }) {
  const { connected, error, backfill } = useLive();
  const state = error ? "error" : connected ? "ok" : "connecting";
  const label = state === "error" ? "Sin conexión con el núcleo" : state === "ok" ? "Núcleo en línea" : "Conectando…";
  const short = state === "error" ? "Sin conexión" : label;
  return (
    <div className={`sh-status sh-status-${state}`} data-tip={label}>
      <div className="sh-status-row">
        <StatusDot
          tone={state === "error" ? "critical" : state === "ok" ? "positive" : "neutral"}
          pulse={state === "connecting"}
          label={label}
        />
        <span className="sh-status-text" title={label}>
          {short}
        </span>
      </div>
      {state === "ok" && (
        <p className="sh-status-meta">
          {backfill?.running
            ? `Leyendo historial · ${percentOf(backfill.bytes_done, backfill.bytes_total)} %`
            : `${formatInt(liveCount)} ${liveCount === 1 ? "sesión viva" : "sesiones vivas"} · ${formatInt(recent)} recientes`}
        </p>
      )}
    </div>
  );
}

export function Sidebar({
  tab,
  onTab,
  counts,
  liveCount,
  recent,
}: {
  tab: Tab;
  onTab: (tab: Tab) => void;
  counts: Partial<Record<Tab, number>>;
  liveCount: number;
  recent: number;
}) {
  return (
    <aside className="sh-side">
      <div className="sh-brand">
        <LogoMark size={34} />
        <span className="sh-wordmark">
          DEV <strong>COCKPIT</strong>
        </span>
      </div>
      <nav className="sh-nav" aria-label="Vistas">
        {NAV.map((item) => {
          const count = counts[item.id];
          const on = item.id === tab;
          return (
            <button
              key={item.id}
              type="button"
              className={on ? "sh-item sh-item-on" : "sh-item"}
              aria-current={on ? "page" : undefined}
              // En la barra de iconos la etiqueta se oculta: el nombre accesible no depende de ella.
              aria-label={count ? `${item.label} (${formatInt(count)})` : item.label}
              onClick={() => onTab(item.id)}
              data-tip={item.label}
            >
              <Icon name={item.icon} size={18} />
              <span className="sh-item-label">{item.label}</span>
              {count !== undefined && count > 0 && <span className="sh-count num">{formatInt(count)}</span>}
            </button>
          );
        })}
      </nav>
      <CoreStatus liveCount={liveCount} recent={recent} />
    </aside>
  );
}

const clockFormat = new Intl.DateTimeFormat("es-ES", { hour: "2-digit", minute: "2-digit" });
const dayFormat = new Intl.DateTimeFormat("es-ES", { day: "numeric", month: "short" });

/** Reloj del HUD: se actualiza una vez por minuto, al cambiar el minuto. */
function Clock() {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    let interval = 0;
    const tick = () => setNow(new Date());
    const timeout = window.setTimeout(
      () => {
        tick();
        interval = window.setInterval(tick, 60_000);
      },
      60_000 - (Date.now() % 60_000),
    );
    return () => {
      window.clearTimeout(timeout);
      window.clearInterval(interval);
    };
  }, []);
  return (
    <time className="ui-clock" dateTime={now.toISOString()}>
      {clockFormat.format(now)}
      <small>{dayFormat.format(now).replace(".", "").toUpperCase()}</small>
    </time>
  );
}

/** Avisos de toda la app: error del núcleo y lectura del historial. */
function GlobalNotices() {
  const { error, backfill } = useLive();
  if (!error && !backfill?.running) return null;
  return (
    <div className="sh-notices">
      {error && (
        <Notice tone="critical" title="No se ha podido conectar con el núcleo">
          {error}. Los datos que ya se ven se quedan; se vuelve a intentar al reabrir la app.
        </Notice>
      )}
      {backfill?.running && (
        <Notice
          tone="info"
          live
          title="Leyendo el historial de transcripts"
          progress={{
            value: backfill.bytes_done,
            max: Math.max(backfill.bytes_total, 1),
            label: "Progreso de la lectura del historial",
          }}
        >
          {formatInt(backfill.files_done)} de {formatInt(backfill.files_total)} ficheros (
          {percentOf(backfill.bytes_done, backfill.bytes_total)} %). Las sesiones aparecen según se leen.
        </Notice>
      )}
    </div>
  );
}

/**
 * Vista vacía de las que se llenan con el stream del núcleo. Distingue «conectando»,
 * «sin conexión» y «vacío de verdad» para no decir «no hay nada» cuando solo falta
 * el núcleo (DESIGN.md §7).
 */
export function LiveEmpty({ icon, title, children }: { icon: IconName; title: string; children?: ReactNode }) {
  const { connected, error } = useLive();
  if (error) {
    return (
      <EmptyState icon="plug" title="Sin datos del núcleo">
        Esta vista se llena con lo que envía el núcleo de DEV COCKPIT. El aviso de arriba explica el fallo.
      </EmptyState>
    );
  }
  if (!connected) return <Loading label="Conectando con el núcleo…" />;
  return (
    <EmptyState icon={icon} title={title}>
      {children}
    </EmptyState>
  );
}

/** Una vista: cabecera HUD (sala, título, resumen, acciones y reloj), avisos y contenido. */
export function View({
  room,
  title,
  summary,
  actions,
  children,
}: {
  room: Tab;
  title: string;
  summary?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="sh-view">
      <ViewHeader
        eyebrow={NAV.find((n) => n.id === room)?.room}
        title={title}
        summary={summary}
        actions={
          <>
            {actions}
            <Clock />
          </>
        }
      />
      <GlobalNotices />
      {children}
    </div>
  );
}

const reducedMotion = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

/**
 * Apertura: el reactor se enciende con la lista de encendido real (núcleo, historial,
 * proyectos) y los paneles llegan desde el fondo. Dura 1,6 s, se puede saltar y no se
 * muestra con «reducir movimiento».
 */
export function Boot() {
  const { connected, error, backfill, workspaces } = useLive();
  const [phase, setPhase] = useState<"on" | "leaving" | "off">(() => (reducedMotion() ? "off" : "on"));
  useEffect(() => {
    if (phase === "off") return;
    const id = window.setTimeout(() => setPhase(phase === "on" ? "leaving" : "off"), phase === "on" ? 1600 : 420);
    return () => window.clearTimeout(id);
  }, [phase]);
  if (phase === "off") return null;
  const roots = Object.keys(workspaces).length;
  const reading = backfill?.running;
  return (
    <div className={phase === "leaving" ? "sh-boot sh-boot-out" : "sh-boot"} role="status" aria-label="Arrancando DEV COCKPIT">
      <div className="sh-boot-core">
        <Reactor tone="accent" live />
        <p className="sh-boot-word">DEV COCKPIT</p>
        <p className="ui-eyebrow sh-boot-sub">Centro de mando de Claude Code</p>
        <ul className="sh-boot-list">
          <li>
            <StatusDot tone={error ? "critical" : connected ? "positive" : "accent"} pulse={!connected && !error} />
            <b>Núcleo</b>
            <span>{error ? "sin conexión" : connected ? "canal abierto" : "conectando…"}</span>
          </li>
          <li>
            <StatusDot tone={reading ? "accent" : connected ? "positive" : "neutral"} pulse={reading} />
            <b>Historial</b>
            <span>
              {reading
                ? `${formatInt(backfill.files_done)} de ${formatInt(backfill.files_total)} transcripts`
                : connected
                  ? "al día"
                  : "esperando"}
            </span>
          </li>
          <li>
            <StatusDot tone={roots > 0 ? "positive" : "neutral"} />
            <b>Proyectos</b>
            <span>{roots > 0 ? `${formatInt(roots)} vigilados` : "esperando a las sesiones vivas"}</span>
          </li>
        </ul>
      </div>
      <Button size="sm" className="sh-boot-skip" onClick={() => setPhase("off")}>
        Saltar la animación
      </Button>
    </div>
  );
}
