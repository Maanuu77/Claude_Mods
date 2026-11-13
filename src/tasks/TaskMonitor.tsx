import { useMemo } from "react";
import { LiveEmpty, View } from "../app/Shell";
import { sessionState } from "../live/SessionCard";
import { formatAgo, formatDuration, formatInt, projectName, toolLabel } from "../live/format";
import { sortSessions, useLive } from "../live/store";
import type { ActivityView, AgentState, AgentView, SessionView, TaskItemView, TaskListView } from "../live/types";
import { Badge, Card, EmptyState, ProgressBar, SectionLabel, Sparkline, StatusDot } from "../ui/components";
import { Icon, type IconName } from "../ui/icons";
import { agentTone } from "../ui/tone";
import { SOURCE_LABEL } from "./labels";
import "../live/live.css";
import "./tasks.css";

const AGENT_LABEL: Record<AgentState, string> = {
  pending: "pendiente",
  running: "en marcha",
  waiting: "esperando",
  idle: "en espera",
  completed: "terminado",
  failed: "con error",
  killed: "detenido",
};

const STATUS_ICON: Record<TaskItemView["status"], IconName> = {
  completed: "check-circle",
  in_progress: "circle-dot",
  pending: "circle",
  deleted: "circle-slash",
};

const STATUS_LABEL: Record<TaskItemView["status"], string> = {
  completed: "completada",
  in_progress: "en curso",
  pending: "pendiente",
  deleted: "borrada",
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
  const finished = total > 0 && completed === total;
  return (
    <section className="tm-list">
      <SectionLabel
        actions={
          <span className="tm-count num">
            {formatInt(completed)} de {formatInt(total)}
          </span>
        }
      >
        {SOURCE_LABEL[list.source]}
      </SectionLabel>
      <ProgressBar
        value={completed}
        max={Math.max(total, 1)}
        tone={finished ? "positive" : "accent"}
        label={`${completed} de ${total} tareas completadas`}
      />
      <ol className="tm-items">
        {list.items.map((item, i) => {
          const blockers = item.blocked_by.filter((id) => !done.has(id));
          return (
            <li key={item.task_id} className={`tm-item tm-${item.status}`}>
              <span className="tm-mark">
                <Icon name={STATUS_ICON[item.status]} size={15} label={STATUS_LABEL[item.status]} />
              </span>
              <span className="tm-title">{itemTitle(list, item, i)}</span>
              {item.owner && (
                <Badge tone="neutral" icon="agent">
                  {item.owner}
                </Badge>
              )}
              {blockers.length > 0 && (
                <Badge tone="attention" icon="circle-slash">
                  bloqueada por {blockers.map((b) => `#${b}`).join(", ")}
                </Badge>
              )}
            </li>
          );
        })}
      </ol>
    </section>
  );
}

function AgentNode({ agent, tree, now }: { agent: AgentView; tree: Map<string | null, AgentView[]>; now: number }) {
  const kids = tree.get(agent.agent_id) ?? [];
  const active = agent.status === "running" || agent.status === "waiting" || agent.status === "pending";
  const time =
    agent.duration_ms !== null
      ? formatDuration(agent.duration_ms)
      : active
        ? formatDuration(now - agent.first_seen)
        : formatDuration(agent.last_seen - agent.first_seen);
  const tone = agentTone(agent.status);
  return (
    <li>
      <div className="tm-agent">
        <StatusDot tone={tone} pulse={agent.status === "running"} />
        <span className="tm-agent-type">{agent.agent_type ?? "agente"}</span>
        <Badge tone={tone}>{agent.status ? AGENT_LABEL[agent.status] : "sin estado"}</Badge>
        <span className="tm-agent-id mono faint" title={agent.agent_id}>
          {agent.agent_id.slice(0, 8)}
        </span>
        <span className="tm-agent-time num">{time}</span>
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
function perMinute(activity: ActivityView, now: number): number[] {
  const nowMinute = Math.floor(now / MINUTE);
  return Array.from({ length: SPARK_MINUTES }, (_, i) => {
    const minute = nowMinute - SPARK_MINUTES + 1 + i;
    const index = minute - Math.floor(activity.since / MINUTE);
    return index >= 0 && index < activity.per_minute.length ? activity.per_minute[index] : 0;
  });
}

function SessionTasks({ session, now }: { session: SessionView; now: number }) {
  const state = sessionState(session);
  const name = projectName(session.cwd) ?? `Sesión ${session.id.slice(0, 8)}`;
  const counts = perMinute(session.activity, now);
  const recentTotal = counts.reduce((a, b) => a + b, 0);
  const current = session.running_tools[0];
  return (
    <Card
      as="article"
      edge={state.tone}
      title={name}
      titleAttr={session.cwd ?? undefined}
      subtitle={`sesión ${session.id.slice(0, 8)}`}
      actions={
        <Badge tone={state.tone} dot pulse={state.pulse}>
          {state.label}
        </Badge>
      }
    >
      {session.tasks.length === 0 ? (
        <EmptyState icon="tasks" title="Sin lista de tareas declarada" compact>
          Esta sesión no ha usado TaskCreate, TodoWrite ni el plan del Desktop. Nunca se inventa un progreso.
        </EmptyState>
      ) : (
        session.tasks.map((list) => <TaskList key={list.source} list={list} />)
      )}

      {session.agents.length > 0 && (
        <>
          <SectionLabel count={session.agents.length}>Subagentes</SectionLabel>
          <AgentTree agents={session.agents} now={now} />
        </>
      )}

      <SectionLabel>Actividad</SectionLabel>
      <div className="tm-activity">
        <Sparkline values={counts} label={`${recentTotal} herramientas en los últimos 30 minutos`} />
        <p className="tm-activity-text">
          <span className="num">{formatInt(recentTotal)}</span> herramientas en 30 min
          {current && (
            <>
              {" · ahora "}
              <strong>{toolLabel(current.tool)}</strong>
            </>
          )}
          <br />
          <span className="muted">último evento {formatAgo(session.last_seen_at, now)}</span>
        </p>
      </div>
    </Card>
  );
}

/**
 * Vista «Tareas» (SPEC §8): lo que cada sesión declara sobre su trabajo. El ritmo
 * de herramientas va aparte, como actividad, y nunca cuenta como progreso.
 */
export function TaskMonitor({ now }: { now: number }) {
  const sessions = useLive((s) => s.sessions);
  const ordered = useMemo(() => sortSessions(sessions), [sessions]);
  return (
    <View
      title="Tareas"
      summary="Lo que declara cada sesión. El ritmo de herramientas va aparte y nunca cuenta como progreso."
    >
      {ordered.length === 0 ? (
        <LiveEmpty icon="tasks" title="No hay sesiones con actividad reciente">
          Las listas de tareas aparecen en cuanto una sesión usa TaskCreate, TodoWrite o el plan del Desktop.
        </LiveEmpty>
      ) : (
        <div className="grid-cards">
          {ordered.map((s) => (
            <SessionTasks key={s.id} session={s} now={now} />
          ))}
        </div>
      )}
    </View>
  );
}
