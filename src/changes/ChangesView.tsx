import { invoke } from "@tauri-apps/api/core";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useMemo, useRef, useState } from "react";
import { LiveEmpty, View } from "../app/Shell";
import { formatAgo, formatInt, toolLabel } from "../live/format";
import { useLive } from "../live/store";
import type { Actor, DirtyFile, FileEventView, WorkspaceView } from "../live/types";
import { Badge, Button, Card, EmptyState, Notice, Segmented, SectionLabel } from "../ui/components";
import { Icon } from "../ui/icons";
import type { Tone } from "../ui/tone";
import { DiffViewer } from "./DiffViewer";
import "./changes.css";

const ACTOR_LABEL: Record<Actor, string> = {
  claude: "Claude",
  "claude-shell": "Claude (shell)",
  unknown: "Desconocido",
  external: "Externo",
};

const CONFIDENCE_LABEL = { high: "alta", medium: "media", low: "baja" } as const;

const CHANGE_LABEL = {
  created: "creado",
  modified: "modificado",
  deleted: "borrado",
  renamed: "renombrado",
} as const;

const GIT_STATUS: Record<string, { label: string; tone: Tone }> = {
  modified: { label: "modificado", tone: "neutral" },
  added: { label: "añadido", tone: "positive" },
  deleted: { label: "borrado", tone: "critical" },
  renamed: { label: "renombrado", tone: "accent" },
  copied: { label: "copiado", tone: "accent" },
  typechange: { label: "cambio de tipo", tone: "neutral" },
  untracked: { label: "nuevo", tone: "positive" },
  conflict: { label: "conflicto", tone: "critical" },
};

const WATCH: Record<WorkspaceView["watch"], { label: string; tone: Tone; warning: string | null }> = {
  active: { label: "vigilado", tone: "positive", warning: null },
  network: { label: "sin vigilar", tone: "attention", warning: "Unidad de red: el watcher está desactivado (§7.1)." },
  too_broad: {
    label: "sin vigilar",
    tone: "attention",
    warning: "Carpeta demasiado amplia sin Git (unidad o carpeta del usuario): no se vigila.",
  },
  error: { label: "error", tone: "critical", warning: "No se ha podido vigilar esta carpeta." },
};

type Row =
  | { kind: "group"; key: string; tool: string; actor: Actor; session: string | null; count: number }
  | { kind: "event"; key: string; event: FileEventView; grouped: boolean };

/** Agrupa los cambios consecutivos de una misma herramienta de Claude (§7.4). */
function buildRows(events: FileEventView[]): Row[] {
  const rows: Row[] = [];
  let group: Extract<Row, { kind: "group" }> | null = null;
  let groupTool: string | null = null;
  for (const event of events) {
    const byTool = event.tool_use_id !== null && (event.actor === "claude" || event.actor === "claude-shell");
    if (byTool && event.tool_use_id === groupTool && group) {
      group.count += 1;
    } else if (byTool) {
      group = {
        kind: "group",
        key: `g-${event.id}`,
        tool: event.tool ?? "herramienta",
        actor: event.actor,
        session: event.session_id,
        count: 1,
      };
      groupTool = event.tool_use_id;
      rows.push(group);
    } else {
      group = null;
      groupTool = null;
    }
    rows.push({ kind: "event", key: `e-${event.id}`, event, grouped: byTool });
  }
  return rows;
}

const clock = new Intl.DateTimeFormat("es-ES", { hour: "2-digit", minute: "2-digit", second: "2-digit" });

function Counts({ file }: { file: DirtyFile | undefined }) {
  if (!file) return <span className="ch-counts" />;
  if (file.added === null && file.deleted === null) {
    return <span className="ch-counts faint">{file.status === "untracked" ? "nuevo" : "binario"}</span>;
  }
  return (
    <span className="ch-counts num">
      <span className="ch-plus">+{formatInt(file.added ?? 0)}</span>
      <span className="ch-minus">−{formatInt(file.deleted ?? 0)}</span>
    </span>
  );
}

/** Quién hizo el cambio (DESIGN.md §3): Claude en azul, lo demás en gris. */
function ActorBadge({ event }: { event: FileEventView }) {
  const claude = event.actor === "claude" || event.actor === "claude-shell";
  return (
    <Badge
      tone={claude ? "accent" : "neutral"}
      variant={claude && event.confidence === "high" ? "soft" : "outline"}
      title={`Confianza ${CONFIDENCE_LABEL[event.confidence]}${event.settled ? "" : " · provisional (se asienta en 2 s)"}`}
    >
      {ACTOR_LABEL[event.actor]}
      <span className="ch-confidence">{CONFIDENCE_LABEL[event.confidence]}</span>
      {!event.settled && <span className="ch-confidence">· provisional</span>}
    </Badge>
  );
}

function EventRow({
  event,
  grouped,
  dirty,
  selected,
  onSelect,
}: {
  event: FileEventView;
  grouped: boolean;
  dirty: DirtyFile | undefined;
  selected: boolean;
  onSelect: (path: string) => void;
}) {
  const path = event.rel_path;
  return (
    <button
      type="button"
      className={`ch-row${grouped ? " ch-row-grouped" : ""}${selected ? " ch-row-on" : ""}`}
      onClick={() => path && onSelect(path)}
      disabled={!path}
    >
      <span className="ch-time num">{clock.format(event.ts)}</span>
      <span className={`ch-change ch-${event.change}`}>{CHANGE_LABEL[event.change]}</span>
      <span className="ch-path" title={path ?? undefined}>
        {event.from_rel_path && <span className="faint">{event.from_rel_path} → </span>}
        {path ?? <span className="faint">(ruta no registrada)</span>}
      </span>
      <Counts file={dirty} />
      <ActorBadge event={event} />
    </button>
  );
}

function EventList({
  rows,
  dirty,
  selected,
  onSelect,
}: {
  rows: Row[];
  dirty: Map<string, DirtyFile>;
  selected: string | null;
  onSelect: (path: string) => void;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual no es memoizable
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => 32,
    overscan: 12,
    getItemKey: (i) => rows[i].key,
  });
  return (
    <div ref={scroller} className="ch-list ui-well">
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((item) => {
          const row = rows[item.index];
          return (
            <div
              key={item.key}
              className="ch-slot"
              style={{ transform: `translateY(${item.start}px)`, height: item.size }}
            >
              {row.kind === "group" ? (
                <div className="ch-group">
                  <Icon name={row.actor === "claude-shell" ? "terminal" : "file"} size={14} />
                  <span className="ch-group-tool">{toolLabel(row.tool)}</span>
                  {row.session && <span className="mono faint">{row.session.slice(0, 8)}</span>}
                  <span className="ch-group-count num">
                    {formatInt(row.count)} {row.count === 1 ? "cambio" : "cambios"}
                  </span>
                </div>
              ) : (
                <EventRow
                  event={row.event}
                  grouped={row.grouped}
                  dirty={row.event.rel_path ? dirty.get(row.event.rel_path) : undefined}
                  selected={row.event.rel_path === selected}
                  onSelect={onSelect}
                />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function GitMeta({ ws, now }: { ws: WorkspaceView; now: number }) {
  const git = ws.git;
  if (!git) {
    return <span>{ws.is_git ? "Leyendo el estado de Git…" : "Carpeta sin Git"}</span>;
  }
  return (
    <span className="ch-meta">
      <span className="ch-meta-item">
        <Icon name="branch" size={14} />
        {git.branch ?? "HEAD separado"}
      </span>
      {(git.ahead || git.behind) && (
        <span className="ch-meta-item num" title="Commits por delante y por detrás del remoto">
          <Icon name="arrow-up" size={12} />
          {git.ahead ?? 0}
          <Icon name="arrow-down" size={12} />
          {git.behind ?? 0}
        </span>
      )}
      <span className="ch-meta-item mono">{git.head ? git.head.slice(0, 7) : "sin commits"}</span>
      <span className="ch-meta-item">
        {formatInt(git.files_total)} {git.files_total === 1 ? "fichero" : "ficheros"} con cambios
      </span>
      <span className="ch-meta-item faint">
        Git {formatAgo(git.updated_at, now)} · {formatInt(git.took_ms)} ms
      </span>
    </span>
  );
}

function RootPanel({ ws, now }: { ws: WorkspaceView; now: number }) {
  const eventsById = useLive((s) => s.fileEvents[ws.root_id]);
  const addHistory = useLive((s) => s.addHistory);
  const [selected, setSelected] = useState<string | null>(null);
  const [history, setHistory] = useState<"idle" | "loading" | "done">("idle");

  const events = useMemo(() => Object.values(eventsById ?? {}).sort((a, b) => b.id - a.id), [eventsById]);
  const rows = useMemo(() => buildRows(events), [events]);
  const git = ws.git;
  const dirty = useMemo(() => new Map((git?.files ?? []).map((f) => [f.rel_path, f])), [git]);
  const selectedFile = selected ? dirty.get(selected) : undefined;

  const loadOlder = () => {
    const oldest = events.length > 0 ? events[events.length - 1].id : undefined;
    setHistory("loading");
    invoke<FileEventView[]>("query_file_events", { rootId: ws.root_id, beforeId: oldest, limit: 200 })
      .then((older) => {
        addHistory(older);
        setHistory(older.length < 200 ? "done" : "idle");
      })
      .catch(() => setHistory("idle"));
  };

  const watch = WATCH[ws.watch];
  const sessions = ws.sessions.length;
  return (
    <Card
      className="ch-panel"
      title={ws.name}
      titleAttr={ws.root}
      subtitle={<GitMeta ws={ws} now={now} />}
      actions={
        <>
          {ws.is_worktree && <Badge tone="neutral">worktree</Badge>}
          <Badge tone={sessions > 0 ? "accent" : "neutral"} dot={sessions > 0}>
            {sessions === 0 ? "sin sesiones vivas" : `${formatInt(sessions)} ${sessions === 1 ? "sesión viva" : "sesiones vivas"}`}
          </Badge>
          <Badge tone={watch.tone}>{watch.label}</Badge>
        </>
      }
    >
      {(watch.warning || git?.error || (ws.bulk && now - ws.bulk.at < 60_000)) && (
        <div className="ch-notices">
          {watch.warning && <Notice tone={watch.tone === "critical" ? "critical" : "attention"}>{watch.warning}</Notice>}
          {git?.error && (
            <Notice tone="critical" title="Git no ha respondido">
              {git.error}
            </Notice>
          )}
          {ws.bulk && now - ws.bulk.at < 60_000 && (
            <Notice tone="attention" live>
              {ws.bulk.kind === "storm"
                ? `Cambio masivo en ${ws.bulk.dir || "la raíz"} (${formatInt(ws.bulk.events)} eventos): se muestra el estado de Git en lugar del detalle.`
                : "El sistema de ficheros perdió eventos: estado resincronizado con Git."}
            </Notice>
          )}
        </div>
      )}

      <div className="ch-body">
        <section className="ch-col">
          <SectionLabel count={events.length}>Cambios en directo</SectionLabel>
          {rows.length === 0 ? (
            <div className="ui-well">
              <EmptyState icon="changes" title="Sin cambios todavía" compact>
                Aparecerán en cuanto se cree, modifique, borre o renombre un fichero del proyecto.
              </EmptyState>
            </div>
          ) : (
            <EventList rows={rows} dirty={dirty} selected={selected} onSelect={setSelected} />
          )}
          {history !== "done" && (
            <div className="ch-more">
              <Button size="sm" variant="ghost" icon="history" onClick={loadOlder} disabled={history === "loading"}>
                {history === "loading" ? "Cargando…" : "Cargar cambios anteriores (30 días)"}
              </Button>
            </div>
          )}
        </section>

        <section className="ch-col">
          <SectionLabel count={git?.files_total}>Con cambios ahora</SectionLabel>
          {git && git.files.length > 0 ? (
            <ul className="ch-dirty ui-well">
              {git.files.map((f) => {
                const st = GIT_STATUS[f.status] ?? { label: f.status, tone: "neutral" as Tone };
                return (
                  <li key={f.rel_path}>
                    <button
                      type="button"
                      className={`ch-dirty-row${f.rel_path === selected ? " ch-row-on" : ""}`}
                      onClick={() => setSelected(f.rel_path)}
                    >
                      <Badge tone={st.tone}>{st.label}</Badge>
                      <span className="ch-path" title={f.rel_path}>
                        {f.rel_path}
                      </span>
                      {f.staged && <span className="faint">preparado</span>}
                      <Counts file={f} />
                    </button>
                  </li>
                );
              })}
              {git.files_total > git.files.length && (
                <li className="ch-dirty-more muted">y {formatInt(git.files_total - git.files.length)} más</li>
              )}
            </ul>
          ) : (
            <div className="ui-well">
              <EmptyState icon={git ? "check-circle" : "branch"} title={git ? "Árbol de trabajo limpio" : "Sin datos de Git"} compact>
                {git ? "Ningún fichero difiere del último commit." : "Esta raíz no tiene Git o aún se está leyendo."}
              </EmptyState>
            </div>
          )}
          {selected && (
            <DiffViewer
              key={`${ws.root_id}:${selected}`}
              rootId={ws.root_id}
              path={selected}
              untracked={selectedFile?.status === "untracked"}
              onClose={() => setSelected(null)}
            />
          )}
        </section>
      </div>
    </Card>
  );
}

/** Vista «Cambios en directo» (SPEC §7.4): una raíz activa cada vez. */
export function ChangesView({ now }: { now: number }) {
  const workspaces = useLive((s) => s.workspaces);
  const list = useMemo(
    () =>
      Object.values(workspaces).sort((a, b) => b.sessions.length - a.sessions.length || a.name.localeCompare(b.name)),
    [workspaces],
  );
  const [chosen, setChosen] = useState<number | null>(null);
  const current = list.find((w) => w.root_id === chosen) ?? list[0];

  return (
    <View
      title="Cambios en directo"
      summary={
        list.length === 0
          ? "Ningún proyecto vigilado"
          : `${formatInt(list.length)} ${list.length === 1 ? "proyecto vigilado" : "proyectos vigilados"} · quién cambia qué, al momento`
      }
      actions={
        list.length > 1 && current ? (
          <Segmented
            label="Proyectos vigilados"
            value={current.root_id}
            onChange={setChosen}
            options={list.map((w) => ({
              value: w.root_id,
              label: w.name,
              count: w.git?.files_total,
              title: w.root,
            }))}
          />
        ) : undefined
      }
    >
      {current ? (
        <RootPanel key={current.root_id} ws={current} now={now} />
      ) : (
        <LiveEmpty icon="changes" title="No hay proyectos vigilados">
          Se vigila la raíz de Git de cada sesión viva y se deja de vigilar 10 minutos después de que termine.
        </LiveEmpty>
      )}
    </View>
  );
}
