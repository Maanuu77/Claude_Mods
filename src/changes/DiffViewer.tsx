import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import { formatInt } from "../live/format";
import type { FileDiff, FilePreview } from "../live/types";
import { Badge, Button, Loading, Notice } from "../ui/components";
import { Icon } from "../ui/icons";

type Loaded = { kind: "diff"; data: FileDiff } | { kind: "preview"; data: FilePreview } | { kind: "error"; message: string };

function lineClass(line: string, isDiff: boolean): string {
  if (!isDiff) return "";
  if (line.startsWith("@@")) return "d-hunk";
  if (line.startsWith("+++") || line.startsWith("---")) return "d-meta";
  if (line.startsWith("diff ") || line.startsWith("index ") || line.startsWith("new file")) return "d-meta";
  if (line.startsWith("+")) return "d-add";
  if (line.startsWith("-")) return "d-del";
  return "";
}

/**
 * Diff de un fichero respecto a `HEAD`, o vista previa si no está versionado.
 * Se pide al abrirlo y solo vive en memoria: nada se guarda (§6.5, §7.2).
 */
export function DiffViewer({
  rootId,
  path,
  untracked,
  onClose,
}: {
  rootId: number;
  path: string;
  untracked: boolean;
  onClose: () => void;
}) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const self = useRef<HTMLElement>(null);

  // El visor se abre debajo de la lista: se trae a la vista, sin animación, al abrirlo
  // y otra vez cuando llega el contenido (crece).
  useEffect(() => {
    self.current?.scrollIntoView({ block: "nearest" });
  }, [loaded]);

  useEffect(() => {
    let cancelled = false;
    const request = untracked
      ? invoke<FilePreview>("read_file_preview", { rootId, relPath: path }).then((data): Loaded => ({ kind: "preview", data }))
      : invoke<FileDiff>("git_diff", { rootId, relPath: path }).then((data): Loaded => ({ kind: "diff", data }));
    request
      .catch((e: unknown): Loaded => ({ kind: "error", message: String(e) }))
      .then((result) => {
        if (!cancelled) setLoaded(result);
      });
    return () => {
      cancelled = true;
    };
  }, [rootId, path, untracked]);

  let body;
  if (!loaded) {
    body = <Loading label="Leyendo el fichero…" />;
  } else if (loaded.kind === "error") {
    body = (
      <Notice tone="critical" title="No se ha podido leer">
        {loaded.message}
      </Notice>
    );
  } else if (loaded.data.binary) {
    body = <p className="dv-note">Fichero binario: no se muestra su contenido.</p>;
  } else if (loaded.data.text.trim() === "") {
    body = <p className="dv-note">{loaded.kind === "diff" ? "Sin diferencias con HEAD." : "Fichero vacío."}</p>;
  } else {
    const isDiff = loaded.kind === "diff";
    const lines = loaded.data.text.replace(/\n$/, "").split("\n");
    body = (
      <>
        <pre className="dv-code">
          {lines.map((line, i) => (
            <span key={i} className={lineClass(line, isDiff)}>
              {line}
              {"\n"}
            </span>
          ))}
        </pre>
        {loaded.data.truncated && (
          <p className="dv-note">
            {isDiff
              ? "Diff recortado a 2 MB."
              : `Vista previa recortada: primeros 256 KB de ${formatInt(Math.round((loaded.data as FilePreview).size / 1024))} KB.`}
          </p>
        )}
      </>
    );
  }

  return (
    <section ref={self} className="dv ui-well" aria-label={`Contenido de ${path}`}>
      <header className="dv-head">
        <Icon name="file" size={14} />
        <span className="dv-path" title={path}>
          {path}
        </span>
        <Badge tone={untracked ? "positive" : "neutral"}>{untracked ? "sin seguimiento · vista previa" : "diff con HEAD"}</Badge>
        <Button size="sm" variant="ghost" icon="x-circle" iconOnly onClick={onClose}>
          Cerrar el visor
        </Button>
      </header>
      <div className="dv-body">{body}</div>
    </section>
  );
}
