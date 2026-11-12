import { useMemo } from "react";
import { formatAgo, formatDuration, formatInt, projectName, toolLabel } from "../live/format";
import { sortSessions, useLive } from "../live/store";
import { SOURCE_LABEL } from "./labels";
import type { ActivityView, AgentState, AgentView, SessionView, TaskItemView, TaskListView } from "../live/types";

const AGENT_LABEL: Record<AgentState, string> = {
  pending: "pendiente",
  running: "en marcha",
  waiting: "esperando",
  idle: "en espera",
  completed: "terminado",
  failed: "con error",
  killed: "detenido",
};

const STATUS_MARK: Record<TaskItemView["status"], string> = {
  completed: "✓",
  in_progress: "●",
  pending: "○",
  deleted: "×",
};

/** Título, o «Tarea n.º k» si la política no guarda títulos (D1). */
function itemTitle(list: TaskListView, item: TaskItemView, index: number): string {
  if (item.subject) return item.subject;
  const number = list.source === "task_tools" ? item.task_id : String(index + 1);
  return `Tarea n.º ${number}`;
}

function TaskList({ list }: { list: TaskListView }) {
  const { total, completed } = list.progress;
  const done = new Set(list.items.filter((i) => i.status === "completed").map((i) => i.task_id));
  return (
    <section className="tm-list">
      <header className="tm-list-head">
        <h3>{SOURCE_LABEL[list.source]}</h3>
        <span className="tm-count">
          {formatInt(completed)} / {formatInt(total)} completadas
        </span>
      </header>
      <progress
        className="tm-bar"
        value={completed}
        max={Math.max(total, 1)}
        aria-label={`${completed} de ${total} tareas completadas`}
      />
      <ol className="tm-items">
        {list.items.map((item, i) => {
          const blockers = item.blocked_by.filter((id) => !done.has(id));
          return (
            <li key={item.task_id} className={`tm-item tm-${item.status}`}>
              <span className="tm-mark" aria-hidden="true">
                {STATUS_MARK[item.status]}
              </span>
              <span className="tm-title">{itemTitle(list, item, i)}</span>
              {item.owner && <span className="tag">{item.owner}</span>}
              {blockers.length > 0 && (
                <span className="tag tag-wait">bloqueada por {blockers.map((b) => `#${b}`).join(", ")}</span>
              )}
            </li>
          );
        })}
      </ol>
    </section>
  );
}

function AgentNode({
  agent,
  tree,
  now,
}: {
  agent: AgentView;
  tree: Map<string | null, AgentView[]>;
  now: number;
}) {
  const kids = tree.get(agent.agent_id) ?? [];
  const active = agent.status === "running" || agent.status === "waiting" || agent.status === "pending";
  const time =
    agent.duration_ms !== null
      ? formatDuration(agent.duration_ms)
      : active
        ? formatDuration(now - agent.first_seen)
        : formatDuration(agent.last_seen - agent.first_seen);
  return (
    <li>
      <div className={`tm-agent tm-agent-${agent.status ?? "unknown"}`}>
        <span className="tool-dot" aria-hidden="true" />
        <span className="tm-agent-type">{agent.agent_type ?? "agente"}</span>
        <span className="tag">{agent.status ? AGENT_LABEL[agent.status] : "sin estado"}</span>
        <span className="tm-agent-id" title={agent.agent_id}>
          {agent.agent_id.slice(0, 8)}
        </span>
        <span className="tool-time">{time}</span>
      </div>
      {kids.length > 0 && (
        <ul className="tm-tree">
          {kids.map((k) => (
            <AgentNode key={k.agent_id} agent={k} tree={tree} now={now} />
          ))}
        </ul>
      )}
    </li>
  );
}

function AgentTree({ agents, now }: { agents: AgentView[]; now: number }) {
  const tree = useMemo(() => {
    const ids = new Set(agents.map((a) => a.agent_id));
    const map = new Map<string | null, AgentView[]>();
    for (const a of agents) {
      // Un padre desconocido cuelga de la raíz para no perder al agente.
      const parent = a.parent_id && ids.has(a.parent_id) ? a.parent_id : null;
      map.set(parent, [...(map.get(parent) ?? []), a]);
    }
    return map;
  }, [agents]);
  return (
    <ul className="tm-tree tm-tree-root">
      {(tree.get(null) ?? []).map((a) => (
        <AgentNode key={a.agent_id} agent={a} tree={tree} now={now} />
      ))}
    </ul>
  );
}

const MINUTE = 60_000;
const SPARK_MINUTES = 30;

/** Herramientas por minuto en los últimos 30 minutos, con el reloj local. */
function Sparkline({ activity, now }: { activity: ActivityView; now: number }) {
  const nowMinute = Math.floor(now / MINUTE);
  const counts = Array.from({ length: SPARK_MINUTES }, (_, i) => {
    const minute = nowMinute - SPARK_MINUTES + 1 + i;
    const index = minute - Math.floor(activity.since / MINUTE);
    return index >= 0 && index < activity.per_minute.length ? activity.per_minute[index] : 0;
  });
  const max = Math.max(1, ...counts);
  const total = counts.reduce((a, b) => a + b, 0);
  const width = SPARK_MINUTES * 6;
  return (
    <svg
      className="tm-spark"
      viewBox={`0 0 ${width} 24`}
      role="img"
      aria-label={`${total} herramientas en los últimos 30 minutos`}
    >
      {counts.map((n, i) => {
        const h = n === 0 ? 1 : Math.max(2, Math.round((n / max) * 22));
        return <rect key={i} x={i * 6} y={24 - h} width={4} height={h} className={n === 0 ? "tm-spark-zero" : ""} />;
      })}
    </svg>
  );
}

function SessionTasks({ session, now }: { session: SessionView; now: number }) {
  const name = projectName(session.cwd) ?? `Sesión ${session.id.slice(0, 8)}`;
  const recentTotal = session.activity.per_minute.reduce((a, b) => a + b, 0);
  const current = session.running_tools[0];
  return (
    <article className="card tm-card">
      <header className="card-head">
        <h2 title={session.cwd ?? undefined}>{name}</h2>
        <span className={`pill ${session.live ? "pill-idle" : ""}`}>{session.live ? "Viva" : "Sin proceso"}</span>
      </header>
      <p className="card-meta">sesión {session.id.slice(0, 8)}</p>

      {session.tasks.length === 0 ? (
        <p className="tm-none">Sin lista de tareas declarada.</p>
      ) : (
        session.tasks.map((list) => <TaskList key={list.source} list={list} />)
      )}

      {session.agents.length > 0 && (
        <section>
          <h3>Subagentes</h3>
          <AgentTree agents={session.agents} now={now} />
        </section>
      )}

      <section className="tm-activity" aria-label="Actividad">
        <h3>Actividad</h3>
        <div className="tm-activity-row">
          <Sparkline activity={session.activity} now={now} />
          <span className="card-meta">
            {formatInt(recentTotal)} herramientas en 30 min
            {current ? ` · ahora: ${toolLabel(current.tool)}` : ""} · último evento{" "}
            {formatAgo(session.last_seen_at, now)}
          </span>
        </div>
      </section>
    </article>
  );
}

/**
 * Vista «Tareas» (SPEC §8): lo que cada sesión declara sobre su trabajo. El ritmo
 * de herramientas va aparte, como actividad, y nunca cuenta como progreso.
 */
export function TaskMonitor({ now }: { now: number }) {
  const sessions = useLive((s) => s.sessions);
  const ordered = useMemo(() => sortSessions(sessions), [sessions]);
  if (ordered.length === 0) {
    return <p className="empty">No hay sesiones con actividad reciente.</p>;
  }
  return (
    <div className="grid">
      {ordered.map((s) => (
        <SessionTasks key={s.id} session={s} now={now} />
      ))}
    </div>
  );
}
