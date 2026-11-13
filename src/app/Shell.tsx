/**
 * Estructura de la ventana (docs/DESIGN.md §5): barra lateral con la marca, la
 * navegación y el estado del núcleo; cada vista con su cabecera y los avisos
 * globales debajo.
 */
import type { ReactNode } from "react";
import { formatInt } from "../live/format";
import { useLive } from "../live/store";
import { EmptyState, Loading, Notice, StatusDot, ViewHeader } from "../ui/components";
import { Icon, type IconName, LogoMark } from "../ui/icons";

export type Tab = "live" | "tasks" | "changes" | "map" | "history" | "diag";

export const NAV: { id: Tab; label: string; icon: IconName }[] = [
  { id: "live", label: "En vivo", icon: "activity" },
  { id: "tasks", label: "Tareas", icon: "tasks" },
  { id: "changes", label: "Cambios en directo", icon: "changes" },
  { id: "map", label: "Mapa", icon: "map" },
  { id: "history", label: "Histórico", icon: "history" },
  { id: "diag", label: "Diagnóstico", icon: "gauge" },
];

function percentOf(done: number, total: number): number {
  return total > 0 ? Math.round((done / total) * 100) : 0;
}

/** Estado del núcleo: conectado, conectando o sin conexión (DESIGN.md §7). */
function CoreStatus({ liveCount, recent }: { liveCount: number; recent: number }) {
  const { connected, error, backfill } = useLive();
  const state = error ? "error" : connected ? "ok" : "connecting";
  const label = state === "error" ? "Sin conexión con el núcleo" : state === "ok" ? "Núcleo conectado" : "Conectando…";
  return (
    <div className="sh-status" data-tip={label}>
      <div className="sh-status-row">
        <StatusDot
          tone={state === "error" ? "critical" : state === "ok" ? "positive" : "neutral"}
          pulse={state === "connecting"}
          label={label}
        />
        <span className="sh-status-text">{label}</span>
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
        <LogoMark size={28} />
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

/** Una vista: cabecera, avisos globales y contenido. */
export function View({
  title,
  summary,
  actions,
  children,
}: {
  title: string;
  summary?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="sh-view">
      <ViewHeader title={title} summary={summary} actions={actions} />
      <GlobalNotices />
      {children}
    </div>
  );
}
