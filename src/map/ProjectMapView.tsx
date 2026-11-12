import { invoke } from "@tauri-apps/api/core";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useMemo, useRef, useState } from "react";
import { formatInt } from "../live/format";
import { useLive } from "../live/store";
import type { FileEventView, WorkspaceView } from "../live/types";
import { ComponentGraph } from "./ComponentGraph";
import type { Component, Dependency, DepScope, DirSummary, Ecosystem, ProjectScan } from "./types";

const ECOSYSTEM_LABEL: Record<Ecosystem, string> = {
  node: "Node",
  rust: "Rust",
  java: "Java",
  python: "Python",
  dotnet: ".NET",
  unity: "Unity",
  rojo: "Roblox (Rojo)",
};

const SCOPE_LABEL: Record<DepScope, string> = {
  prod: "producción",
  dev: "desarrollo",
  build: "compilación",
  peer: "peer",
  optional: "opcional",
  module: "módulo",
};

const PROBLEM_LABEL: Record<string, string> = {
  invalid_json: "JSON no válido",
  invalid_toml: "TOML no válido",
  invalid_xml: "XML no válido",
  too_large: "demasiado grande",
};

function formatBytes(n: number): string {
  if (n < 1024) return `${formatInt(n)} B`;
  const units = ["KiB", "MiB", "GiB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toLocaleString("es-ES", { maximumFractionDigits: v < 10 ? 1 : 0 })} ${units[i]}`;
}

function startOfToday(now: number): number {
  const d = new Date(now);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** Cambios de hoy por directorio (los suyos y los de debajo): la capa de calor de §9. */
function heatByDir(events: FileEventView[], since: number): Map<string, number> {
  const heat = new Map<string, number>();
  for (const e of events) {
    if (e.ts < since || e.rel_path === null) continue;
    const parts = e.rel_path.split("/");
    parts.pop();
    for (let i = parts.length; i >= 0; i -= 1) {
      const dir = parts.slice(0, i).join("/");
      heat.set(dir, (heat.get(dir) ?? 0) + 1);
    }
  }
  return heat;
}

// --- Árbol --------------------------------------------------------------------

interface TreeNode {
  dir: DirSummary;
  name: string;
  children: TreeNode[];
}

/** Árbol de directorios a partir de la lista plana (ordenada por segmentos). */
function buildTree(dirs: DirSummary[], rootName: string): TreeNode | null {
  const byPath = new Map<string, TreeNode>();
  const sorted = [...dirs].sort((a, b) => {
    const x = a.rel_dir.split("/");
    const y = b.rel_dir.split("/");
    for (let i = 0; i < Math.min(x.length, y.length); i += 1) {
      const c = x[i].localeCompare(y[i], "es", { sensitivity: "base" });
      if (c !== 0) return c;
    }
    return x.length - y.length;
  });
  for (const dir of sorted) {
    const name = dir.rel_dir === "" ? rootName : dir.rel_dir.slice(dir.rel_dir.lastIndexOf("/") + 1);
    const node: TreeNode = { dir, name, children: [] };
    byPath.set(dir.rel_dir, node);
    if (dir.rel_dir !== "") {
      const slash = dir.rel_dir.lastIndexOf("/");
      const parent = byPath.get(slash < 0 ? "" : dir.rel_dir.slice(0, slash));
      parent?.children.push(node);
    }
  }
  return byPath.get("") ?? null;
}

function visibleRows(root: TreeNode, open: Set<string>): TreeNode[] {
  const rows: TreeNode[] = [];
  const walk = (node: TreeNode) => {
    rows.push(node);
    if (open.has(node.dir.rel_dir)) node.children.forEach(walk);
  };
  walk(root);
  return rows;
}

function Tree({ root, heat }: { root: TreeNode; heat: Map<string, number> }) {
  const [open, setOpen] = useState<Set<string>>(() => new Set([""]));
  const rows = useMemo(() => visibleRows(root, open), [root, open]);
  const parentRef = useRef<HTMLDivElement>(null);
  // eslint-disable-next-line react-hooks/incompatible-library -- TanStack Virtual no es memoizable
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 28,
    overscan: 12,
  });
  const toggle = (path: string) =>
    setOpen((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  return (
    <div ref={parentRef} className="pm-tree" role="tree" aria-label="Estructura del proyecto">
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((item) => {
          const node = rows[item.index];
          const path = node.dir.rel_dir;
          const expandable = node.children.length > 0;
          const changes = heat.get(path) ?? 0;
          const top = node.dir.languages[0];
          return (
            <div
              key={path || "."}
              role="treeitem"
              aria-level={node.dir.depth + 1}
              aria-expanded={expandable ? open.has(path) : undefined}
              className="pm-row"
              style={{ transform: `translateY(${item.start}px)`, paddingLeft: 8 + node.dir.depth * 16 }}
            >
              <button
                type="button"
                className="pm-toggle"
                onClick={() => toggle(path)}
                disabled={!expandable}
                aria-label={expandable ? (open.has(path) ? `Plegar ${node.name}` : `Desplegar ${node.name}`) : undefined}
              >
                {expandable ? (open.has(path) ? "▾" : "▸") : ""}
              </button>
              <span className="pm-name" title={path || node.name}>
                {node.name}
              </span>
              {top && <span className="tag">{top.language}</span>}
              {changes > 0 && (
                <span className="tag tag-wait" title="Cambios de hoy en este directorio y por debajo">
                  {formatInt(changes)} hoy
                </span>
              )}
              <span className="pm-num">{formatInt(node.dir.files)}</span>
              <span className="pm-num pm-bytes">{formatBytes(node.dir.bytes)}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

// --- Componentes --------------------------------------------------------------

function ComponentCard({
  component,
  deps,
  usedBy,
  names,
}: {
  component: Component;
  deps: Dependency[];
  usedBy: Dependency[];
  names: Map<string, string>;
}) {
  const internal = deps.filter((d) => d.to !== null && d.scope !== "module");
  const members = deps.filter((d) => d.scope === "module");
  const external = deps.filter((d) => d.to === null && d.scope !== "module");
  const byScope = new Map<DepScope, Dependency[]>();
  for (const d of external) byScope.set(d.scope, [...(byScope.get(d.scope) ?? []), d]);
  const name = (id: string | null) => (id ? (names.get(id) ?? id) : "");

  return (
    <li className="pm-comp">
      <div className="pm-comp-head">
        <strong>{component.name}</strong>
        {component.is_group && <span className="tag">grupo</span>}
        {component.version && <span className="pm-version">v{component.version}</span>}
        <span className="pm-dir" title={component.manifest}>
          {component.rel_dir || "."}
        </span>
      </div>
      {component.parts.length > 0 && <p className="pm-line">Servicios: {component.parts.join(", ")}</p>}
      {members.length > 0 && <p className="pm-line">Contiene: {members.map((d) => name(d.to)).join(", ")}</p>}
      {internal.length > 0 && (
        <p className="pm-line">Depende de: {[...new Set(internal.map((d) => name(d.to)))].join(", ")}</p>
      )}
      {usedBy.length > 0 && (
        <p className="pm-line">Lo usan: {[...new Set(usedBy.map((d) => name(d.from)))].join(", ")}</p>
      )}
      {external.length > 0 && (
        <details className="pm-ext">
          <summary>
            {formatInt(external.length)} dependencias externas (
            {[...byScope.entries()].map(([s, list]) => `${formatInt(list.length)} ${SCOPE_LABEL[s]}`).join(", ")})
          </summary>
          <ul>
            {external.map((d) => (
              <li key={`${d.scope}-${d.name}`}>
                <span>{d.name}</span>
                {d.version_req && <span className="pm-version">{d.version_req}</span>}
                {d.scope !== "prod" && <span className="tag">{SCOPE_LABEL[d.scope]}</span>}
              </li>
            ))}
          </ul>
        </details>
      )}
    </li>
  );
}

function Components({ scan }: { scan: ProjectScan }) {
  const { components, deps } = scan.map;
  const names = useMemo(() => new Map(components.map((c) => [c.id, c.name])), [components]);
  const groups = useMemo(() => {
    const out = new Map<Ecosystem, Component[]>();
    for (const c of components) out.set(c.ecosystem, [...(out.get(c.ecosystem) ?? []), c]);
    return [...out.entries()];
  }, [components]);
  if (components.length === 0) {
    return <p className="card-meta">No hay manifiestos reconocidos (SPEC §9) en esta raíz.</p>;
  }
  return (
    <>
      {groups.map(([eco, list]) => (
        <section key={eco} className="pm-eco">
          <h3>
            {ECOSYSTEM_LABEL[eco]} · {formatInt(list.length)}
          </h3>
          <ul className="pm-comps">
            {list.map((c) => (
              <ComponentCard
                key={c.id}
                component={c}
                deps={deps.filter((d) => d.from === c.id)}
                usedBy={deps.filter((d) => d.to === c.id && d.scope !== "module")}
                names={names}
              />
            ))}
          </ul>
        </section>
      ))}
    </>
  );
}

// --- Panel por raíz -----------------------------------------------------------

type Load = { state: "loading" } | { state: "ready"; scan: ProjectScan } | { state: "error"; message: string };

function MapPanel({ ws, now }: { ws: WorkspaceView; now: number }) {
  const [load, setLoad] = useState<Load>({ state: "loading" });
  const [events, setEvents] = useState<FileEventView[]>([]);
  const [round, setRound] = useState(0);

  useEffect(() => {
    let alive = true;
    invoke<ProjectScan>("project_map", { rootId: ws.root_id })
      .then((scan) => alive && setLoad({ state: "ready", scan }))
      .catch((e: unknown) => alive && setLoad({ state: "error", message: String(e) }));
    invoke<FileEventView[]>("query_file_events", { rootId: ws.root_id, limit: 500 })
      .then((list) => alive && setEvents(list))
      .catch(() => alive && setEvents([]));
    return () => {
      alive = false;
    };
  }, [ws.root_id, round]);

  const today = startOfToday(now);
  const heat = useMemo(() => heatByDir(events, today), [events, today]);
  const tree = useMemo(
    () => (load.state === "ready" ? buildTree(load.scan.tree.dirs, ws.name) : null),
    [load, ws.name],
  );

  const refresh = () => {
    setLoad({ state: "loading" });
    setRound((r) => r + 1);
  };

  return (
    <div className="root-panel">
      <header className="root-head pm-head">
        <h2 title={ws.root}>{ws.name}</h2>
        {load.state === "ready" && (
          <span className="card-meta">
            {formatInt(load.scan.tree.files)} ficheros · {formatBytes(load.scan.tree.bytes)} ·{" "}
            {load.scan.source === "git" ? "versionados en Git" : "sin Git, según .gitignore"}
          </span>
        )}
        <button type="button" className="tab" onClick={refresh} disabled={load.state === "loading"}>
          Actualizar
        </button>
      </header>

      {load.state === "loading" && <p className="card-meta">Leyendo la estructura y los manifiestos…</p>}
      {load.state === "error" && (
        <p className="banner banner-error" role="alert">
          No se ha podido leer el proyecto: {load.message}
        </p>
      )}
      {load.state === "ready" && (
        <>
          {load.scan.tree.truncated && (
            <p className="banner" role="status">
              El proyecto tiene más de 50.000 ficheros: solo se cuentan los primeros.
            </p>
          )}
          <div className="changes-body">
            <section>
              <h3>Estructura</h3>
              {tree ? <Tree root={tree} heat={heat} /> : <p className="card-meta">Sin ficheros.</p>}
            </section>
            <section>
              <h3>Componentes</h3>
              <Components scan={load.scan} />
              {load.scan.map.problems.length > 0 && (
                <section className="pm-eco">
                  <h3>Manifiestos que no se han podido leer</h3>
                  <ul className="pm-comps">
                    {load.scan.map.problems.map((p) => (
                      <li key={p.manifest} className="pm-line">
                        {p.manifest}: {PROBLEM_LABEL[p.reason] ?? p.reason}
                      </li>
                    ))}
                  </ul>
                </section>
              )}
            </section>
          </div>
          <ComponentGraph components={load.scan.map.components} deps={load.scan.map.deps} />
        </>
      )}
    </div>
  );
}

/**
 * Vista «Mapa» (SPEC §9): estructura de cada proyecto vigilado y los componentes
 * que declaran sus manifiestos. Nunca muestra contenido de ficheros.
 */
export function ProjectMapView({ now }: { now: number }) {
  const workspaces = useLive((s) => s.workspaces);
  const list = useMemo(
    () =>
      Object.values(workspaces).sort(
        (a, b) => b.sessions.length - a.sessions.length || a.name.localeCompare(b.name),
      ),
    [workspaces],
  );
  const [chosen, setChosen] = useState<number | null>(null);
  const current = list.find((w) => w.root_id === chosen) ?? list[0];

  if (!current) {
    return (
      <p className="empty">
        No hay proyectos vigilados. El mapa se calcula para la raíz de Git de cada sesión viva.
      </p>
    );
  }
  return (
    <section className="changes">
      <nav className="roots" aria-label="Proyectos vigilados">
        {list.map((w) => (
          <button
            key={w.root_id}
            type="button"
            className={`root-chip${w.root_id === current.root_id ? " root-chip-on" : ""}`}
            onClick={() => setChosen(w.root_id)}
            title={w.root}
          >
            <strong>{w.name}</strong>
          </button>
        ))}
      </nav>
      <MapPanel key={current.root_id} ws={current} now={now} />
    </section>
  );
}
