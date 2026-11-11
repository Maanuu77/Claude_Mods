import { Channel, invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { BackfillState, LiveDelta, SessionView } from "./types";

interface LiveStore {
  connected: boolean;
  error: string | null;
  sessions: Record<string, SessionView>;
  backfill: BackfillState | null;
  apply: (delta: LiveDelta) => void;
}

export const useLive = create<LiveStore>((set) => ({
  connected: false,
  error: null,
  sessions: {},
  backfill: null,
  apply: (delta) =>
    set((state) => {
      if (delta.type === "snapshot") {
        return {
          connected: true,
          sessions: Object.fromEntries(delta.sessions.map((s) => [s.id, s])),
          backfill: delta.backfill,
        };
      }
      const sessions = { ...state.sessions };
      for (const id of delta.removed) delete sessions[id];
      for (const s of delta.upserts) sessions[s.id] = s;
      return { sessions, backfill: delta.backfill ?? state.backfill };
    }),
}));

/** Latencia transcript → pintado. Solo cuando la app se lanza para medir B4. */
let perfEnabled = false;

function markPainted(delta: LiveDelta) {
  if (!perfEnabled || delta.type !== "update" || delta.upserts.length === 0) return;
  const eventTs = Math.max(...delta.upserts.map((s) => s.last_seen_at));
  requestAnimationFrame(() => {
    void invoke("perf_mark", { eventTsMs: eventTs, paintedTsMs: Date.now() });
  });
}

let started = false;

/** Abre el stream una sola vez. Rust envía una instantánea y después los cambios. */
export function connectLive() {
  if (started) return;
  started = true;
  const channel = new Channel<LiveDelta>();
  channel.onmessage = (delta) => {
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
