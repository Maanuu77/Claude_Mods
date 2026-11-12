import { useMemo } from "react";
import { NODE_HEIGHT, layeredLayout, type Point } from "./layout";
import type { Component, Dependency } from "./types";

const PAD = 16;

const num = (v: number) => Math.round(v * 10) / 10;

/** Tramos entre capas como curvas verticales; el paso por un nodo virtual, recto. */
function pathOf(points: Point[]): string {
  let d = `M${num(points[0].x)},${num(points[0].y)}`;
  for (let i = 1; i < points.length; i += 1) {
    const a = points[i - 1];
    const b = points[i];
    if (i % 2 === 1) {
      const mid = num((a.y + b.y) / 2);
      d += ` C${num(a.x)},${mid} ${num(b.x)},${mid} ${num(b.x)},${num(b.y)}`;
    } else {
      d += ` L${num(b.x)},${num(b.y)}`;
    }
  }
  return d;
}

/** Ancho aproximado de la caja según el nombre (sin medir el DOM). */
function boxWidth(c: Component): number {
  const meta = `${c.ecosystem}${c.version ? ` · v${c.version}` : ""}`;
  return Math.min(240, Math.max(96, Math.max(c.name.length * 7.2, meta.length * 6) + 24));
}

function truncate(text: string, width: number, charWidth: number): string {
  const max = Math.floor((width - 16) / charWidth);
  return text.length > max ? `${text.slice(0, Math.max(1, max - 1))}…` : text;
}

/**
 * Grafo de componentes (SPEC §9, G8): quien depende arriba, aquello de lo que
 * depende abajo; los grupos (workspaces, soluciones) encima de sus miembros.
 * Línea continua = depende de; discontinua = contiene. La lista de la derecha es
 * su alternativa en texto.
 */
export function ComponentGraph({ components, deps }: { components: Component[]; deps: Dependency[] }) {
  const graph = useMemo(() => {
    const byId = new Map(components.map((c) => [c.id, c]));
    // Una arista por par: «depende de» gana a «contiene».
    const edges = new Map<string, { from: string; to: string; module: boolean }>();
    for (const d of deps) {
      if (!d.to || !byId.has(d.from) || !byId.has(d.to) || d.from === d.to) continue;
      const key = `${d.from}\u0000${d.to}`;
      const module = d.scope === "module";
      const prev = edges.get(key);
      edges.set(key, { from: d.from, to: d.to, module: prev ? prev.module && module : module });
    }
    const list = [...edges.values()];
    const layout = layeredLayout(
      components.map((c) => ({ id: c.id, width: boxWidth(c) })),
      list,
    );
    return { byId, edges: list, layout };
  }, [components, deps]);

  const { byId, edges, layout } = graph;
  if (edges.length === 0) return null;
  const w = layout.width + PAD * 2;
  const h = layout.height + PAD * 2;

  return (
    <figure className="pm-graph">
      <figcaption>
        Grafo de componentes
        <span className="pm-graph-key">
          <svg width="22" height="8" aria-hidden="true">
            <line x1="0" y1="4" x2="22" y2="4" className="pm-edge" />
          </svg>
          depende de
          <svg width="22" height="8" aria-hidden="true">
            <line x1="0" y1="4" x2="22" y2="4" className="pm-edge pm-edge-module" />
          </svg>
          contiene
        </span>
      </figcaption>
      <div className="pm-graph-scroll">
        <svg
          width={w}
          height={h}
          viewBox={`${-PAD} ${-PAD} ${w} ${h}`}
          role="img"
          aria-label={`Grafo de ${components.length} componentes y ${edges.length} relaciones internas`}
        >
          <defs>
            <marker id="pm-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6" markerHeight="6" orient="auto">
              <path d="M0,0 L8,4 L0,8 z" className="pm-arrow" />
            </marker>
          </defs>
          {edges.map((e, i) => {
            const route = layout.routes[i];
            if (!route) return null;
            return (
              <path
                key={`${e.from}-${e.to}`}
                d={pathOf(route)}
                className={e.module ? "pm-edge pm-edge-module" : "pm-edge"}
                markerEnd={e.module ? undefined : "url(#pm-arrow)"}
              >
                <title>{`${byId.get(e.from)!.name} ${e.module ? "contiene" : "depende de"} ${byId.get(e.to)!.name}`}</title>
              </path>
            );
          })}
          {[...layout.nodes.values()].map((n) => {
            const c = byId.get(n.id)!;
            const meta = `${c.ecosystem}${c.version ? ` · v${c.version}` : ""}`;
            return (
              <g key={n.id} transform={`translate(${n.x},${n.y})`}>
                <title>{`${c.name} (${c.ecosystem}) · ${c.rel_dir || "."}`}</title>
                <rect width={n.width} height={NODE_HEIGHT} rx={6} className={c.is_group ? "pm-node pm-node-group" : "pm-node"} />
                <text x={8} y={14} className="pm-node-name">
                  {truncate(c.name, n.width, 7.2)}
                </text>
                <text x={8} y={27} className="pm-node-meta">
                  {truncate(meta, n.width, 6)}
                </text>
              </g>
            );
          })}
        </svg>
      </div>
    </figure>
  );
}
