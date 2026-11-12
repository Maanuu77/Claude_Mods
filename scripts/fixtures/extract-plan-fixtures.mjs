// Extrae fixtures de `mcp__ccd_turn__update_plan` (SPEC §8, puerta G6) de los
// transcripts locales, sin ningún contenido del usuario.
//
// Solo lee `~/.claude/projects`. De cada llamada del bucle principal guarda:
// - la forma de la entrada, con el texto de cada paso cambiado por «Paso k»;
// - las llamadas a `wrap_up` (solo que ocurrieron);
// - si el resultado fue un error;
// - la primera frase del resultado, que solo contiene números
//   («Plan updated: step 2 of 7 is in progress.», «Plan shown (7 steps).»).
// Las sesiones se renombran s1, s2… y se agrupan por versión del motor.
//
// Uso: node scripts/fixtures/extract-plan-fixtures.mjs <carpeta de salida> [versión…]
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const [outDir, ...wanted] = process.argv.slice(2);
if (!outDir) {
  console.error("Uso: extract-plan-fixtures.mjs <carpeta de salida> [versión…]");
  process.exit(2);
}
const root = path.join(os.homedir(), ".claude", "projects");
const TOOL = "mcp__ccd_turn__update_plan";
const WRAP_UP = "mcp__ccd_turn__wrap_up";
const STATUSES = new Set(["pending", "in_progress", "completed"]);
/** versión → líneas */
const byVersion = new Map();
let sessionCounter = 0;

function sanitizeInput(input) {
  const out = {};
  for (const [key, value] of Object.entries(input ?? {})) {
    if (key === "items" && Array.isArray(value)) {
      out.items = value.map((item, i) => ({
        content: `Paso ${i + 1}`,
        status: STATUSES.has(item?.status) ? item.status : "?",
      }));
    } else if (key === "advance") {
      out.advance = value === true;
    } else if (key === "done" && Array.isArray(value)) {
      out.done = value.filter((n) => Number.isInteger(n));
    } else if (key === "start" && Number.isInteger(value)) {
      out.start = value;
    } else if (key === "scope" && typeof value === "string" && /^[a-z_]{1,20}$/.test(value)) {
      out.scope = value;
    } else {
      // Campo desconocido: se conserva solo el nombre, para detectar cambios de esquema.
      out[`unknown:${key.replace(/[^A-Za-z0-9_]/g, "").slice(0, 32)}`] = true;
    }
  }
  return out;
}

function firstSentence(text) {
  const m = /^(Plan (?:updated|shown)[^.]*\.)/.exec(text.trim());
  return m ? m[1] : "?";
}

function resultText(block) {
  if (Array.isArray(block.content)) return block.content.map((c) => (typeof c?.text === "string" ? c.text : "")).join(" ");
  return typeof block.content === "string" ? block.content : "";
}

function walk(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== "subagents") walk(p);
    } else if (entry.name.endsWith(".jsonl")) {
      scan(p);
    }
  }
}

function scan(file) {
  const text = fs.readFileSync(file, "utf8");
  if (!text.includes(TOOL)) return;
  const calls = new Map(); // tool_use_id → registro
  const order = [];
  let version = null;
  for (const line of text.split("\n")) {
    if (!line.trim()) continue;
    let j;
    try {
      j = JSON.parse(line);
    } catch {
      continue;
    }
    if (j.type !== "assistant" && j.type !== "user") continue;
    const content = j.message?.content;
    if (!Array.isArray(content)) continue;
    for (const b of content) {
      if (b?.type === "tool_use" && b.name === TOOL && !j.agentId) {
        version ??= j.version ?? null;
        const rec = { version: j.version ?? null, input: sanitizeInput(b.input), is_error: null, result: null };
        calls.set(b.id, rec);
        order.push(rec);
      } else if (b?.type === "tool_use" && b.name === WRAP_UP && !j.agentId && order.length > 0) {
        // `wrap_up` también «marca» la lista: se guarda solo que ocurrió, sin su texto.
        const rec = { version: j.version ?? null, tool: "wrap_up", is_error: null, result: null };
        calls.set(b.id, rec);
        order.push(rec);
      } else if (b?.type === "tool_result" && calls.has(b.tool_use_id)) {
        const rec = calls.get(b.tool_use_id);
        rec.is_error = b.is_error === true;
        if (rec.tool === "wrap_up") rec.result = rec.is_error ? "[error]" : "ok";
        else rec.result = rec.is_error ? "[error]" : firstSentence(resultText(b));
      }
    }
  }
  if (order.length === 0 || !version) return;
  if (wanted.length > 0 && !wanted.includes(version)) return;
  sessionCounter += 1;
  const session = `s${sessionCounter}`;
  const lines = byVersion.get(version) ?? [];
  order.forEach((rec, seq) => lines.push(JSON.stringify({ session, seq, ...rec })));
  byVersion.set(version, lines);
}

walk(root);
fs.mkdirSync(outDir, { recursive: true });
for (const [version, lines] of [...byVersion].sort()) {
  const file = path.join(outDir, `desktop-${version}.jsonl`);
  fs.writeFileSync(file, lines.join("\n") + "\n");
  const sessions = new Set(lines.map((l) => JSON.parse(l).session)).size;
  console.log(`${version}: ${lines.length} llamadas en ${sessions} sesiones → ${file}`);
}
