import { type CSSProperties, useMemo } from "react";
import { LiveEmpty, View } from "../app/Shell";
import { ACTIVITY_MINUTES, perMinute } from "../live/activity";
import { sessionState } from "../live/SessionCard";
import { formatAgo, formatDuration, formatInt, projectName, toolLabel } from "../live/format";
import { sortSessions, useLive } from "../live/store";
import type { AgentState, AgentView, SessionView, TaskItemView, TaskListView } from "../live/types";
import { Badge, Card, EmptyState, Prisms, ProgressBar, SectionLabel, StatusDot } from "../ui/components";
import { type Tone, agentTone } from "../ui/tone";
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

/** Una lista sobre su columna de luz: hecho en verde, en curso encendido, bloqueos en ámbar. */
function TaskList({ list }: { list: TaskListView }) {
  const { total, completed } = list.progress;
  const done = new Set(list.items.filter((i) => i.status === "completed").map((i) => i.task_id));
  const finished = total > 0 && completed === total;
  const reached = list.items.findIndex((i) => i.status !== "completed" && i.status !== "deleted");
  const lit = list.items.length > 0 ? ((reached < 0 ? list.items.length : reached + 0.5) / list.items.length) * 100 : 0;
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
        size="sm"
        label={`${completed} de ${total} tareas completadas`}
      />
      <ol className="tm-spine" style={{ "--lit": `${lit}%` } as CSSProperties}>
        {list.items.map((item, i) => {
          const blockers = item.blocked_by.filter((id) => !done.has(id));
          return (
            <li key={item.task_id} className={`tm-step tm-${item.status}`}>
              <span className="sr-only">{STATUS_LABEL[item.status]}: </span>
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

function agentTime(agent: AgentView, now: number): string {
  const active = agent.status === "running" || agent.status === "waiting" || agent.status === "pending";
  return agent.duration_ms !== null
    ? formatDuration(agent.duration_ms)
    : active
      ? formatDuration(now - agent.first_seen)
      : formatDuration(agent.last_seen - agent.first_seen);
}

function AgentNode({ agent, tree, now }: { agent: AgentView; tree: Map<string | null, AgentView[]>; now: number }) {
  const kids = tree.get(agent.agent_id) ?? [];
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
        <span className="tm-agent-time num">{agentTime(agent, now)}</span>
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

const TONE_VAR: Record<Tone, string> = {
  accent: "var(--accent-text)",
  positive: "var(--positive)",
  attention: "var(--attention)",
  critical: "var(--critical)",
  neutral: "var(--neutral)",
};

interface Placed {
  id: string;
  label: string;
  status: string;
  tone: Tone;
  x: number;
  y: number;
  height: number;
  parent: { x: number; y: number } | null;
}

/**
 * Constelación de subagentes sobre un plano holográfico inclinado: el bucle principal
 * en el centro, los subagentes alrededor y sus hijos un anillo más afuera. Es la vista
 * de conjunto; la lista de al lado lleva todos los datos. Con más de 5 nodos el plano
 * solo pinta los puntos, para que las etiquetas nunca se monten.
 */
function Constellation({ agents, tree }: { agents: AgentView[]; tree: Map<string | null, AgentView[]> }) {
  // Tres filas en profundidad: subagentes al fondo, el bucle principal en medio y los
  // hijos delante. Así ninguna etiqueta queda detrás de otra.
  const placed = useMemo(() => {
    const out: Placed[] = [];
    const center = { x: 50, y: 56 };
    const roots = tree.get(null) ?? [];
    const kids = roots.flatMap((root) => (tree.get(root.agent_id) ?? []).map((kid) => ({ kid, root })));
    const spread = (i: number, n: number) => (n === 1 ? 50 : 12 + (76 * i) / (n - 1));
    const rootAt = new Map<string, { x: number; y: number }>();
    roots.forEach((root, i) => {
      const p = { x: spread(i, roots.length), y: 18 };
      rootAt.set(root.agent_id, p);
      out.push(place(root, p, center, 46));
    });
    kids.forEach(({ kid, root }, i) => {
      out.push(place(kid, { x: spread(i, kids.length), y: 92 }, rootAt.get(root.agent_id) ?? center, 18));
    });
    return out;
  }, [tree]);
  const labelled = agents.length <= 5;
  return (
    <div className="tm-space" aria-hidden="true">
      <div className="tm-plane">
        <svg className="tm-links" viewBox="0 0 100 100" preserveAspectRatio="none">
          {placed.map(
            (n) =>
              n.parent && (
                <line
                  key={n.id}
                  x1={n.parent.x}
                  y1={n.parent.y}
                  x2={n.x}
                  y2={n.y}
                  stroke={TONE_VAR[n.tone]}
                  strokeWidth="1.5"
                  strokeDasharray={n.height < 40 ? "4 4" : undefined}
                  vectorEffect="non-scaling-stroke"
                  opacity="0.8"
                />
              ),
          )}
        </svg>
        <span className="tm-node tm-node-main ui-tone-accent" style={{ left: "50%", top: "56%", "--hz": "20px" } as CSSProperties}>
          <span className="tm-disc" />
          <span className="tm-pin" />
          {labelled && (
            <span className="tm-node-card">
              <b>Bucle principal</b>
            </span>
          )}
        </span>
        {placed.map((n) => (
          <span
            key={n.id}
            className={`tm-node ui-tone-${n.tone}`}
            style={{ left: `${n.x}%`, top: `${n.y}%`, "--hz": `${n.height}px` } as CSSProperties}
          >
            <span className="tm-disc" />
            <span className="tm-pin" />
            {labelled && (
              <span className="tm-node-card">
                <b>{n.label}</b>
                <span>{n.status}</span>
              </span>
            )}
          </span>
        ))}
      </div>
    </div>
  );
}

function place(agent: AgentView, p: { x: number; y: number }, parent: { x: number; y: number }, height: number): Placed {
  const clamp = (v: number) => Math.min(94, Math.max(6, v));
  return {
    id: agent.agent_id,
    label: agent.agent_type ?? "agente",
    status: agent.status ? AGENT_LABEL[agent.status] : "sin estado",
    tone: agentTone(agent.status),
    x: clamp(p.x),
    y: clamp(p.y),
    height,
    parent,
  };
}

function Agents({ agents, now }: { agents: AgentView[]; now: number }) {
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
    <div className="tm-agents">
      <Constellation agents={agents} tree={tree} />
      <ul className="tm-tree tm-tree-root ui-well">
        {(tree.get(null) ?? []).map((a) => (
          <AgentNode key={a.agent_id} agent={a} tree={tree} now={now} />
        ))}
      </ul>
    </div>
  );
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
      edge={session.live ? state.tone : undefined}
      live={state.pulse}
      plane={session.live ? undefined : "far"}
      className="tm-card"
      title={name}
      titleAttr={session.cwd ?? undefined}
      subtitle={`sesión ${session.id.slice(0, 8)}${session.model ? ` · ${session.model}` : ""}`}
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
        <div className="tm-lists">
          {session.tasks.map((list) => (
            <TaskList key={list.source} list={list} />
          ))}
        </div>
      )}

      {session.agents.length > 0 && (
        <>
          <SectionLabel count={session.agents.length}>Subagentes</SectionLabel>
          <Agents agents={session.agents} now={now} />
        </>
      )}

      <SectionLabel>
        Actividad <span className="lv-label-note">· herramientas por minuto, no es progreso</span>
      </SectionLabel>
      <Prisms
        data={counts.map((n, i) => ({
          key: String(i),
          value: n,
          label: `hace ${ACTIVITY_MINUTES - 1 - i} min · ${formatInt(n)} ${n === 1 ? "herramienta" : "herramientas"}`,
        }))}
        label={`${recentTotal} herramientas en los últimos 30 minutos`}
        height={70}
        depth={6}
        gap={4}
        maxWidth={14}
        markPeak={false}
      />
      <p className="tm-activity-text">
        <span className="num">{formatInt(recentTotal)}</span> herramientas en 30 min
        {current && (
          <>
            {" · ahora "}
            <strong>{toolLabel(current.tool)}</strong>
          </>
        )}
        <span className="muted"> · último evento {formatAgo(session.last_seen_at, now)}</span>
      </p>
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
      room="tasks"
      title="Tareas"
      summary="Lo que declara cada sesión. El ritmo de herramientas va aparte y nunca cuenta como progreso."
    >
      {ordered.length === 0 ? (
        <LiveEmpty icon="tasks" title="No hay sesiones con actividad reciente">
          Las listas de tareas aparecen en cuanto una sesión usa TaskCreate, TodoWrite o el plan del Desktop.
        </LiveEmpty>
      ) : (
        <div className="tm-grid">
          {ordered.map((s) => (
            <SessionTasks key={s.id} session={s} now={now} />
          ))}
        </div>
      )}
    </View>
  );
}
