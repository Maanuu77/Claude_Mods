import { LiveEmpty, View } from "../app/Shell";
import { formatInt } from "./format";
import { SessionCard } from "./SessionCard";
import { useLive } from "./store";
import type { SessionView } from "./types";
import "./live.css";

/** Vista «En vivo» (SPEC §6.5): una tarjeta por sesión viva o con actividad reciente. */
export function LiveView({ sessions, now }: { sessions: SessionView[]; now: number }) {
  const { connected, error, backfill } = useLive();
  const live = sessions.filter((s) => s.live !== null).length;
  const summary = connected
    ? `${formatInt(live)} ${live === 1 ? "sesión viva" : "sesiones vivas"} · ${formatInt(sessions.length)} con actividad reciente`
    : error
      ? "Sin conexión con el núcleo"
      : "Conectando con el núcleo…";

  let body;
  if (sessions.length === 0 && !backfill?.running) {
    body = (
      <LiveEmpty icon="activity" title="No hay sesiones de Claude Code activas">
        Aparecerán aquí en cuanto se abra una, en la terminal o en la app de escritorio.
      </LiveEmpty>
    );
  } else {
    body = (
      <div className="grid-cards">
        {sessions.map((s) => (
          <SessionCard key={s.id} session={s} now={now} />
        ))}
      </div>
    );
  }
  return (
    <View title="En vivo" summary={summary}>
      {body}
    </View>
  );
}
