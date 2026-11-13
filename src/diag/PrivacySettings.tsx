import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { formatAgo, formatInt } from "../live/format";
import { useLive } from "../live/store";
import { Button, Card, ConfirmBar, Loading, Notice, NumberField, SectionLabel, Switch } from "../ui/components";

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
      <Card title="Privacidad y retención" className="grid-wide">
        {failure ? (
          <Notice tone="critical" title="No se han podido leer los ajustes">
            {failure}
          </Notice>
        ) : (
          <Loading label="Leyendo los ajustes…" />
        )}
      </Card>
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
  const dirty = !same(draft, view.saved);
  return (
    <Card
      title="Privacidad y retención"
      subtitle="Qué se guarda y cuánto tiempo (SPEC §10). Los cambios se aplican al reiniciar la app."
      className="grid-wide"
    >
      {failure && (
        <Notice tone="critical" title="No se ha podido completar la acción">
          {failure}
        </Notice>
      )}
      <div className="pv-grid">
        {FLAGS.map((f) => (
          <Switch
            key={f.key.join(".")}
            checked={flag(draft, f.key)}
            onChange={(v) => setDraft(withFlag(draft, f.key, v))}
            label={f.label}
            help={f.help}
          />
        ))}
        <NumberField
          value={draft.retention.path_detail_days}
          min={1}
          max={30}
          unit="días"
          onChange={(v) => setDraft({ ...draft, retention: { path_detail_days: v } })}
          label="Días con rutas"
          help="De 1 a 30. Después solo quedan los totales por día, sin rutas."
        />
      </div>
      <div className="pv-actions">
        <Button variant="primary" onClick={save} disabled={busy || !dirty}>
          Guardar
        </Button>
        {dirty && (
          <Button variant="ghost" onClick={() => setDraft(view.saved)} disabled={busy}>
            Descartar cambios
          </Button>
        )}
      </div>
      {pendingRestart && (
        <Notice
          tone="attention"
          title="Hay ajustes guardados sin aplicar"
          actions={
            <Button size="sm" variant="primary" icon="refresh" onClick={() => void invoke("restart_app")} disabled={busy}>
              Reiniciar ahora
            </Button>
          }
        >
          Se aplican al reiniciar DEV COCKPIT; cada mod adopta la política nueva en su siguiente conexión.
        </Notice>
      )}

      <SectionLabel>Borrar rutas ahora</SectionLabel>
      <p className="pv-text">
        Borra ya el detalle de rutas de todos los días agregados, quita la ruta de todas las herramientas y borra las
        incidencias de la ingesta. Los totales por día se conservan. Las rutas de los cambios de hoy se borran en cuanto
        el día se agregue, porque hacen falta para contar los ficheros distintos.
        {view.applied.privacy.record_paths && " Con «Rutas de fichero» activado, las rutas nuevas se siguen guardando."}
        {view.paths_cleared_at && <span className="muted"> Última vez: {formatAgo(view.paths_cleared_at, now)}.</span>}
      </p>
      {confirming ? (
        <ConfirmBar
          message="No se puede deshacer. ¿Borrar las rutas?"
          confirmLabel="Borrar las rutas"
          onConfirm={clear}
          onCancel={() => setConfirming(false)}
          busy={busy}
        />
      ) : (
        <div className="pv-actions">
          <Button variant="danger" icon="trash" onClick={() => setConfirming(true)} disabled={busy}>
            Borrar rutas ahora
          </Button>
        </div>
      )}
      {report && (
        <Notice tone="positive" live title="Rutas borradas">
          {formatInt(report.compact.tool_paths_purged + report.tool_paths_cleared)} herramientas sin ruta,{" "}
          {formatInt(report.compact.file_events_purged)} cambios de fichero borrados y{" "}
          {formatInt(report.compact.errors_purged)} incidencias borradas.
          {report.file_events_pending > 0 &&
            ` ${formatInt(report.file_events_pending)} cambios conservan la ruta hasta que su día se agregue.`}
          {report.compact.failed > 0 && ` ${formatInt(report.compact.failed)} días no se pudieron agregar y siguen pendientes.`}
        </Notice>
      )}
    </Card>
  );
}
