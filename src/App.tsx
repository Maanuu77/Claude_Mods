import { useEffect, useMemo, useRef, useState } from "react";
import "./ui/tokens.css";
import "./ui/base.css";
import "./ui/components.css";
import "./app/shell.css";
import { Sidebar, type Tab } from "./app/Shell";
import { ChangesView } from "./changes/ChangesView";
import { DiagnosticsView } from "./diag/DiagnosticsView";
import { HistoryView } from "./history/HistoryView";
import { LiveView } from "./live/LiveView";
import { connectLive, sortSessions, useLive } from "./live/store";
import { ProjectMapView } from "./map/ProjectMapView";
import { TaskMonitor } from "./tasks/TaskMonitor";

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
  const { sessions, workspaces } = useLive();
  useEffect(connectLive, []);
  const [tab, setTab] = useState<Tab>("live");
  const main = useRef<HTMLElement>(null);
  // Las vistas comparten el contenedor con desplazamiento: al cambiar, se vuelve arriba.
  useEffect(() => {
    main.current?.scrollTo(0, 0);
  }, [tab]);

  const ordered = useMemo(() => sortSessions(sessions), [sessions]);
  const roots = Object.keys(workspaces).length;
  const now = useNow(ordered.length > 0 || roots > 0);
  const liveCount = ordered.filter((s) => s.live !== null).length;

  return (
    <div className="sh">
      <Sidebar
        tab={tab}
        onTab={setTab}
        counts={{ live: liveCount, changes: roots }}
        liveCount={liveCount}
        recent={ordered.length}
      />
      <main ref={main} className="sh-main">
        {tab === "live" ? (
          <LiveView sessions={ordered} now={now} />
        ) : tab === "tasks" ? (
          <TaskMonitor now={now} />
        ) : tab === "changes" ? (
          <ChangesView now={now} />
        ) : tab === "map" ? (
          <ProjectMapView now={now} />
        ) : tab === "history" ? (
          <HistoryView now={now} />
        ) : (
          <DiagnosticsView now={now} />
        )}
      </main>
    </div>
  );
}

export default App;
