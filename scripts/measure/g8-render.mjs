// G8 (SPEC §13.2): tiempo hasta ver pintado el grafo de componentes en el motor de
// WebView2 (Edge headless), con 50, 200 y 500 nodos.
//
//   node --no-warnings scripts/measure/g8-render.mjs
//
// Construye con DOM el mismo SVG que `src/map/ComponentGraph.tsx` (sin React) sobre
// el layout real (`src/map/layout.ts`, con los tipos quitados) y mide layout, DOM y
// los dos fotogramas siguientes (estilo, layout y pintado). Usa un perfil temporal
// de Edge en el directorio temporal del sistema y lo borra al terminar. No toca la
// base de datos ni `~/.claude`.
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const EDGE = [
  "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
  "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
].find((p) => existsSync(p));
if (!EDGE) throw new Error("no se encuentra msedge.exe");
const PORT = 9333;

const layout = stripTypeScriptTypes(readFileSync(new URL("../../src/map/layout.ts", import.meta.url), "utf8")).replace(
  /^export /gm,
  "",
);

function harness() {
  const NS = "http://www.w3.org/2000/svg";
  const num = (v) => Math.round(v * 10) / 10;
  function pathOf(points) {
    let d = `M${num(points[0].x)},${num(points[0].y)}`;
    for (let i = 1; i < points.length; i += 1) {
      const a = points[i - 1];
      const b = points[i];
      if (i % 2 === 1) {
        const mid = num((a.y + b.y) / 2);
        d += ` C${num(a.x)},${mid} ${num(b.x)},${mid} ${num(b.x)},${num(b.y)}`;
      } else d += ` L${num(b.x)},${num(b.y)}`;
    }
    return d;
  }
  // Mismo generador que scripts/measure/g8-layout.ts (semilla fija, reiniciada por tamaño).
  function graph(n) {
    let seed = 42;
    const rand = () => {
      seed = (seed * 1103515245 + 12345) % 2147483648;
      return seed / 2147483648;
    };
    const nodes = Array.from({ length: n }, (_, i) => ({ id: `c${i}`, width: 80 + Math.floor(rand() * 100) }));
    const edges = [];
    for (let i = 1; i < n; i += 1) {
      for (let k = 0; k < 2; k += 1) {
        const j = Math.floor(rand() * i);
        edges.push(rand() < 0.05 ? { from: `c${j}`, to: `c${i}` } : { from: `c${i}`, to: `c${j}` });
      }
    }
    return { nodes, edges };
  }
  const frame = () => new Promise((r) => requestAnimationFrame(() => r()));
  const el = (tag, attrs, parent) => {
    const e = document.createElementNS(NS, tag);
    for (const k in attrs) e.setAttribute(k, attrs[k]);
    parent.appendChild(e);
    return e;
  };
  async function run(n) {
    const { nodes, edges } = graph(n);
    const host = document.getElementById("host");
    const out = [];
    for (let rep = 0; rep < 7; rep += 1) {
      host.textContent = "";
      await frame();
      const t0 = performance.now();
      const L = layeredLayout(nodes, edges);
      const t1 = performance.now();
      const PAD = 16;
      const w = L.width + PAD * 2;
      const h = L.height + PAD * 2;
      const svg = document.createElementNS(NS, "svg");
      svg.setAttribute("width", w);
      svg.setAttribute("height", h);
      svg.setAttribute("viewBox", `${-PAD} ${-PAD} ${w} ${h}`);
      const marker = el("marker", { id: "a", viewBox: "0 0 8 8", refX: 7, refY: 4, markerWidth: 6, markerHeight: 6, orient: "auto" }, el("defs", {}, svg));
      el("path", { d: "M0,0 L8,4 L0,8 z", fill: "#666" }, marker);
      edges.forEach((e, i) => {
        const r = L.routes[i];
        if (!r) return;
        const p = el("path", { d: pathOf(r), fill: "none", stroke: "#666", "stroke-width": 1.5, "marker-end": "url(#a)" }, svg);
        el("title", {}, p).textContent = `${e.from} depende de ${e.to}`;
      });
      for (const nd of L.nodes.values()) {
        const g = el("g", { transform: `translate(${nd.x},${nd.y})` }, svg);
        el("title", {}, g).textContent = nd.id;
        el("rect", { width: nd.width, height: NODE_HEIGHT, rx: 6, fill: "#fff", stroke: "#ccc" }, g);
        el("text", { x: 8, y: 14, "font-size": 12, "font-weight": 600 }, g).textContent = nd.id;
        el("text", { x: 8, y: 27, "font-size": 10, fill: "#888" }, g).textContent = "rust · v0.1.0";
      }
      host.appendChild(svg);
      const t2 = performance.now();
      await frame();
      await frame();
      const t3 = performance.now();
      out.push({ layout: t1 - t0, dom: t2 - t1, frames: t3 - t2, total: t3 - t0 });
    }
    const med = (k) => out.map((o) => o[k]).sort((a, b) => a - b)[3];
    return (
      `${String(n).padStart(3)} nodos, ${edges.length} aristas · layout ${med("layout").toFixed(1)} ms · DOM ${med("dom").toFixed(1)} ms · ` +
      `2 fotogramas ${med("frames").toFixed(1)} ms · total p50 ${med("total").toFixed(1)} ms, máx ${Math.max(...out.map((o) => o.total)).toFixed(1)} ms`
    );
  }
  (async () => {
    const lines = [];
    for (const n of [50, 200, 500]) lines.push(await run(n));
    lines.push(navigator.userAgent.match(/Edg\/[\d.]+/)?.[0] ?? navigator.userAgent);
    document.getElementById("result").textContent = lines.join("\n");
    document.title = "done";
  })();
}

const dir = mkdtempSync(join(tmpdir(), "dev-cockpit-g8-"));
const page = join(dir, "g8-render.html");
writeFileSync(
  page,
  `<!doctype html><meta charset="utf-8"><title>g8</title><pre id="result"></pre>` +
    `<div id="host" style="width:1100px;height:600px;overflow:auto"></div>` +
    `<script>${layout}\n(${harness.toString()})();</script>`,
);
const edge = spawn(
  EDGE,
  [
    "--headless=new",
    `--remote-debugging-port=${PORT}`,
    `--user-data-dir=${join(dir, "profile")}`,
    "--no-first-run",
    "--disable-extensions",
    "--window-size=1280,900",
    pathToFileURL(page).href,
  ],
  { stdio: "ignore" },
);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
try {
  let target;
  for (let i = 0; i < 50 && !target; i += 1) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
      target = list.find((t) => t.type === "page" && t.url.includes("g8-render"));
    } catch {
      // Edge todavía no escucha.
    }
    if (!target) await sleep(200);
  }
  if (!target) throw new Error("Edge headless no abrió la página");
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((r) => ws.addEventListener("open", r));
  let id = 0;
  const pending = new Map();
  ws.addEventListener("message", (m) => {
    const msg = JSON.parse(m.data);
    pending.get(msg.id)?.(msg.result);
  });
  const evaluate = (expression) =>
    new Promise((r) => {
      id += 1;
      pending.set(id, r);
      ws.send(JSON.stringify({ id, method: "Runtime.evaluate", params: { expression, returnByValue: true } }));
    });
  let result = null;
  for (let i = 0; i < 120 && !result; i += 1) {
    const res = await evaluate("document.title === 'done' ? document.getElementById('result').textContent : null");
    result = res?.result?.value ?? null;
    if (!result) await sleep(500);
  }
  ws.close();
  console.log(result ?? "sin resultado en 60 s");
} finally {
  edge.kill();
  await sleep(1000);
  rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
}
