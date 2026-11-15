import { Badge, Card, Gauge, Metric, ProgressBar, Reactor, SectionLabel, StatusDot, Trail } from "../ui/components";
import { Icon } from "../ui/icons";
import { type Tone, TOOL_STATUS_LABEL, toolTone, usageTone } from "../ui/tone";
import { SOURCE_LABEL } from "../tasks/labels";
import {
  formatAgo,
  formatCompact,
  formatDuration,
  formatInt,
  formatUsd,
  projectName,
  toolLabel,
} from "./format";
import { perMinute } from "./activity";
import type { SessionView, ToolView } from "./types";

/** Estado de la sesión según el registro de Claude Code (DESIGN.md §3). */
export function sessionState(session: SessionView): { label: string; tone: Tone; pulse: boolean } {
  if (!session.live) {
    return session.mod_connected
      ? { label: "Viva", tone: "positive", pulse: false }
      : { label: "Sin proceso", tone: "neutral", pulse: false };
  }
  switch (session.live.status) {
    case "busy":
      return { label: "Trabajando", tone: "accent", pulse: true };
    case "waiting":
      return { label: "Esperando respuesta", tone: "attention", pulse: false };
    case "idle":
      return { label: "En espera", tone: "positive", pulse: false };
    default:
      return { label: "Viva", tone: "positive", pulse: false };
  }
}

const RATE_LIMIT_LABEL: Record<string, string> = {
  five_hour: "Límite 5 h",
  seven_day: "Límite 7 días",
  seven_day_opus: "7 días (Opus)",
  seven_day_sonnet: "7 días (Sonnet)",
};

const ENTRYPOINT_LABEL: Record<string, string> = {
  cli: "CLI",
  "claude-desktop": "Desktop",
};

export function ToolRow({ tool, now }: { tool: ToolView; now: number }) {
  const waiting = tool.status === "awaiting_permission";
  const running = tool.status === "running" || waiting;
  const time = running
    ? formatDuration(now - tool.started_at)
    : tool.duration_ms !== null
      ? formatDuration(tool.duration_ms)
      : "";
  return (
    <li className={tool.status === "running" ? "lv-tool lv-tool-hot" : "lv-tool"}>
      <StatusDot tone={toolTone(tool.status)} pulse={tool.status === "running"} label={TOOL_STATUS_LABEL[tool.status]} />
      <span className="lv-tool-name">{toolLabel(tool.tool)}</span>
      {tool.agent_id && (
        <Badge tone="neutral" icon="agent" title={`Subagente ${tool.agent_id}`}>
          subagente
        </Badge>
      )}
      <span className="path" title={tool.rel_path ?? undefined}>
        {tool.rel_path ?? ""}
      </span>
      {waiting && (
        <Badge tone="attention" icon="hourglass" title="Esperando permiso">
          permiso
        </Badge>
      )}
      {tool.status === "error" && <Badge tone="critical">error</Badge>}
      {tool.status === "interrupted" && <Badge tone="neutral">interrumpida</Badge>}
      <span className="lv-tool-time num">{time}</span>
    </li>
  );
}

/** Diales de contexto y límites. Sin el mod no hay lectura: se dice, no se inventa. */
function Gauges({ session }: { session: SessionView }) {
  const snapshot = session.usage_snapshot;
  const hint = snapshot ? "sin lectura" : "solo con el mod";
  const limits = snapshot?.rate_limits ?? [];
  const known = ["five_hour", "seven_day"];
  const extra = limits.filter((l) => !known.includes(l.kind));
  const limit = (kind: string) => limits.find((l) => l.kind === kind)?.percent_used ?? null;
  const context = snapshot?.context_percent ?? null;
  return (
    <div className="lv-gauges">
      <Gauge percent={context} tone={context === null ? "neutral" : usageTone(context)} label="Contexto" emptyHint={hint} />
      {known.map((kind) => {
        const p = limit(kind);
        return <Gauge key={kind} percent={p} tone={p === null ? "neutral" : usageTone(p)} label={RATE_LIMIT_LABEL[kind]} emptyHint={hint} />;
      })}
      {extra.map((l) => (
        <Gauge key={l.kind} percent={l.percent_used} tone={usageTone(l.percent_used)} label={RATE_LIMIT_LABEL[l.kind] ?? l.kind} />
      ))}
    </div>
  );
}

/**
 * Una sesión. La principal (`hero`) va en el plano cercano con su reactor, sus diales
 * y la estela de actividad; las demás, compactas, con las mismas cifras. Las que ya no
 * tienen proceso quedan en el plano lejano.
 */
export function SessionCard({ session, now, hero = false }: { session: SessionView; now: number; hero?: boolean }) {
  const state = sessionState(session);
  const name = projectName(session.cwd) ?? `Sesión ${session.id.slice(0, 8)}`;
  const { usage } = session;
  const snapshot = session.usage_snapshot;
  const cost = snapshot?.cost_usd ?? session.cost_state_usd;
  const finished = session.recent_tools.filter((t) => t.status !== "running").slice(0, hero ? 5 : 4);
  const meta = [
    session.entrypoint && (ENTRYPOINT_LABEL[session.entrypoint] ?? session.entrypoint),
    session.engine_version && `v${session.engine_version}`,
    session.model,
  ]
    .filter(Boolean)
    .join(" · ");
  const trail = perMinute(session.activity, now);
  const recent = trail.reduce((a, b) => a + b, 0);

  const metrics = (
    <div className="lv-metrics">
      <Metric label="Tokens de salida" value={formatCompact(usage.output)} />
      <Metric label="Entrada + caché" value={formatCompact(usage.input + usage.cache_read + usage.cache_creation)} />
      <Metric label="Respuestas" value={formatInt(session.api_messages)} />
      <Metric
        label="Herramientas"
        value={formatInt(session.tool_calls)}
        hint={session.tool_errors > 0 ? `${formatInt(session.tool_errors)} con error` : undefined}
        hintTone="critical"
      />
      <Metric
        label="Coste estimado"
        value={cost !== null ? formatUsd(cost) : "n/d"}
        title="Estimación del propio cliente de Claude Code"
      />
      <Metric label="Errores de API" value={formatInt(session.api_errors)} />
    </div>
  );

  return (
    <Card
      as="article"
      edge={session.live ? state.tone : undefined}
      live={state.pulse}
      plane={hero ? "lift" : session.live ? undefined : "far"}
      className={hero ? "lv-card lv-hero" : "lv-card"}
      title={name}
      titleAttr={session.cwd ?? undefined}
      subtitle={meta || `sesión ${session.id.slice(0, 8)}`}
      actions={
        <>
          {session.mod_connected && (
            <Badge tone="positive" icon="plug">
              Mod conectado
            </Badge>
          )}
          <Badge tone={state.tone} dot pulse={state.pulse}>
            {state.label}
          </Badge>
        </>
      }
    >
      {hero ? (
        <div className="lv-core">
          <Reactor
            tone={state.tone}
            live={state.pulse}
            label={`${state.label}${session.running_tools[0] ? ` · ${toolLabel(session.running_tools[0].tool)}` : ""}`}
          />
          <div className="lv-core-data">
            <Gauges session={session} />
            {metrics}
          </div>
        </div>
      ) : (
        <>
          {snapshot && <Gauges session={session} />}
          {metrics}
        </>
      )}

      {session.tasks.length > 0 && (
        <div className="lv-plans">
          {session.tasks.map((l) => (
            <div key={l.source} className="lv-plan">
              <span className="lv-plan-label">{SOURCE_LABEL[l.source]}</span>
              <ProgressBar
                value={l.progress.completed}
                max={Math.max(l.progress.total, 1)}
                tone={l.progress.completed === l.progress.total ? "positive" : "accent"}
                size="sm"
                label={`${SOURCE_LABEL[l.source]}: ${l.progress.completed} de ${l.progress.total}`}
              />
              <span className="lv-plan-count num">
                {formatInt(l.progress.completed)} / {formatInt(l.progress.total)}
              </span>
            </div>
          ))}
        </div>
      )}

      {hero && (
        <>
          <SectionLabel>
            Estela de actividad <span className="lv-label-note">· 30 min, una luz por minuto con herramientas</span>
          </SectionLabel>
          <Trail values={trail} label={`${formatInt(recent)} herramientas en los últimos 30 minutos`} />
        </>
      )}

      <div className={hero ? "lv-lists" : undefined}>
        {session.running_tools.length > 0 && (
          <div>
            <SectionLabel count={session.running_tools.length}>En curso</SectionLabel>
            <ul className="lv-tools ui-well">
              {session.running_tools.map((t) => (
                <ToolRow key={t.tool_use_id} tool={t} now={now} />
              ))}
            </ul>
          </div>
        )}
        {finished.length > 0 && (
          <div>
            <SectionLabel>Últimas herramientas</SectionLabel>
            <ul className="lv-tools ui-well">
              {finished.map((t) => (
                <ToolRow key={t.tool_use_id} tool={t} now={now} />
              ))}
            </ul>
          </div>
        )}
      </div>

      <footer className="lv-foot">
        <span className={session.mod_connected ? "lv-mod lv-mod-on" : "lv-mod"}>
          <Icon name="plug" size={14} />
          {session.mod_connected ? "Mod conectado" : "Solo transcript"}
        </span>
        <span className="lv-foot-sep" aria-hidden="true" />
        <span className="muted">última actividad {formatAgo(session.last_seen_at, now)}</span>
        {session.api_errors > 0 && (
          <Badge tone="attention" icon="alert">
            {formatInt(session.api_errors)} {session.api_errors === 1 ? "error de API" : "errores de API"}
          </Badge>
        )}
      </footer>
    </Card>
  );
}
