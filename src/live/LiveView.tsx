import { LiveEmpty, View } from "../app/Shell";
import { formatInt } from "./format";
import { SessionCard } from "./SessionCard";
import { useLive } from "./store";
import type { SessionView } from "./types";
import "./live.css";

/**
 * Vista «En vivo» (SPEC §6.5), el puente de mando: la sesión viva más reciente en el
 * plano cercano y las demás a su lado; las que ya no tienen proceso, en el lejano.
 */
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
    // Las sesiones llegan ordenadas: primero las vivas, por actividad más reciente.
    const [first, ...rest] = sessions;
    const hero = first?.live ? first : null;
    const others = hero ? rest : sessions;
    body = (
      <div className="lv-stage">
        {hero && <SessionCard key={hero.id} session={hero} now={now} hero />}
        {others.length > 0 && (
          <div className={hero ? "lv-side" : "grid-cards lv-all"}>
            {others.map((s) => (
              <SessionCard key={s.id} session={s} now={now} />
            ))}
          </div>
        )}
      </div>
    );
  }
  return (
    <View room="live" title="En vivo" summary={summary}>
      {body}
    </View>
  );
}
