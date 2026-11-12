// G8 (SPEC §13.2): ¿el grafo de componentes rinde con 200 nodos o más?
//
//   node --experimental-strip-types --no-warnings scripts/measure/g8-layout.ts
//
// Grafos aleatorios (semilla fija) de 50, 200 y 500 nodos con unas 2 aristas por
// nodo y un 5 % de aristas que cierran ciclos. Mide el layout (mediana de 20) y
// comprueba que:
//   - ninguna caja se solapa con otra de su capa;
//   - toda arista tiene ruta y la ruta empieza y acaba en el borde de sus cajas;
//   - ningún tramo de una ruta atraviesa una caja (dentro de una capa, la ruta
//     pasa por su propio hueco; entre capas no hay cajas).
import { NODE_HEIGHT, layeredLayout, type LayoutEdge, type LayoutNode } from "../../src/map/layout.ts";

let seed = 42;
const rand = () => {
  seed = (seed * 1103515245 + 12345) % 2147483648;
  return seed / 2147483648;
};

for (const n of [50, 200, 500]) {
  seed = 42; // los mismos grafos que scripts/measure/g8-render.mjs
  const nodes: LayoutNode[] = Array.from({ length: n }, (_, i) => ({ id: `c${i}`, width: 80 + Math.floor(rand() * 100) }));
  const edges: LayoutEdge[] = [];
  for (let i = 1; i < n; i += 1) {
    for (let k = 0; k < 2; k += 1) {
      const j = Math.floor(rand() * i);
      edges.push(rand() < 0.05 ? { from: `c${j}`, to: `c${i}` } : { from: `c${i}`, to: `c${j}` });
    }
  }
  const times: number[] = [];
  let layout = layeredLayout(nodes, edges);
  for (let r = 0; r < 20; r += 1) {
    const t = performance.now();
    layout = layeredLayout(nodes, edges);
    times.push(performance.now() - t);
  }
  times.sort((a, b) => a - b);

  const placed = [...layout.nodes.values()];
  const byLayer = new Map<number, typeof placed>();
  for (const p of placed) byLayer.set(p.layer, [...(byLayer.get(p.layer) ?? []), p]);
  const overlaps = placed.some((a) =>
    (byLayer.get(a.layer) ?? []).some((b) => a !== b && a.x < b.x + b.width && b.x < a.x + a.width),
  );
  const eps = 0.01;
  let missing = 0;
  let badEnds = 0;
  let crossings = 0;
  let upward = 0;
  edges.forEach((e, i) => {
    const route = layout.routes[i];
    if (!route) {
      missing += 1;
      return;
    }
    const a = layout.nodes.get(e.from)!;
    const b = layout.nodes.get(e.to)!;
    const first = route[0];
    const last = route[route.length - 1];
    const onBorder = (p: { x: number; y: number }, box: typeof a) =>
      p.x >= box.x - eps && p.x <= box.x + box.width + eps && (Math.abs(p.y - box.y) < eps || Math.abs(p.y - box.y - NODE_HEIGHT) < eps);
    if (!onBorder(first, a) || !onBorder(last, b)) badEnds += 1;
    if (a.layer > b.layer) upward += 1;
    // Tramos rectos dentro de una capa (puntos 1→2, 3→4…): no pueden pisar una caja.
    for (let k = 2; k < route.length - 1; k += 2) {
      const p = route[k];
      const layer = Math.round(route[k - 1].y / (NODE_HEIGHT + 56));
      if ((byLayer.get(layer) ?? []).some((box) => p.x > box.x - eps && p.x < box.x + box.width + eps)) crossings += 1;
    }
  });
  console.log(
    `${String(n).padStart(3)} nodos, ${edges.length} aristas · layout p50 ${times[10].toFixed(2)} ms, máx ${times[19].toFixed(2)} ms · ` +
      `${layout.width.toFixed(0)}×${layout.height.toFixed(0)} · solapes: ${overlaps ? "sí" : "no"} · sin ruta ${missing} · ` +
      `extremos mal ${badEnds} · tramos sobre cajas ${crossings} · hacia arriba (ciclos) ${upward}`,
  );
}
