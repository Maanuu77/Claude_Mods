/**
 * Componentes compartidos del sistema visual (docs/DESIGN.md §6). Las vistas no
 * definen botones, insignias, avisos ni estados propios: usan estos.
 */
import { type ButtonHTMLAttributes, type CSSProperties, type KeyboardEvent, type ReactNode, useId } from "react";
import { Icon, type IconName } from "./icons";
import type { Tone } from "./tone";

const cx = (...parts: (string | false | null | undefined)[]) => parts.filter(Boolean).join(" ");

// --- Controles ------------------------------------------------------------------

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "secondary" | "ghost" | "danger";
  size?: "sm" | "md";
  icon?: IconName;
  /** Botón solo de icono: el texto pasa a nombre accesible y tooltip. */
  iconOnly?: boolean;
};

export function Button({
  variant = "secondary",
  size = "md",
  icon,
  iconOnly = false,
  className,
  children,
  type = "button",
  title,
  ...rest
}: ButtonProps) {
  const label = typeof children === "string" ? children : undefined;
  return (
    <button
      type={type}
      className={cx("ui-btn", `ui-btn-${variant}`, `ui-btn-${size}`, iconOnly && "ui-btn-icon", className)}
      title={title ?? (iconOnly ? label : undefined)}
      aria-label={iconOnly ? label : undefined}
      {...rest}
    >
      {icon && <Icon name={icon} size={size === "sm" ? 14 : 16} />}
      {!iconOnly && children}
    </button>
  );
}

export interface SegmentedOption<T extends string | number> {
  value: T;
  label: ReactNode;
  count?: number;
  title?: string;
}

/** Selector de una sola opción (periodo, proyecto). */
export function Segmented<T extends string | number>({
  options,
  value,
  onChange,
  label,
}: {
  options: SegmentedOption<T>[];
  value: T;
  onChange: (value: T) => void;
  label: string;
}) {
  // Patrón de grupo de radio (WAI-ARIA): un solo punto de tabulación y flechas
  // para cambiar de opción.
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (step === 0 || options.length === 0) return;
    e.preventDefault();
    const current = Math.max(0, options.findIndex((o) => o.value === value));
    const next = (current + step + options.length) % options.length;
    onChange(options[next].value);
    (e.currentTarget.children[next] as HTMLElement | undefined)?.focus();
  };
  return (
    <div className="ui-segmented" role="radiogroup" aria-label={label} onKeyDown={onKeyDown}>
      {options.map((o) => (
        <button
          key={String(o.value)}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          tabIndex={o.value === value ? 0 : -1}
          className={cx("ui-seg", o.value === value && "ui-seg-on")}
          onClick={() => onChange(o.value)}
          title={o.title}
        >
          {o.label}
          {o.count !== undefined && o.count > 0 && <span className="ui-seg-count num">{o.count}</span>}
        </button>
      ))}
    </div>
  );
}

export function Switch({
  checked,
  onChange,
  label,
  help,
  disabled,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  help?: string;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <div className="ui-field ui-field-inline">
      <button
        id={id}
        type="button"
        role="switch"
        aria-checked={checked}
        className={cx("ui-switch", checked && "ui-switch-on")}
        onClick={() => onChange(!checked)}
        disabled={disabled}
      >
        <span className="ui-switch-thumb" />
      </button>
      <label htmlFor={id} className="ui-field-text">
        <span className="ui-field-label">{label}</span>
        {help && <span className="ui-field-help">{help}</span>}
      </label>
    </div>
  );
}

export function NumberField({
  value,
  min,
  max,
  onChange,
  label,
  help,
  unit,
}: {
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
  label: string;
  help?: string;
  unit?: string;
}) {
  const id = useId();
  return (
    <div className="ui-field ui-field-inline">
      <span className="ui-number">
        <input
          id={id}
          type="number"
          min={min}
          max={max}
          value={value}
          className="num"
          onChange={(e) => onChange(Math.min(max, Math.max(min, Number(e.target.value) || min)))}
        />
        {unit && <span className="ui-number-unit">{unit}</span>}
      </span>
      <label htmlFor={id} className="ui-field-text">
        <span className="ui-field-label">{label}</span>
        {help && <span className="ui-field-help">{help}</span>}
      </label>
    </div>
  );
}

/** Confirmación en línea de una acción irreversible. */
export function ConfirmBar({
  message,
  confirmLabel,
  onConfirm,
  onCancel,
  busy = false,
}: {
  message: string;
  confirmLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
  busy?: boolean;
}) {
  return (
    <div className="ui-confirm" role="alertdialog" aria-label={message}>
      <Icon name="alert" />
      <span className="ui-confirm-text">{message}</span>
      <Button variant="danger" size="sm" onClick={onConfirm} disabled={busy}>
        {busy ? "Un momento…" : confirmLabel}
      </Button>
      <Button variant="ghost" size="sm" onClick={onCancel} disabled={busy}>
        Cancelar
      </Button>
    </div>
  );
}

// --- Estado ---------------------------------------------------------------------

export function StatusDot({ tone, pulse = false, label }: { tone: Tone; pulse?: boolean; label?: string }) {
  return (
    <span
      className={cx("ui-dot", `ui-tone-${tone}`, pulse && "ui-dot-pulse")}
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    />
  );
}

export function Badge({
  tone = "neutral",
  variant = "soft",
  dot = false,
  pulse = false,
  icon,
  title,
  children,
}: {
  tone?: Tone;
  variant?: "soft" | "outline" | "solid";
  dot?: boolean;
  pulse?: boolean;
  icon?: IconName;
  title?: string;
  children: ReactNode;
}) {
  return (
    <span className={cx("ui-badge", `ui-badge-${variant}`, `ui-tone-${tone}`)} title={title}>
      {dot && <span className={cx("ui-badge-dot", pulse && "ui-dot-pulse")} aria-hidden="true" />}
      {icon && <Icon name={icon} size={12} />}
      {children}
    </span>
  );
}

export function ProgressBar({
  value,
  max,
  tone = "accent",
  label,
  size = "md",
}: {
  value: number;
  max: number;
  tone?: Tone;
  label: string;
  size?: "sm" | "md";
}) {
  const pct = max > 0 ? Math.min(100, Math.max(0, (value / max) * 100)) : 0;
  return (
    <div
      className={cx("ui-progress", `ui-progress-${size}`, `ui-tone-${tone}`)}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={max}
      aria-valuenow={value}
    >
      <span className="ui-progress-fill" style={{ width: `${pct}%` }} />
    </div>
  );
}

/**
 * Dial holográfico de 0 a 100 % (contexto, límites): aro de 270° proyectado sobre su
 * base. Sin dato (`null`), el dial queda apagado con su explicación.
 */
export function Gauge({
  percent,
  tone,
  label,
  emptyHint = "sin dato",
}: {
  percent: number | null;
  tone: Tone;
  label: string;
  emptyHint?: string;
}) {
  const arc = 179; // 270° de un aro de radio 38 (perímetro 239).
  const value = percent === null ? 0 : Math.min(100, Math.max(0, percent));
  return (
    <div
      className={cx("ui-gauge", `ui-tone-${percent === null ? "neutral" : tone}`, percent === null && "ui-gauge-empty")}
      role="img"
      aria-label={percent === null ? `${label}: ${emptyHint}` : `${label}: ${Math.round(value)} %`}
    >
      <svg width="104" height="104" viewBox="0 0 100 100" aria-hidden="true">
        <circle cx="50" cy="50" r="46" fill="none" className="ui-gauge-ticks" strokeWidth="1" strokeDasharray="1 5" transform="rotate(135 50 50)" />
        <circle cx="50" cy="50" r="38" fill="none" className="ui-gauge-track" strokeWidth="7" strokeDasharray={`${arc} 239`} strokeLinecap="round" transform="rotate(135 50 50)" />
        {percent !== null && value > 0 && (
          <circle cx="50" cy="50" r="38" fill="none" className="ui-gauge-arc" strokeWidth="7" strokeDasharray={`${(arc * value) / 100} 239`} strokeLinecap="round" transform="rotate(135 50 50)" />
        )}
      </svg>
      <span className="ui-gauge-value">
        {percent === null ? "—" : Math.round(value)}
        {percent !== null && <small> %</small>}
      </span>
      <span className="ui-gauge-cone" aria-hidden="true" />
      <span className="ui-gauge-base" aria-hidden="true" />
      <span className="ui-gauge-label">{label}</span>
      {percent === null && <span className="ui-field-help">{emptyHint}</span>}
    </div>
  );
}

/** Reactor de sesión: anillos 3D del color de su estado; solo giran si la sesión trabaja. */
export function Reactor({ tone, live, size = "md", label }: { tone: Tone; live: boolean; size?: "sm" | "md"; label?: string }) {
  return (
    <div
      className={cx("ui-reactor", `ui-tone-${tone}`, live && "ui-reactor-live", size === "sm" && "ui-reactor-sm")}
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      <span className="ui-reactor-ring ui-reactor-r1" />
      <span className="ui-reactor-ring ui-reactor-r2" />
      <span className="ui-reactor-ring ui-reactor-r3" />
      <span className="ui-reactor-core" />
    </div>
  );
}

// --- Estructura -----------------------------------------------------------------

export function ViewHeader({
  eyebrow,
  title,
  summary,
  actions,
}: {
  /** Antetítulo HUD: el nombre de la sala y su número. */
  eyebrow?: string;
  title: string;
  summary?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <header className="ui-view-head">
      <div className="ui-view-titles">
        {eyebrow && <p className="ui-eyebrow">{eyebrow}</p>}
        <h1>{title}</h1>
        {summary && <p className="ui-view-summary">{summary}</p>}
      </div>
      {actions && <div className="ui-view-actions">{actions}</div>}
    </header>
  );
}

export function Card({
  title,
  subtitle,
  actions,
  edge,
  live = false,
  plane,
  className,
  children,
  as: Tag = "section",
  titleAttr,
}: {
  title?: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
  /** Canto de estado encendido: solo para entidades vivas (sesiones, subagentes). */
  edge?: Tone;
  /** La entidad trabaja: un barrido de luz recorre su canto. */
  live?: boolean;
  /** Plano de profundidad: más cerca o más lejos (DESIGN.md §5). */
  plane?: "lift" | "far";
  className?: string;
  children?: ReactNode;
  as?: "section" | "article" | "div";
  titleAttr?: string;
}) {
  return (
    <Tag
      className={cx(
        "ui-card",
        edge && "ui-card-edge",
        edge && `ui-tone-${edge}`,
        plane && `ui-card-${plane}`,
        className,
      )}
    >
      {edge && <span className={cx("ui-card-beam", live && "ui-card-beam-live")} aria-hidden="true" />}
      {(title || actions) && (
        <header className="ui-card-head">
          <div className="ui-card-titles">
            {title && (
              <h2 className="ui-card-title truncate" title={titleAttr}>
                {title}
              </h2>
            )}
            {subtitle && <p className="ui-card-subtitle">{subtitle}</p>}
          </div>
          {actions && <div className="ui-card-actions">{actions}</div>}
        </header>
      )}
      {children}
    </Tag>
  );
}

export function SectionLabel({ children, count, actions }: { children: ReactNode; count?: number; actions?: ReactNode }) {
  return (
    <div className="ui-section-label">
      <h3>
        {children}
        {count !== undefined && <span className="ui-section-count num">{count}</span>}
      </h3>
      {actions}
    </div>
  );
}

export function Metric({
  label,
  value,
  hint,
  hintTone,
  icon,
  size = "md",
  title,
}: {
  label: string;
  value: ReactNode;
  hint?: ReactNode;
  hintTone?: Tone;
  icon?: IconName;
  size?: "md" | "lg";
  title?: string;
}) {
  return (
    <div className={cx("ui-metric", `ui-metric-${size}`)} title={title}>
      <span className="ui-metric-label">
        {icon && <Icon name={icon} size={14} />}
        {label}
      </span>
      <span className="ui-metric-value num">{value}</span>
      {hint && <span className={cx("ui-metric-hint", hintTone && `ui-text-${hintTone}`)}>{hint}</span>}
    </div>
  );
}

export function PropertyList({ children }: { children: ReactNode }) {
  return <dl className="ui-props">{children}</dl>;
}

export function Property({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="ui-prop">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

// --- Avisos y estados de vista ------------------------------------------------------

const NOTICE_ICON: Record<"info" | "attention" | "critical" | "positive", IconName> = {
  info: "info",
  attention: "alert",
  critical: "x-circle",
  positive: "check-circle",
};

export function Notice({
  tone = "info",
  title,
  children,
  actions,
  progress,
  live = false,
}: {
  tone?: "info" | "attention" | "critical" | "positive";
  title?: ReactNode;
  children?: ReactNode;
  actions?: ReactNode;
  progress?: { value: number; max: number; label: string };
  /** Se anuncia a los lectores de pantalla al aparecer. */
  live?: boolean;
}) {
  return (
    <div
      className={cx("ui-notice", `ui-notice-${tone}`)}
      role={tone === "critical" ? "alert" : live ? "status" : undefined}
    >
      <Icon name={NOTICE_ICON[tone]} className="ui-notice-icon" />
      <div className="ui-notice-body">
        {title && <p className="ui-notice-title">{title}</p>}
        {children && <div className="ui-notice-text">{children}</div>}
        {progress && (
          <ProgressBar
            value={progress.value}
            max={progress.max}
            tone={tone === "info" ? "accent" : tone}
            size="sm"
            label={progress.label}
          />
        )}
      </div>
      {actions && <div className="ui-notice-actions">{actions}</div>}
    </div>
  );
}

export function EmptyState({
  icon,
  title,
  children,
  action,
  compact = false,
}: {
  icon: IconName;
  title: string;
  children?: ReactNode;
  action?: ReactNode;
  compact?: boolean;
}) {
  return (
    <div className={cx("ui-empty", compact && "ui-empty-compact")}>
      <span className="ui-empty-icon">
        <Icon name={icon} size={compact ? 18 : 22} />
      </span>
      <p className="ui-empty-title">{title}</p>
      {children && <p className="ui-empty-text">{children}</p>}
      {action && <div className="ui-empty-action">{action}</div>}
    </div>
  );
}

export function Loading({ label }: { label: string }) {
  return (
    <div className="ui-loading" role="status">
      <span className="ui-spinner" aria-hidden="true" />
      <span>{label}</span>
    </div>
  );
}

export function Skeleton({ lines = 3 }: { lines?: number }) {
  return (
    <div className="ui-skeleton" aria-hidden="true">
      {Array.from({ length: lines }, (_, i) => (
        <span key={i} className="ui-skeleton-line" style={{ width: `${92 - ((i * 17) % 40)}%` }} />
      ))}
    </div>
  );
}

// --- Datos ------------------------------------------------------------------------

export interface PrismDatum {
  key: string;
  value: number;
  /** Texto del tooltip: fecha y cifra legibles. */
  label: string;
}

/**
 * Prismas 3D en proyección oblicua (una serie). La base es horizontal: las alturas se
 * comparan sin distorsión. El máximo lleva su etiqueta; los demás, al pasar el ratón.
 */
export function Prisms({
  data,
  label,
  height = 200,
  depth = 12,
  gap = 10,
  maxWidth = 30,
  markPeak = true,
}: {
  data: PrismDatum[];
  label: string;
  height?: number;
  depth?: number;
  gap?: number;
  maxWidth?: number;
  markPeak?: boolean;
}) {
  const max = Math.max(0, ...data.map((d) => d.value));
  const peak = markPeak && max > 0 ? data.findIndex((d) => d.value === max) : -1;
  const style = { "--hh": `${height}px`, "--d": `${depth}px`, "--gap": `${gap}px`, "--max": `${maxWidth}px` } as CSSProperties;
  return (
    <div className="ui-prisms" style={style} role="img" aria-label={label}>
      {data.map((d, i) => {
        const pct = max > 0 ? (d.value / max) * 100 : 0;
        return (
          <span
            key={d.key}
            className={cx("ui-prism", d.value === 0 && "ui-prism-zero", i === peak && "ui-prism-peak")}
            style={{ "--v": `${Math.max(pct, d.value > 0 ? 1.5 : 0)}%` } as CSSProperties}
            title={d.label}
          >
            {d.value > 0 && (
              <span className={cx("ui-prism-tip", i < data.length * 0.2 && "ui-prism-tip-start", i > data.length * 0.8 && "ui-prism-tip-end")}>
                {d.label}
              </span>
            )}
          </span>
        );
      })}
    </div>
  );
}

/** Estela de actividad: una luz por minuto con herramientas; más ancha cuantas más hubo. */
export function Trail({ values, label }: { values: number[]; label: string }) {
  const max = Math.max(1, ...values);
  const step = 100 / Math.max(1, values.length);
  return (
    <div className="ui-trail" role="img" aria-label={label}>
      {values.map((n, i) =>
        n === 0 ? null : (
          <i
            key={i}
            style={{ left: `calc(${i * step}% + 4px)`, "--w": `${6 + Math.round((n / max) * 18)}px` } as CSSProperties}
          />
        ),
      )}
      <span className="ui-trail-now" aria-hidden="true" />
    </div>
  );
}
