import { Channel, invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { BackfillState, FileEventView, LiveDelta, SessionView, WorkspaceView } from "./types";

/** Cambios por raíz que se guardan en memoria; los anteriores se piden a Rust. */
const MAX_EVENTS_PER_ROOT = 2000;

type EventsByRoot = Record<number, Record<number, FileEventView>>;

interface LiveStore {
  connected: boolean;
  error: string | null;
  sessions: Record<string, SessionView>;
  backfill: BackfillState | null;
  workspaces: Record<number, WorkspaceView>;
  fileEvents: EventsByRoot;
  apply: (delta: LiveDelta) => void;
  /** Añade cambios más antiguos pedidos con `query_file_events`. */
  addHistory: (events: FileEventView[]) => void;
}

function mergeEvents(base: EventsByRoot, events: FileEventView[]): EventsByRoot {
  if (events.length === 0) return base;
  const next = { ...base };
  const touched = new Set<number>();
  for (const e of events) {
    if (!touched.has(e.root_id)) {
      next[e.root_id] = { ...(next[e.root_id] ?? {}) };
      touched.add(e.root_id);
    }
    next[e.root_id][e.id] = e;
  }
  for (const root of touched) {
    const ids = Object.keys(next[root]).map(Number);
    if (ids.length > MAX_EVENTS_PER_ROOT) {
      ids.sort((a, b) => a - b);
      for (const id of ids.slice(0, ids.length - MAX_EVENTS_PER_ROOT)) delete next[root][id];
    }
  }
  return next;
}

export const useLive = create<LiveStore>((set) => ({
  connected: false,
  error: null,
  sessions: {},
  backfill: null,
  workspaces: {},
  fileEvents: {},
  apply: (delta) =>
    set((state) => {
      if (delta.type === "snapshot") {
        return {
          connected: true,
          sessions: Object.fromEntries(delta.sessions.map((s) => [s.id, s])),
          backfill: delta.backfill,
          workspaces: Object.fromEntries(delta.workspaces.map((w) => [w.root_id, w])),
          fileEvents: mergeEvents({}, delta.file_events),
        };
      }
      const sessions = { ...state.sessions };
      for (const id of delta.removed) delete sessions[id];
      for (const s of delta.upserts) sessions[s.id] = s;
      let workspaces = state.workspaces;
      let fileEvents = state.fileEvents;
      if (delta.workspaces.length > 0 || delta.workspaces_removed.length > 0) {
        workspaces = { ...workspaces };
        fileEvents = { ...fileEvents };
        for (const id of delta.workspaces_removed) {
          delete workspaces[id];
          delete fileEvents[id];
        }
        for (const w of delta.workspaces) workspaces[w.root_id] = w;
      }
      fileEvents = mergeEvents(fileEvents, delta.file_events);
      return { sessions, backfill: delta.backfill ?? state.backfill, workspaces, fileEvents };
    }),
  addHistory: (events) => set((state) => ({ fileEvents: mergeEvents(state.fileEvents, events) })),
}));

/** Latencias hasta el pintado. Solo cuando la app se lanza para medir B3, B4, B6 y B7. */
let perfEnabled = false;
let newestToolTs = 0;
const seenFileEvents = new Set<number>();
const lastGitUpdate = new Map<number, number>();

function mark(kind: string, eventTs: number) {
  requestAnimationFrame(() => {
    void invoke("perf_mark", { kind, eventTsMs: eventTs, paintedTsMs: Date.now() });
  });
}

function markPainted(delta: LiveDelta) {
  if (!perfEnabled || delta.type !== "update") return;
  // B6: un cambio de fichero que aparece por primera vez, desde el evento del sistema.
  const fresh = delta.file_events.filter((e) => !seenFileEvents.has(e.id));
  for (const e of fresh) seenFileEvents.add(e.id);
  if (fresh.length > 0) mark("file", Math.min(...fresh.map((e) => e.ts)));
  // B7: un `numstat` nuevo, desde el último cambio de fichero que incluye.
  for (const w of delta.workspaces) {
    const git = w.git;
    if (!git || git.based_on_ts <= 0 || lastGitUpdate.get(w.root_id) === git.updated_at) continue;
    lastGitUpdate.set(w.root_id, git.updated_at);
    mark("numstat", git.based_on_ts);
  }
  if (delta.upserts.length === 0) return;
  // B3: una herramienta que aparece por primera vez, desde su inicio real.
  const toolTs = Math.max(
    0,
    ...delta.upserts.flatMap((s) => [...s.running_tools, ...s.recent_tools].map((t) => t.started_at)),
  );
  const isNewTool = toolTs > newestToolTs;
  if (isNewTool) newestToolTs = toolTs;
  // B4: cualquier otra actualización, desde la última actividad de la sesión.
  const eventTs = isNewTool ? toolTs : Math.max(...delta.upserts.map((s) => s.last_seen_at));
  mark(isNewTool ? "tool" : "other", eventTs);
}

let started = false;

/** Abre el stream una sola vez. Rust envía una instantánea y después los cambios. */
export function connectLive() {
  if (started) return;
  started = true;
  const channel = new Channel<LiveDelta>();
  channel.onmessage = (delta) => {
    if (delta.type === "snapshot") {
      for (const e of delta.file_events) seenFileEvents.add(e.id);
    }
    useLive.getState().apply(delta);
    markPainted(delta);
  };
  void invoke<boolean>("perf_enabled").then((enabled) => {
    perfEnabled = enabled;
  });
  invoke("subscribe", { onDelta: channel }).catch((e: unknown) => {
    started = false;
    useLive.setState({ error: String(e) });
  });
}

/** Sesiones ordenadas: primero las vivas, después por actividad más reciente. */
export function sortSessions(sessions: Record<string, SessionView>): SessionView[] {
  return Object.values(sessions).sort(
    (a, b) =>
      Number(b.live !== null) - Number(a.live !== null) ||
      b.last_seen_at - a.last_seen_at ||
      a.id.localeCompare(b.id),
  );
}
