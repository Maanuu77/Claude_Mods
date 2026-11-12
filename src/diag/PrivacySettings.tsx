import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { formatAgo, formatInt } from "../live/format";
import { useLive } from "../live/store";

/** Espejo de `cockpit_ingest::settings::Settings` (SPEC §10). */
interface Settings {
  privacy: { capture_task_titles: boolean; capture_shell_program: boolean; record_paths: boolean };
  retention: { path_detail_days: number };
  adapters: { desktop_plan: boolean };
}

interface SettingsView {
  saved: Settings;
  applied: Settings;
  paths_cleared_at: number | null;
}

/** Espejo de `cockpit_store::ClearPathsReport`. */
interface ClearReport {
  compact: { aggregated: number; failed: number; tool_paths_purged: number; file_events_purged: number; errors_purged: number };
  tool_paths_cleared: number;
  file_events_pending: number;
}

type Flag =
  | ["privacy", "capture_task_titles"]
  | ["privacy", "capture_shell_program"]
  | ["privacy", "record_paths"]
  | ["adapters", "desktop_plan"];

const FLAGS: { key: Flag; label: string; help: string }[] = [
  {
    key: ["privacy", "record_paths"],
    label: "Rutas de fichero",
    help: "Rutas relativas de herramientas y cambios, carpeta de trabajo y host. Sin ellas no hay atribución por ruta.",
  },
  {
    key: ["privacy", "capture_task_titles"],
    label: "Títulos de tarea",
    help: "Hasta 120 caracteres. Sin ellos se ve «Tarea n.º k», y al aplicarlo se borran los ya guardados.",
  },
  {
    key: ["privacy", "capture_shell_program"],
    label: "Programa de cada Bash",
    help: "Solo el nombre del ejecutable, nunca el comando.",
  },
  {
    key: ["adapters", "desktop_plan"],
    label: "Plan del Desktop",
    help: "Lee update_plan, una herramienta del Desktop con un esquema no publicado.",
  },
];

const same = (a: Settings, b: Settings) => JSON.stringify(a) === JSON.stringify(b);

function flag(s: Settings, [group, name]: Flag): boolean {
  return (s[group] as Record<string, boolean>)[name];
}

function withFlag(s: Settings, [group, name]: Flag, value: boolean): Settings {
  return { ...s, [group]: { ...s[group], [name]: value } };
}

/**
 * Ajustes de privacidad y retención (SPEC §10) y «borrar rutas ahora» (§6.4.1).
 * Los ajustes se aplican al reiniciar la app: así cada mod adopta la política nueva
 * en su siguiente `hello` (§5.4).
 */
export function PrivacySettings({ now, onCleared }: { now: number; onCleared: () => void }) {
  const [view, setView] = useState<SettingsView | null>(null);
  const [draft, setDraft] = useState<Settings | null>(null);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [report, setReport] = useState<ClearReport | null>(null);

  useEffect(() => {
    invoke<SettingsView>("get_settings")
      .then((v) => {
        setView(v);
        setDraft(v.saved);
      })
      .catch((e: unknown) => setFailure(String(e)));
  }, []);

  if (!view || !draft) {
    return (
      <section className="card">
        <h3>Privacidad y retención</h3>
        <p className="card-meta">{failure ? `No se han podido leer los ajustes: ${failure}` : "Cargando…"}</p>
      </section>
    );
  }

  const save = () => {
    setBusy(true);
    invoke<Settings>("save_settings", { settings: draft })
      .then((saved) => {
        setView({ ...view, saved });
        setDraft(saved);
        setFailure(null);
      })
      .catch((e: unknown) => setFailure(String(e)))
      .finally(() => setBusy(false));
  };

  const clear = () => {
    setBusy(true);
    invoke<ClearReport>("clear_paths_now")
      .then((r) => {
        setReport(r);
        setConfirming(false);
        setView({ ...view, paths_cleared_at: Date.now() });
        // Los cambios en memoria de la interfaz se vuelven a pedir a la base.
        useLive.setState({ fileEvents: {} });
        onCleared();
      })
      .catch((e: unknown) => setFailure(String(e)))
      .finally(() => setBusy(false));
  };

  const pendingRestart = !same(view.saved, view.applied);
  return (
    <section className="card dg-wide">
      <h3>Privacidad y retención</h3>
      {failure && (
        <p className="banner banner-error" role="alert">
          {failure}
        </p>
      )}
      <div className="pv-grid">
        {FLAGS.map((f) => (
          <label key={f.key.join(".")} className="pv-flag">
            <input
              type="checkbox"
              checked={flag(draft, f.key)}
              onChange={(e) => setDraft(withFlag(draft, f.key, e.target.checked))}
            />
            <span>
              <strong>{f.label}</strong>
              <small>{f.help}</small>
            </span>
          </label>
        ))}
        <label className="pv-flag">
          <input
            type="number"
            min={1}
            max={30}
            value={draft.retention.path_detail_days}
            onChange={(e) =>
              setDraft({
                ...draft,
                retention: { path_detail_days: Math.min(30, Math.max(1, Number(e.target.value) || 1)) },
              })
            }
          />
          <span>
            <strong>Días con rutas</strong>
            <small>De 1 a 30. Después solo quedan los totales por día, sin rutas.</small>
          </span>
        </label>
      </div>
      <div className="pv-actions">
        <button type="button" className="tab" onClick={save} disabled={busy || same(draft, view.saved)}>
          Guardar
        </button>
        {pendingRestart && (
          <>
            <span className="card-meta">Los cambios guardados se aplican al reiniciar DEV COCKPIT.</span>
            <button type="button" className="tab" onClick={() => void invoke("restart_app")} disabled={busy}>
              Reiniciar ahora
            </button>
          </>
        )}
      </div>

      <h3>Borrar rutas ahora</h3>
      <p className="card-meta">
        Borra ya el detalle de rutas de todos los días agregados, quita la ruta de todas las herramientas y borra
        las incidencias de la ingesta. Los totales por día se conservan. Las rutas de los cambios de hoy se borran en
        cuanto el día se agregue, porque hacen falta para contar los ficheros distintos.
        {view.applied.privacy.record_paths && " Con «Rutas de fichero» activado, las rutas nuevas se siguen guardando."}
        {view.paths_cleared_at && ` Última vez: ${formatAgo(view.paths_cleared_at, now)}.`}
      </p>
      {confirming ? (
        <div className="pv-actions">
          <span className="warn">No se puede deshacer. ¿Borrar las rutas?</span>
          <button type="button" className="tab" onClick={clear} disabled={busy}>
            {busy ? "Borrando…" : "Borrar"}
          </button>
          <button type="button" className="tab" onClick={() => setConfirming(false)} disabled={busy}>
            Cancelar
          </button>
        </div>
      ) : (
        <div className="pv-actions">
          <button type="button" className="tab" onClick={() => setConfirming(true)} disabled={busy}>
            Borrar rutas ahora
          </button>
        </div>
      )}
      {report && (
        <p className="card-meta" role="status">
          Hecho: {formatInt(report.compact.tool_paths_purged + report.tool_paths_cleared)} herramientas sin ruta,{" "}
          {formatInt(report.compact.file_events_purged)} cambios de fichero borrados y{" "}
          {formatInt(report.compact.errors_purged)} incidencias borradas.
          {report.file_events_pending > 0 &&
            ` ${formatInt(report.file_events_pending)} cambios conservan la ruta hasta que su día se agregue.`}
          {report.compact.failed > 0 && ` ${formatInt(report.compact.failed)} días no se pudieron agregar y siguen pendientes.`}
        </p>
      )}
    </section>
  );
}
