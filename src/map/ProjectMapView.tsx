import { invoke } from "@tauri-apps/api/core";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useMemo, useRef, useState } from "react";
import { LiveEmpty, View } from "../app/Shell";
import { formatInt } from "../live/format";
import { useLive } from "../live/store";
import type { FileEventView, WorkspaceView } from "../live/types";
import { Badge, Button, Card, EmptyState, Loading, Notice, SectionLabel, Segmented } from "../ui/components";
import { Icon } from "../ui/icons";
import { ComponentGraph } from "./ComponentGraph";
import "./map.css";
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
    <div ref={parentRef} className="pm-tree ui-well" role="tree" aria-label="Estructura del proyecto">
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
                {expandable && <Icon name={open.has(path) ? "chevron-down" : "chevron-right"} size={14} />}
              </button>
              <Icon name="folder" size={14} className="pm-folder" />
              <span className="pm-name" title={path || node.name}>
                {node.name}
              </span>
              {top && <span className="pm-lang">{top.language}</span>}
              {changes > 0 && (
                <Badge tone="attention" title="Cambios de hoy en este directorio y por debajo">
                  {formatInt(changes)} hoy
                </Badge>
              )}
              <span className="pm-num num">{formatInt(node.dir.files)}</span>
              <span className="pm-num pm-bytes num">{formatBytes(node.dir.bytes)}</span>
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
        <strong className="truncate">{component.name}</strong>
        {component.is_group && <Badge tone="accent">grupo</Badge>}
        {component.version && <span className="pm-version mono">v{component.version}</span>}
        <span className="pm-dir path" title={component.manifest}>
          {component.rel_dir || "raíz"}
        </span>
      </div>
      {component.parts.length > 0 && (
        <p className="pm-line">
          <span className="pm-rel">Servicios</span> {component.parts.join(", ")}
        </p>
      )}
      {members.length > 0 && (
        <p className="pm-line">
          <span className="pm-rel">Contiene</span> {members.map((d) => name(d.to)).join(", ")}
        </p>
      )}
      {internal.length > 0 && (
        <p className="pm-line">
          <span className="pm-rel">Depende de</span> {[...new Set(internal.map((d) => name(d.to)))].join(", ")}
        </p>
      )}
      {usedBy.length > 0 && (
        <p className="pm-line">
          <span className="pm-rel">Lo usan</span> {[...new Set(usedBy.map((d) => name(d.from)))].join(", ")}
        </p>
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
                <span className="truncate">{d.name}</span>
                {d.version_req && <span className="pm-version mono">{d.version_req}</span>}
                {d.scope !== "prod" && <Badge tone="neutral">{SCOPE_LABEL[d.scope]}</Badge>}
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
    return (
      <EmptyState icon="layers" title="Sin componentes" compact>
        No hay manifiestos reconocidos (package.json, Cargo.toml, pom.xml…) en esta raíz.
      </EmptyState>
    );
  }
  return (
    <>
      {groups.map(([eco, list]) => (
        <section key={eco} className="pm-eco">
          <SectionLabel count={list.length}>{ECOSYSTEM_LABEL[eco]}</SectionLabel>
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
    <Card
      className="pm-panel"
      title={ws.name}
      titleAttr={ws.root}
      subtitle={
        load.state === "ready" ? (
          <>
            <span className="num">{formatInt(load.scan.tree.files)}</span> ficheros ·{" "}
            <span className="num">{formatBytes(load.scan.tree.bytes)}</span> ·{" "}
            {load.scan.source === "git" ? "versionados en Git" : "sin Git, según .gitignore"}
          </>
        ) : (
          "Leyendo la estructura y los manifiestos…"
        )
      }
      actions={
        <>
          {load.state === "ready" && load.scan.map_cached && (
            <Badge
              tone="neutral"
              title="Ningún manifiesto ha cambiado desde el último escaneo: los componentes no se han vuelto a leer."
            >
              componentes guardados
            </Badge>
          )}
          <Button size="sm" icon="refresh" onClick={refresh} disabled={load.state === "loading"}>
            Actualizar
          </Button>
        </>
      }
    >
      {load.state === "loading" && <Loading label="Leyendo la estructura y los manifiestos…" />}
      {load.state === "error" && (
        <Notice
          tone="critical"
          title="No se ha podido leer el proyecto"
          actions={
            <Button size="sm" onClick={refresh}>
              Reintentar
            </Button>
          }
        >
          {load.message}
        </Notice>
      )}
      {load.state === "ready" && (
        <>
          {load.scan.tree.truncated && (
            <Notice tone="attention">El proyecto tiene más de 50.000 ficheros: solo se cuentan los primeros.</Notice>
          )}
          <div className="pm-body">
            <section className="pm-col">
              <SectionLabel count={load.scan.tree.dirs.length}>Estructura</SectionLabel>
              {tree ? (
                <Tree root={tree} heat={heat} />
              ) : (
                <div className="ui-well">
                  <EmptyState icon="folder" title="Sin ficheros" compact />
                </div>
              )}
            </section>
            <section className="pm-col">
              <SectionLabel count={load.scan.map.components.length}>Componentes</SectionLabel>
              <Components scan={load.scan} />
              {load.scan.map.problems.length > 0 && (
                <Notice tone="attention" title="Manifiestos que no se han podido leer">
                  <ul className="pm-problems">
                    {load.scan.map.problems.map((p) => (
                      <li key={p.manifest}>
                        <span className="mono">{p.manifest}</span>: {PROBLEM_LABEL[p.reason] ?? p.reason}
                      </li>
                    ))}
                  </ul>
                </Notice>
              )}
            </section>
          </div>
          <ComponentGraph components={load.scan.map.components} deps={load.scan.map.deps} />
        </>
      )}
    </Card>
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

  return (
    <View
      title="Mapa del proyecto"
      summary="Estructura, componentes y dependencias declaradas. Solo se leen los manifiestos, nunca el código."
      actions={
        list.length > 1 && current ? (
          <Segmented
            label="Proyectos vigilados"
            value={current.root_id}
            onChange={setChosen}
            options={list.map((w) => ({ value: w.root_id, label: w.name, title: w.root }))}
          />
        ) : undefined
      }
    >
      {current ? (
        <MapPanel key={current.root_id} ws={current} now={now} />
      ) : (
        <LiveEmpty icon="map" title="No hay proyectos vigilados">
          El mapa se calcula para la raíz de Git de cada sesión viva.
        </LiveEmpty>
      )}
    </View>
  );
}
