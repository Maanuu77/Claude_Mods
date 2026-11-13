import type { ReactNode } from "react";

/**
 * Iconos propios (docs/DESIGN.md §4): rejilla de 16 px, trazo de 1,5 px, extremos
 * redondeados y `currentColor`. Sin dependencias.
 */
const PATHS = {
  activity: <polyline points="1.5 8.5 4.5 8.5 6.5 3.5 9.5 12.5 11.5 7.5 14.5 7.5" />,
  tasks: (
    <>
      <path d="M6.5 4h7M6.5 8h7M6.5 12h7" />
      <path d="M1.8 4.1l1 1 1.9-2.1M1.8 8.1l1 1 1.9-2.1" />
      <circle cx="3.2" cy="12" r="1.15" />
    </>
  ),
  changes: (
    <>
      <path d="M3.5 2a.5.5 0 0 1 .5-.5h5.5l3 3V14a.5.5 0 0 1-.5.5H4a.5.5 0 0 1-.5-.5z" />
      <path d="M9.5 1.5v3h3M6 8.5h4M8 6.5v4M6 12.2h4" />
    </>
  ),
  map: (
    <>
      <circle cx="8" cy="3.5" r="1.8" />
      <circle cx="3.5" cy="12" r="1.8" />
      <circle cx="12.5" cy="12" r="1.8" />
      <path d="M7.1 5.1 4.4 10.4M8.9 5.1l2.7 5.3M5.3 12h5.4" />
    </>
  ),
  history: <path d="M2 13.5h12M4.5 11V8M8 11V4.5M11.5 11V6.5" />,
  gauge: (
    <>
      <path d="M2.4 11.5a5.6 5.6 0 1 1 11.2 0" />
      <path d="M8 11.5l2.6-4.2" />
      <path d="M4.2 6.6l.8.6M8 4.9v1M11.8 6.6l-.8.6" />
    </>
  ),
  refresh: (
    <>
      <path d="M13 2.8v3.4H9.6" />
      <path d="M12.6 6.1A5.1 5.1 0 1 0 13.1 9" />
    </>
  ),
  check: <path d="M3.5 8.4l3 3 6-6.6" />,
  "check-circle": (
    <>
      <circle cx="8" cy="8" r="6.25" />
      <path d="M5.4 8.2l1.8 1.8 3.4-3.7" />
    </>
  ),
  alert: (
    <>
      <path d="M7.13 2.6a1 1 0 0 1 1.74 0l5.4 9.4a1 1 0 0 1-.87 1.5H2.6a1 1 0 0 1-.87-1.5z" />
      <path d="M8 6.4v3.1" />
      <circle cx="8" cy="11.4" r="0.35" fill="currentColor" />
    </>
  ),
  "x-circle": (
    <>
      <circle cx="8" cy="8" r="6.25" />
      <path d="M6 6l4 4M10 6l-4 4" />
    </>
  ),
  info: (
    <>
      <circle cx="8" cy="8" r="6.25" />
      <path d="M8 7.3v3.9" />
      <circle cx="8" cy="5.1" r="0.35" fill="currentColor" />
    </>
  ),
  clock: (
    <>
      <circle cx="8" cy="8" r="6.25" />
      <path d="M8 4.7V8l2.3 1.5" />
    </>
  ),
  circle: <circle cx="8" cy="8" r="5.25" />,
  "circle-dot": (
    <>
      <circle cx="8" cy="8" r="5.25" />
      <circle cx="8" cy="8" r="2.2" fill="currentColor" stroke="none" />
    </>
  ),
  "circle-slash": (
    <>
      <circle cx="8" cy="8" r="5.25" />
      <path d="M4.3 11.7l7.4-7.4" />
    </>
  ),
  branch: (
    <>
      <circle cx="4.5" cy="3.5" r="1.5" />
      <circle cx="4.5" cy="12.5" r="1.5" />
      <circle cx="11.5" cy="5" r="1.5" />
      <path d="M4.5 5v6M11.5 6.5c0 2.6-2.4 3.4-6 4.6" />
    </>
  ),
  folder: (
    <path d="M1.5 4.5a1 1 0 0 1 1-1h3.3l1.5 1.5h6.2a1 1 0 0 1 1 1v6.5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z" />
  ),
  file: (
    <>
      <path d="M4 1.5h5.5l3 3V14a.5.5 0 0 1-.5.5H4a.5.5 0 0 1-.5-.5V2a.5.5 0 0 1 .5-.5z" />
      <path d="M9.5 1.5v3h3" />
    </>
  ),
  "chevron-right": <path d="M6 3.5 10.5 8 6 12.5" />,
  "chevron-down": <path d="M3.5 6 8 10.5 12.5 6" />,
  plug: (
    <>
      <path d="M6 1.5v3M10 1.5v3" />
      <path d="M4.5 4.5h7V7a3.5 3.5 0 0 1-7 0z" />
      <path d="M8 10.5v4" />
    </>
  ),
  agent: (
    <>
      <rect x="3" y="5" width="10" height="8" rx="2" />
      <path d="M8 2.6V5" />
      <circle cx="8" cy="2.1" r="0.6" fill="currentColor" />
      <path d="M6 8.6v.8M10 8.6v.8" />
    </>
  ),
  terminal: (
    <>
      <rect x="1.5" y="2.5" width="13" height="11" rx="1.5" />
      <path d="M4.5 6.3l2 1.7-2 1.7M8.5 10.2h3" />
    </>
  ),
  coin: (
    <>
      <circle cx="8" cy="8" r="6.25" />
      <path d="M9.9 6.1c-.4-.6-1.1-.9-1.9-.9-1.1 0-1.9.6-1.9 1.4 0 1.9 3.8 1 3.8 2.9 0 .8-.8 1.4-1.9 1.4-.9 0-1.6-.3-2-.9M8 4v1.2M8 10.9V12" />
    </>
  ),
  shield: <path d="M8 1.8 13 3.6v4.1c0 3.1-2.1 5.4-5 6.5-2.9-1.1-5-3.4-5-6.5V3.6z" />,
  trash: <path d="M2.5 4.5h11M6.3 4.5V3a.5.5 0 0 1 .5-.5h2.4a.5.5 0 0 1 .5.5v1.5M4 4.5l.7 9h6.6l.7-9" />,
  "arrow-up": <path d="M8 12.5v-9M4.5 7 8 3.5 11.5 7" />,
  "arrow-down": <path d="M8 3.5v9M4.5 9 8 12.5 11.5 9" />,
  database: (
    <>
      <ellipse cx="8" cy="3.8" rx="5" ry="2" />
      <path d="M3 3.8v8.4c0 1.1 2.2 2 5 2s5-.9 5-2V3.8M3 8c0 1.1 2.2 2 5 2s5-.9 5-2" />
    </>
  ),
  hourglass: (
    <>
      <path d="M4.5 1.8h7M4.5 14.2h7" />
      <path d="M5.2 1.8c0 3.1 5.6 3.4 5.6 6.2s-5.6 3.1-5.6 6.2M10.8 1.8c0 3.1-5.6 3.4-5.6 6.2s5.6 3.1 5.6 6.2" />
    </>
  ),
  layers: (
    <>
      <path d="M8 2 14 5.2 8 8.4 2 5.2z" />
      <path d="M2 8.2l6 3.2 6-3.2M2 11.2l6 3.2 6-3.2" />
    </>
  ),
  eye: (
    <>
      <path d="M1.5 8s2.4-4.5 6.5-4.5S14.5 8 14.5 8 12.1 12.5 8 12.5 1.5 8 1.5 8z" />
      <circle cx="8" cy="8" r="1.9" />
    </>
  ),
  minus: <path d="M3.5 8h9" />,
} satisfies Record<string, ReactNode>;

export type IconName = keyof typeof PATHS;

export function Icon({
  name,
  size = 16,
  label,
  className,
}: {
  name: IconName;
  size?: number;
  /** Nombre accesible cuando el icono va solo. Si no, es decorativo. */
  label?: string;
  className?: string;
}) {
  return (
    <svg
      className={className ? `ui-icon ${className}` : "ui-icon"}
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      {PATHS[name]}
    </svg>
  );
}

/**
 * Marca de DEV COCKPIT: un dial de instrumento. Colores fijos (no dependen del
 * tema) para que la marca sea siempre la misma; es también el icono de la app.
 */
export function LogoMark({ size = 28 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true" className="ui-logo">
      <rect width="32" height="32" rx="8" fill="#2459DB" />
      <path d="M8.2 21.6a9 9 0 1 1 15.6 0" fill="none" stroke="#FFFFFF" strokeOpacity="0.38" strokeWidth="2.4" strokeLinecap="round" />
      <path d="M8.2 21.6a9 9 0 0 1 11.6-12.4" fill="none" stroke="#FFFFFF" strokeWidth="2.4" strokeLinecap="round" />
      <path d="M16 17.5l4.6-6.1" stroke="#FFFFFF" strokeWidth="2.2" strokeLinecap="round" />
      <circle cx="16" cy="17.5" r="2.1" fill="#FFFFFF" />
    </svg>
  );
}
