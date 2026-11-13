/**
 * Componentes compartidos del sistema visual (docs/DESIGN.md §6). Las vistas no
 * definen botones, insignias, avisos ni estados propios: usan estos.
 */
import { type ButtonHTMLAttributes, type KeyboardEvent, type ReactNode, useId } from "react";
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

/** Uso de 0 a 100 % con su cifra; el tono lo decide quien llama (`usageTone`). */
export function Meter({ percent, tone, label }: { percent: number; tone: Tone; label: string }) {
  return (
    <div className="ui-meter">
      <span className="ui-meter-label">{label}</span>
      <ProgressBar value={percent} max={100} tone={tone} size="sm" label={`${label}: ${Math.round(percent)} %`} />
      <span className="ui-meter-value num">{Math.round(percent)} %</span>
    </div>
  );
}

// --- Estructura -----------------------------------------------------------------

export function ViewHeader({
  title,
  summary,
  actions,
}: {
  title: string;
  summary?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <header className="ui-view-head">
      <div className="ui-view-titles">
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
  className,
  children,
  as: Tag = "section",
  titleAttr,
}: {
  title?: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
  /** Filo de estado: solo para entidades vivas (sesiones, subagentes). */
  edge?: Tone;
  className?: string;
  children?: ReactNode;
  as?: "section" | "article" | "div";
  titleAttr?: string;
}) {
  return (
    <Tag className={cx("ui-card", edge && "ui-card-edge", edge && `ui-tone-${edge}`, className)}>
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

/** Actividad por minuto (barras finas). Serie única: `--series-1`. */
export function Sparkline({ values, label, width = 180, height = 28 }: { values: number[]; label: string; width?: number; height?: number }) {
  const max = Math.max(1, ...values);
  const step = width / Math.max(1, values.length);
  const bar = Math.max(1, step - 2);
  return (
    <svg className="ui-spark" viewBox={`0 0 ${width} ${height}`} width={width} height={height} role="img" aria-label={label}>
      <line x1={0} x2={width} y1={height - 0.5} y2={height - 0.5} className="ui-spark-base" />
      {values.map((n, i) =>
        n === 0 ? null : (
          <rect
            key={i}
            x={i * step + 1}
            y={height - Math.max(2, Math.round((n / max) * (height - 2)))}
            width={bar}
            height={Math.max(2, Math.round((n / max) * (height - 2)))}
            rx={Math.min(1.5, bar / 2)}
            className="ui-spark-bar"
          >
            <title>{`${n} ${n === 1 ? "herramienta" : "herramientas"}`}</title>
          </rect>
        ),
      )}
    </svg>
  );
}
