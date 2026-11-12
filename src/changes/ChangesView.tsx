import { invoke } from "@tauri-apps/api/core";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useMemo, useRef, useState } from "react";
import { formatAgo, formatInt, toolLabel } from "../live/format";
import { useLive } from "../live/store";
import type { Actor, DirtyFile, FileEventView, WorkspaceView } from "../live/types";
import { DiffViewer } from "./DiffViewer";

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

const WATCH_WARNING: Record<WorkspaceView["watch"], string | null> = {
  active: null,
  network: "Unidad de red: el watcher está desactivado (§7.1).",
  too_broad: "Carpeta demasiado amplia sin Git (unidad o carpeta del usuario): no se vigila.",
  error: "No se ha podido vigilar esta carpeta.",
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
    const byTool =
      event.tool_use_id !== null && (event.actor === "claude" || event.actor === "claude-shell");
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
  if (!file) return <span className="counts" />;
  if (file.added === null && file.deleted === null) {
    return <span className="counts">{file.status === "untracked" ? "nuevo" : "bin"}</span>;
  }
  return (
    <span className="counts">
      <span className="plus">+{formatInt(file.added ?? 0)}</span>{" "}
      <span className="minus">−{formatInt(file.deleted ?? 0)}</span>
    </span>
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
      className={`fe-row${grouped ? " fe-grouped" : ""}${selected ? " fe-selected" : ""}`}
      onClick={() => path && onSelect(path)}
      disabled={!path}
    >
      <span className="fe-time">{clock.format(event.ts)}</span>
      <span className={`fe-change fe-${event.change}`}>{CHANGE_LABEL[event.change]}</span>
      <span className="fe-path" title={path ?? undefined}>
        {event.from_rel_path && <span className="fe-from">{event.from_rel_path} → </span>}
        {path ?? "(ruta no registrada)"}
      </span>
      <Counts file={dirty} />
      <span className={`actor actor-${event.actor}`} title={`Confianza ${CONFIDENCE_LABEL[event.confidence]}`}>
        {ACTOR_LABEL[event.actor]}
        <small> · {CONFIDENCE_LABEL[event.confidence]}</small>
        {!event.settled && <small> · provisional</small>}
      </span>
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
    <div ref={scroller} className="fe-list">
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((item) => {
          const row = rows[item.index];
          return (
            <div
              key={item.key}
              className="fe-slot"
              style={{ transform: `translateY(${item.start}px)`, height: item.size }}
            >
              {row.kind === "group" ? (
                <div className="fe-group">
                  <span className={`actor actor-${row.actor}`}>{toolLabel(row.tool)}</span>
                  <span>
                    {row.session ? `sesión ${row.session.slice(0, 8)}` : ""} · {formatInt(row.count)}{" "}
                    {row.count === 1 ? "cambio" : "cambios"}
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

function RootPanel({ ws, now }: { ws: WorkspaceView; now: number }) {
  const eventsById = useLive((s) => s.fileEvents[ws.root_id]);
  const addHistory = useLive((s) => s.addHistory);
  const [selected, setSelected] = useState<string | null>(null);
  const [history, setHistory] = useState<"idle" | "loading" | "done">("idle");

  const events = useMemo(
    () => Object.values(eventsById ?? {}).sort((a, b) => b.id - a.id),
    [eventsById],
  );
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

  const warning = WATCH_WARNING[ws.watch];
  return (
    <div className="root-panel">
      <header className="root-head">
        <h2 title={ws.root}>{ws.name}</h2>
        <p className="card-meta">
          {git
            ? [
                git.branch ? `rama ${git.branch}` : "HEAD separado",
                git.ahead || git.behind ? `↑${git.ahead ?? 0} ↓${git.behind ?? 0}` : null,
                git.head ? git.head.slice(0, 7) : "sin commits",
                `${formatInt(git.files_total)} ${git.files_total === 1 ? "fichero" : "ficheros"} con cambios`,
                `Git ${formatAgo(git.updated_at, now)} (${formatInt(git.took_ms)} ms)`,
              ]
                .filter(Boolean)
                .join(" · ")
            : ws.is_git
              ? "Leyendo el estado de Git…"
              : "Carpeta sin Git"}
          {ws.is_worktree && " · worktree"}
          {` · ${ws.sessions.length === 0 ? "sin sesiones vivas" : `${formatInt(ws.sessions.length)} ${ws.sessions.length === 1 ? "sesión viva" : "sesiones vivas"}`}`}
        </p>
      </header>

      {warning && <p className="banner banner-error">{warning}</p>}
      {git?.error && <p className="banner banner-error">Git no ha respondido: {git.error}</p>}
      {ws.bulk && now - ws.bulk.at < 60_000 && (
        <p className="banner" role="status">
          {ws.bulk.kind === "storm"
            ? `Cambio masivo en ${ws.bulk.dir || "la raíz"} (${formatInt(ws.bulk.events)} eventos): se muestra el estado de Git en lugar del detalle.`
            : "El sistema de ficheros perdió eventos: estado resincronizado con Git."}
        </p>
      )}

      <div className="changes-body">
        <section className="changes-left">
          <h3>Cambios en directo</h3>
          {rows.length === 0 ? (
            <p className="empty">Sin cambios todavía. Aparecerán en cuanto se modifique un fichero.</p>
          ) : (
            <EventList rows={rows} dirty={dirty} selected={selected} onSelect={setSelected} />
          )}
          {history !== "done" && (
            <button type="button" className="more" onClick={loadOlder} disabled={history === "loading"}>
              {history === "loading" ? "Cargando…" : "Cargar cambios anteriores (30 días)"}
            </button>
          )}
        </section>

        <section className="changes-right">
          <h3>Con cambios ahora</h3>
          {git && git.files.length > 0 ? (
            <ul className="dirty">
              {git.files.map((f) => (
                <li key={f.rel_path}>
                  <button
                    type="button"
                    className={`dirty-row${f.rel_path === selected ? " fe-selected" : ""}`}
                    onClick={() => setSelected(f.rel_path)}
                  >
                    <span className={`tag status-${f.status}`}>{f.status}</span>
                    <span className="fe-path" title={f.rel_path}>
                      {f.rel_path}
                    </span>
                    <Counts file={f} />
                  </button>
                </li>
              ))}
              {git.files_total > git.files.length && (
                <li className="card-meta">y {formatInt(git.files_total - git.files.length)} más</li>
              )}
            </ul>
          ) : (
            <p className="card-meta">{git ? "Árbol de trabajo limpio." : "—"}</p>
          )}
          {selected && (
            <DiffViewer
              key={`${ws.root_id}:${selected}`}
              rootId={ws.root_id}
              path={selected}
              untracked={selectedFile?.status === "untracked"}
            />
          )}
        </section>
      </div>
    </div>
  );
}

/** Vista «Cambios en directo» (SPEC §7.4): una raíz activa cada vez. */
export function ChangesView({ now }: { now: number }) {
  const workspaces = useLive((s) => s.workspaces);
  const list = useMemo(
    () =>
      Object.values(workspaces).sort(
        (a, b) => b.sessions.length - a.sessions.length || a.name.localeCompare(b.name),
      ),
    [workspaces],
  );
  const [chosen, setChosen] = useState<number | null>(null);
  const current = list.find((w) => w.root_id === chosen) ?? list[0];

  if (!current) {
    return (
      <p className="empty">
        No hay proyectos vigilados. Se vigila la raíz de Git de cada sesión viva y se deja de vigilar
        10 minutos después de que termine.
      </p>
    );
  }
  return (
    <section className="changes">
      <nav className="roots" aria-label="Proyectos vigilados">
        {list.map((w) => (
          <button
            key={w.root_id}
            type="button"
            className={`root-chip${w.root_id === current.root_id ? " root-chip-on" : ""}`}
            onClick={() => setChosen(w.root_id)}
            title={w.root}
          >
            <strong>{w.name}</strong>
            {w.git?.branch && <span> · {w.git.branch}</span>}
            {w.git && w.git.files_total > 0 && <span className="tag">{formatInt(w.git.files_total)}</span>}
          </button>
        ))}
      </nav>
      <RootPanel key={current.root_id} ws={current} now={now} />
    </section>
  );
}
