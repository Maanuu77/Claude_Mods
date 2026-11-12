/**
 * Layout por capas del grafo de componentes (SPEC §9, G8), sin dependencias.
 * Es un Sugiyama simplificado:
 *
 * 1. Ciclos: las aristas de vuelta de un DFS se invierten solo para el layout (se
 *    dibujan hacia arriba). Invertirlas deja siempre un grafo acíclico.
 * 2. Capa = camino más largo desde una fuente: quien depende queda encima de aquello
 *    de lo que depende, y un grupo encima de sus miembros.
 * 3. Una arista que salta capas pasa por un nodo virtual en cada capa intermedia,
 *    así que nunca atraviesa una caja: entre capas no hay cajas y, dentro de una
 *    capa, el nodo virtual ocupa su propio hueco.
 * 4. Orden dentro de cada capa por baricentro de los vecinos, en varias pasadas
 *    hacia abajo y hacia arriba, para reducir cruces.
 * 5. Posición horizontal: cada nodo, lo más cerca posible de la media de sus
 *    vecinos sin romper el orden ni la separación mínima (regresión isotónica por
 *    capa), en varias pasadas. Así las aristas largas quedan casi rectas.
 *
 * Coste O((V + D + E) · pasadas · log) con D = nodos virtuales. Puro: sin DOM ni
 * React, para poder medirlo aparte (scripts/measure/g8-layout.ts).
 */

export interface LayoutNode {
  id: string;
  /** Ancho de la caja, en las mismas unidades que el resultado. */
  width: number;
}

export interface LayoutEdge {
  from: string;
  to: string;
}

export interface Point {
  x: number;
  y: number;
}

export interface PlacedNode {
  id: string;
  x: number;
  y: number;
  width: number;
  layer: number;
}

export interface Layout {
  nodes: Map<string, PlacedNode>;
  /**
   * Ruta de cada arista de entrada (mismo índice), de `from` a `to`; `null` si no se
   * dibuja (bucle o extremo desconocido). Los puntos alternan tramo entre capas
   * (pares → impares: curva) y paso por un nodo virtual (impares → pares: recta).
   */
  routes: (Point[] | null)[];
  width: number;
  height: number;
}

export const NODE_HEIGHT = 34;
const H_GAP = 24;
const V_GAP = 56;
const VIRTUAL_WIDTH = 2;
const VIRTUAL_GAP = 12;
/** Separación entre aristas en el borde de una caja: más ancha que la punta de flecha. */
const PORT_SPACING = 16;
const SWEEPS = 4;
const PLACEMENT_PASSES = 6;

export function layeredLayout(nodes: LayoutNode[], edges: LayoutEdge[]): Layout {
  const n = nodes.length;
  const index = new Map(nodes.map((node, i) => [node.id, i]));
  const outgoing: { to: number; edge: number }[][] = nodes.map(() => []);
  edges.forEach((e, edge) => {
    const a = index.get(e.from);
    const b = index.get(e.to);
    if (a !== undefined && b !== undefined && a !== b) outgoing[a].push({ to: b, edge });
  });

  // 1. Aristas de vuelta de un DFS (iterativo: sin límite de pila).
  const reversed = new Set<number>();
  const state = new Uint8Array(n); // 0 sin ver, 1 en la pila, 2 hecho
  for (let root = 0; root < n; root += 1) {
    if (state[root]) continue;
    const stack: [number, number][] = [[root, 0]];
    state[root] = 1;
    while (stack.length > 0) {
      const top = stack[stack.length - 1];
      const next = outgoing[top[0]][top[1]];
      if (next === undefined) {
        state[top[0]] = 2;
        stack.pop();
        continue;
      }
      top[1] += 1;
      if (state[next.to] === 1) reversed.add(next.edge);
      else if (state[next.to] === 0) {
        state[next.to] = 1;
        stack.push([next.to, 0]);
      }
    }
  }
  const dag: { u: number; v: number; edge: number }[] = [];
  outgoing.forEach((list, a) =>
    list.forEach(({ to, edge }) => dag.push(reversed.has(edge) ? { u: to, v: a, edge } : { u: a, v: to, edge })),
  );

  // 2. Capas por el camino más largo (orden topológico de Kahn).
  const succ: number[][] = nodes.map(() => []);
  const indegree = new Int32Array(n);
  for (const { u, v } of dag) {
    succ[u].push(v);
    indegree[v] += 1;
  }
  const layerOf: number[] = new Array<number>(n).fill(0);
  const queue: number[] = [];
  for (let i = 0; i < n; i += 1) if (indegree[i] === 0) queue.push(i);
  for (let q = 0; q < queue.length; q += 1) {
    const u = queue[q];
    for (const v of succ[u]) {
      if (layerOf[u] + 1 > layerOf[v]) layerOf[v] = layerOf[u] + 1;
      indegree[v] -= 1;
      if (indegree[v] === 0) queue.push(v);
    }
  }

  // 3. Nodos virtuales: cada arista queda como una cadena de tramos de una capa.
  const width: number[] = nodes.map((node) => node.width);
  const up: number[][] = nodes.map(() => []); // vecinos en la capa de arriba
  const down: number[][] = nodes.map(() => []); // vecinos en la capa de abajo
  const chains: number[][] = [];
  for (const { u, v } of dag) {
    const chain = [u];
    let prev = u;
    for (let l = layerOf[u] + 1; l < layerOf[v]; l += 1) {
      const d = width.length;
      width.push(VIRTUAL_WIDTH);
      layerOf.push(l);
      up.push([prev]);
      down.push([]);
      down[prev].push(d);
      chain.push(d);
      prev = d;
    }
    down[prev].push(v);
    up[v].push(prev);
    chain.push(v);
    chains.push(chain);
  }
  const total = width.length;
  const layers: number[][] = [];
  for (let x = 0; x < total; x += 1) (layers[layerOf[x]] ??= []).push(x);

  // 4. Orden por baricentros.
  const pos = new Float64Array(total);
  const renumber = (ids: number[]) => ids.forEach((x, i) => (pos[x] = i));
  layers.forEach(renumber);
  const key = new Float64Array(total);
  const reorder = (ids: number[], neighbours: number[][]) => {
    for (const x of ids) {
      const ns = neighbours[x];
      let sum = 0;
      for (const y of ns) sum += pos[y];
      key[x] = ns.length === 0 ? pos[x] : sum / ns.length;
    }
    ids.sort((a, b) => key[a] - key[b] || pos[a] - pos[b]);
    renumber(ids);
  };
  for (let sweep = 0; sweep < SWEEPS; sweep += 1) {
    for (let l = 1; l < layers.length; l += 1) reorder(layers[l], up);
    for (let l = layers.length - 2; l >= 0; l -= 1) reorder(layers[l], down);
  }

  // 5. Posición horizontal (centros), respetando orden y separación.
  const center = new Float64Array(total);
  const sep = (a: number, b: number) =>
    (width[a] + width[b]) / 2 + (a >= n || b >= n ? VIRTUAL_GAP : H_GAP);
  const offsets = (ids: number[]) => {
    const o = new Float64Array(ids.length);
    for (let i = 1; i < ids.length; i += 1) o[i] = o[i - 1] + sep(ids[i - 1], ids[i]);
    return o;
  };
  for (const ids of layers) {
    const o = offsets(ids);
    const half = ids.length > 0 ? o[ids.length - 1] / 2 : 0;
    ids.forEach((x, i) => (center[x] = o[i] - half));
  }
  // Regresión isotónica (pool adjacent violators): minimiza Σ(c − deseado)² con
  // c[i+1] − c[i] ≥ sep(i, i+1).
  const sums: number[] = [];
  const counts: number[] = [];
  const place = (ids: number[], desired: (x: number) => number) => {
    if (ids.length === 0) return;
    const o = offsets(ids);
    sums.length = 0;
    counts.length = 0;
    for (let i = 0; i < ids.length; i += 1) {
      let s = desired(ids[i]) - o[i];
      let c = 1;
      while (sums.length > 0 && sums[sums.length - 1] / counts[counts.length - 1] > s / c) {
        s += sums.pop()!;
        c += counts.pop()!;
      }
      sums.push(s);
      counts.push(c);
    }
    let i = 0;
    for (let b = 0; b < sums.length; b += 1) {
      const y = sums[b] / counts[b];
      for (let k = 0; k < counts[b]; k += 1, i += 1) center[ids[i]] = y + o[i];
    }
  };
  const mean = (x: number, lists: number[][][]) => {
    let sum = 0;
    let count = 0;
    for (const list of lists) {
      for (const y of list[x]) {
        sum += center[y];
        count += 1;
      }
    }
    return count === 0 ? center[x] : sum / count;
  };
  for (let pass = 0; pass < PLACEMENT_PASSES; pass += 1) {
    for (let l = 1; l < layers.length; l += 1) place(layers[l], (x) => mean(x, [up]));
    for (let l = layers.length - 2; l >= 0; l -= 1) place(layers[l], (x) => mean(x, [down]));
  }
  for (const ids of layers) place(ids, (x) => mean(x, [up, down]));

  let minX = Infinity;
  let maxX = -Infinity;
  for (let x = 0; x < total; x += 1) {
    minX = Math.min(minX, center[x] - width[x] / 2);
    maxX = Math.max(maxX, center[x] + width[x] / 2);
  }
  if (total === 0) minX = maxX = 0;
  for (let x = 0; x < total; x += 1) center[x] -= minX;

  const rowY = (l: number) => l * (NODE_HEIGHT + V_GAP);
  const placed = new Map<string, PlacedNode>();
  nodes.forEach((node, i) =>
    placed.set(node.id, { id: node.id, x: center[i] - node.width / 2, y: rowY(layerOf[i]), width: node.width, layer: layerOf[i] }),
  );

  // Puertos: las aristas que salen (abajo) o entran (arriba) de una caja se reparten
  // por su borde, ordenadas como sus vecinos, para que no se superpongan.
  const outPort = new Float64Array(chains.length);
  const inPort = new Float64Array(chains.length);
  const outs: number[][] = nodes.map(() => []);
  const ins: number[][] = nodes.map(() => []);
  chains.forEach((chain, k) => {
    outs[chain[0]].push(k);
    ins[chain[chain.length - 1]].push(k);
  });
  const spread = (x: number, list: number[], neighbour: (k: number) => number, into: Float64Array) => {
    list.sort((a, b) => center[neighbour(a)] - center[neighbour(b)] || a - b);
    const span = Math.min(Math.max(0, width[x] - 16), (list.length - 1) * PORT_SPACING);
    list.forEach((k, i) => (into[k] = center[x] + (list.length === 1 ? 0 : span * (i / (list.length - 1) - 0.5))));
  };
  for (let x = 0; x < n; x += 1) {
    spread(x, outs[x], (k) => chains[k][1], outPort);
    spread(x, ins[x], (k) => chains[k][chains[k].length - 2], inPort);
  }

  const routes: (Point[] | null)[] = edges.map(() => null);
  dag.forEach(({ edge }, k) => {
    const chain = chains[k];
    const points: Point[] = [{ x: outPort[k], y: rowY(layerOf[chain[0]]) + NODE_HEIGHT }];
    for (let i = 1; i < chain.length - 1; i += 1) {
      const d = chain[i];
      points.push({ x: center[d], y: rowY(layerOf[d]) }, { x: center[d], y: rowY(layerOf[d]) + NODE_HEIGHT });
    }
    points.push({ x: inPort[k], y: rowY(layerOf[chain[chain.length - 1]]) });
    routes[edge] = reversed.has(edge) ? points.reverse() : points;
  });

  return {
    nodes: placed,
    routes,
    width: maxX - minX,
    height: layers.length === 0 ? 0 : layers.length * (NODE_HEIGHT + V_GAP) - V_GAP,
  };
}
