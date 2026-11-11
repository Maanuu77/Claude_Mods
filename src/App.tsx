import { useEffect, useMemo, useState } from "react";
import "./App.css";
import { SessionCard } from "./live/SessionCard";
import { formatInt } from "./live/format";
import { connectLive, sortSessions, useLive } from "./live/store";

/** Reloj local para los tiempos transcurridos. Se para con la ventana oculta. */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const tick = () => setNow(Date.now());
    const id = window.setInterval(tick, 1000);
    const onVisible = () => {
      if (!document.hidden) tick();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [active]);
  return now;
}

function App() {
  const { connected, error, sessions, backfill } = useLive();
  useEffect(connectLive, []);

  const ordered = useMemo(() => sortSessions(sessions), [sessions]);
  const now = useNow(ordered.length > 0);
  const liveCount = ordered.filter((s) => s.live !== null).length;
  const percent =
    backfill && backfill.bytes_total > 0
      ? Math.round((backfill.bytes_done / backfill.bytes_total) * 100)
      : 0;

  return (
    <main className="app">
      <header className="top">
        <h1>DEV COCKPIT</h1>
        <p className="summary">
          {connected
            ? `${formatInt(liveCount)} ${liveCount === 1 ? "sesión viva" : "sesiones vivas"} · ${formatInt(ordered.length)} con actividad reciente`
            : "Conectando…"}
        </p>
      </header>

      {error && (
        <p className="banner banner-error" role="alert">
          No se ha podido conectar con el núcleo: {error}
        </p>
      )}

      {backfill?.running && (
        <p className="banner" role="status">
          Leyendo el historial de transcripts: {formatInt(backfill.files_done)} de{" "}
          {formatInt(backfill.files_total)} ficheros ({percent} %)
          <progress value={backfill.bytes_done} max={Math.max(backfill.bytes_total, 1)} />
        </p>
      )}

      {connected && ordered.length === 0 && !backfill?.running ? (
        <p className="empty">
          No hay sesiones de Claude Code activas. Aparecerán aquí en cuanto se abra una.
        </p>
      ) : (
        <div className="grid">
          {ordered.map((s) => (
            <SessionCard key={s.id} session={s} now={now} />
          ))}
        </div>
      )}
    </main>
  );
}

export default App;
