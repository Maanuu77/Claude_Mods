// B5: reproduce un transcript real a ritmo de pico (SPEC §11.4).
//
// Lee las líneas `assistant` y `user` de un JSONL (solo lectura) y las añade a un
// JSONL de prueba, de forma que las llamadas a herramientas salgan a N por minuto
// (107 en la SPEC, la punta medida en M2). Cada línea se reescribe con la hora en
// que se escribe, una sesión de prueba y un `cwd` de prueba. Cuando el original se
// acaba, vuelve a empezar con ids nuevos (`message.id`, `tool_use.id` y su
// `tool_result`), para que la app no los deduplique.
//
// Uso: node replay-transcript.mjs <transcript real> <carpeta projects de prueba> [herramientas/min=107] [minutos=30]
// Nunca se escribe en ~/.claude: la app se lanza con DEV_COCKPIT_PROJECTS_DIR. El
// JSONL de prueba lleva contenido real del transcript: hay que borrarlo al acabar.
import fs from "node:fs";
import path from "node:path";

const [source, root, rateArg = "107", minutesArg = "30"] = process.argv.slice(2);
if (!source || !root) {
  console.error("uso: node replay-transcript.mjs <transcript> <carpeta projects de prueba> [herramientas/min] [minutos]");
  process.exit(2);
}
const perTool = 60_000 / Number(rateArg);
const endAt = Date.now() + Number(minutesArg) * 60_000;
const session = "bbbbbbbb-5555-4555-8555-000000000005";
const cwd = "C:\\work\\demo";
const dir = path.join(root, "C--work-demo");
fs.mkdirSync(dir, { recursive: true });
const out = path.join(dir, `${session}.jsonl`);

const lines = fs
  .readFileSync(source, "utf8")
  .split("\n")
  .filter((l) => l.includes('"type":"assistant"') || l.includes('"type":"user"'))
  .map((l) => {
    try {
      return JSON.parse(l);
    } catch {
      return null;
    }
  })
  .filter((j) => j && (j.type === "assistant" || j.type === "user") && j.message);
const toolsIn = (j) => (Array.isArray(j.message.content) ? j.message.content.filter((b) => b?.type === "tool_use").length : 0);
const total = lines.reduce((n, j) => n + toolsIn(j), 0);
if (total === 0) {
  console.error("el transcript no tiene llamadas a herramientas");
  process.exit(1);
}
console.log(`${lines.length} líneas, ${total} herramientas por vuelta; ritmo ${rateArg}/min durante ${minutesArg} min`);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let round = 0;
let written = 0;
let tools = 0;
let next = Date.now();
const fd = fs.openSync(out, "a");
try {
  while (Date.now() < endAt) {
    round += 1;
    const tag = `r${round}`;
    for (const original of lines) {
      if (Date.now() >= endAt) break;
      const j = structuredClone(original);
      j.sessionId = session;
      j.cwd = cwd;
      j.timestamp = new Date().toISOString();
      delete j.agentId; // todo en el bucle principal
      if (j.message.id) j.message.id = `${j.message.id}_${tag}`;
      if (Array.isArray(j.message.content)) {
        for (const b of j.message.content) {
          if (b?.type === "tool_use" && b.id) b.id = `${b.id}_${tag}`;
          if (b?.type === "tool_result" && b.tool_use_id) b.tool_use_id = `${b.tool_use_id}_${tag}`;
        }
      }
      fs.writeSync(fd, JSON.stringify(j) + "\n");
      written += 1;
      const n = toolsIn(original);
      if (n > 0) {
        tools += n;
        next += n * perTool;
        const wait = next - Date.now();
        if (wait > 0) await sleep(wait);
      }
    }
  }
} finally {
  fs.closeSync(fd);
}
console.log(`escritas ${written} líneas y ${tools} herramientas en ${round} vueltas`);
