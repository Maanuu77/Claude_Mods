const integer = new Intl.NumberFormat("es-ES");
const compact = new Intl.NumberFormat("es-ES", { notation: "compact", maximumFractionDigits: 1 });
const money = new Intl.NumberFormat("es-ES", {
  style: "currency",
  currency: "USD",
  minimumFractionDigits: 2,
});

export const formatInt = (n: number) => integer.format(n);
export const formatCompact = (n: number) => compact.format(n);
export const formatUsd = (n: number) => money.format(n);

/** `1 h 02 min`, `3 min 05 s`, `12 s`, `340 ms`. */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))} ms`;
  const totalSeconds = Math.floor(ms / 1000);
  const h = Math.floor(totalSeconds / 3600);
  const m = Math.floor((totalSeconds % 3600) / 60);
  const s = totalSeconds % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  if (h > 0) return `${h} h ${pad(m)} min`;
  if (m > 0) return `${m} min ${pad(s)} s`;
  return `${s} s`;
}

export function formatAgo(ts: number, now: number): string {
  const diff = now - ts;
  if (diff < 5000) return "ahora mismo";
  return `hace ${formatDuration(diff)}`;
}

/** Último segmento de una ruta: el nombre de la carpeta del proyecto. */
export function projectName(cwd: string | null): string | null {
  if (!cwd) return null;
  const parts = cwd.split(/[\\/]/).filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : null;
}

/** `mcp__servidor__herramienta` → `servidor · herramienta`. */
export function toolLabel(tool: string): string {
  const match = /^mcp__(.+?)__(.+)$/.exec(tool);
  return match ? `${match[1]} · ${match[2]}` : tool;
}
