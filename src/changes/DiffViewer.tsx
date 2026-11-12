import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { formatInt } from "../live/format";
import type { FileDiff, FilePreview } from "../live/types";

type Loaded =
  | { kind: "diff"; data: FileDiff }
  | { kind: "preview"; data: FilePreview }
  | { kind: "error"; message: string };

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
}: {
  rootId: number;
  path: string;
  untracked: boolean;
}) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);

  useEffect(() => {
    let cancelled = false;
    const request = untracked
      ? invoke<FilePreview>("read_file_preview", { rootId, relPath: path }).then(
          (data): Loaded => ({ kind: "preview", data }),
        )
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
    body = <p className="viewer-note">Cargando…</p>;
  } else if (loaded.kind === "error") {
    body = <p className="viewer-note warn">No se ha podido leer: {loaded.message}</p>;
  } else if (loaded.data.binary) {
    body = <p className="viewer-note">Fichero binario: no se muestra su contenido.</p>;
  } else if (loaded.data.text.trim() === "") {
    body = (
      <p className="viewer-note">
        {loaded.kind === "diff" ? "Sin diferencias con HEAD." : "Fichero vacío."}
      </p>
    );
  } else {
    const isDiff = loaded.kind === "diff";
    const lines = loaded.data.text.replace(/\n$/, "").split("\n");
    body = (
      <>
        <pre className="viewer-code">
          {lines.map((line, i) => (
            <span key={i} className={lineClass(line, isDiff)}>
              {line}
              {"\n"}
            </span>
          ))}
        </pre>
        {loaded.data.truncated && (
          <p className="viewer-note">
            {isDiff
              ? "Diff recortado a 2 MB."
              : `Vista previa recortada: primeros 256 KB de ${formatInt(Math.round((loaded.data as FilePreview).size / 1024))} KB.`}
          </p>
        )}
      </>
    );
  }

  return (
    <section className="viewer" aria-label={`Contenido de ${path}`}>
      <header className="viewer-head">
        <span className="viewer-path" title={path}>
          {path}
        </span>
        <span className="tag">{untracked ? "sin seguimiento · vista previa" : "diff con HEAD"}</span>
      </header>
      {body}
    </section>
  );
}
