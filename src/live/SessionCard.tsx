import {
  formatAgo,
  formatCompact,
  formatDuration,
  formatInt,
  formatUsd,
  projectName,
  toolLabel,
} from "./format";
import type { SessionView, ToolView } from "./types";

const STATUS_LABEL: Record<string, string> = {
  busy: "Trabajando",
  idle: "En espera",
  waiting: "Esperando respuesta",
};

function liveLabel(session: SessionView): { text: string; tone: "busy" | "idle" | "off" } {
  if (!session.live) return { text: "Sin proceso", tone: "off" };
  const status = session.live.status ?? "";
  return {
    text: STATUS_LABEL[status] ?? "Viva",
    tone: status === "busy" ? "busy" : "idle",
  };
}

const ENTRYPOINT_LABEL: Record<string, string> = {
  cli: "CLI",
  "claude-desktop": "Desktop",
};

function ToolRow({ tool, now }: { tool: ToolView; now: number }) {
  const running = tool.status === "running";
  const time = running
    ? formatDuration(now - tool.started_at)
    : tool.duration_ms !== null
      ? formatDuration(tool.duration_ms)
      : "";
  return (
    <li className={`tool tool-${tool.status}`}>
      <span className="tool-dot" aria-hidden="true" />
      <span className="tool-name">{toolLabel(tool.tool)}</span>
      {tool.agent_id && <span className="tag">subagente</span>}
      <span className="tool-path" title={tool.rel_path ?? undefined}>
        {tool.rel_path ?? ""}
      </span>
      {tool.status === "error" && <span className="tag tag-error">error</span>}
      <span className="tool-time">{time}</span>
    </li>
  );
}

export function SessionCard({ session, now }: { session: SessionView; now: number }) {
  const live = liveLabel(session);
  const name = projectName(session.cwd) ?? `Sesión ${session.id.slice(0, 8)}`;
  const { usage } = session;
  const finished = session.recent_tools.filter((t) => t.status !== "running").slice(0, 5);
  return (
    <article className={`card card-${live.tone}`}>
      <header className="card-head">
        <h2 title={session.cwd ?? undefined}>{name}</h2>
        <span className={`pill pill-${live.tone}`}>{live.text}</span>
      </header>

      <p className="card-meta">
        {[
          session.entrypoint && (ENTRYPOINT_LABEL[session.entrypoint] ?? session.entrypoint),
          session.engine_version && `v${session.engine_version}`,
          session.model,
        ]
          .filter(Boolean)
          .join(" · ")}
      </p>

      <dl className="metrics">
        <div>
          <dt>Tokens de salida</dt>
          <dd>{formatCompact(usage.output)}</dd>
        </div>
        <div>
          <dt>Entrada + caché</dt>
          <dd>{formatCompact(usage.input + usage.cache_read + usage.cache_creation)}</dd>
        </div>
        <div>
          <dt>Respuestas</dt>
          <dd>{formatInt(session.api_messages)}</dd>
        </div>
        <div>
          <dt>Herramientas</dt>
          <dd>
            {formatInt(session.tool_calls)}
            {session.tool_errors > 0 && (
              <small className="warn"> · {formatInt(session.tool_errors)} con error</small>
            )}
          </dd>
        </div>
        <div>
          <dt>Coste estimado</dt>
          <dd>{session.cost_state_usd !== null ? formatUsd(session.cost_state_usd) : "n/d"}</dd>
        </div>
        <div>
          <dt>Contexto</dt>
          <dd title="El porcentaje de contexto solo lo da el mod (F3)">n/d</dd>
        </div>
      </dl>

      {session.running_tools.length > 0 && (
        <section>
          <h3>En curso</h3>
          <ul className="tools">
            {session.running_tools.map((t) => (
              <ToolRow key={t.tool_use_id} tool={t} now={now} />
            ))}
          </ul>
        </section>
      )}

      {finished.length > 0 && (
        <section>
          <h3>Últimas herramientas</h3>
          <ul className="tools">
            {finished.map((t) => (
              <ToolRow key={t.tool_use_id} tool={t} now={now} />
            ))}
          </ul>
        </section>
      )}

      <footer className="card-foot">
        Última actividad {formatAgo(session.last_seen_at, now)}
        {session.api_errors > 0 && ` · ${formatInt(session.api_errors)} errores de API`}
      </footer>
    </article>
  );
}
