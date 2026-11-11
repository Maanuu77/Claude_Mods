// B4: añade líneas sintéticas a un JSONL de prueba a ritmo constante.
// Cada línea lleva como `timestamp` el instante en que se escribe, así que la
// latencia es (instante de pintado) − (timestamp de la línea).
//
// Uso: node gen-transcript.mjs <carpeta projects de prueba> [líneas=120] [líneas/s=2]
// Nunca se apunta a ~/.claude: la app se lanza con DEV_COCKPIT_PROJECTS_DIR.
import fs from "node:fs";
import path from "node:path";

const [root, countArg = "120", rateArg = "2"] = process.argv.slice(2);
if (!root) {
  console.error("uso: node gen-transcript.mjs <carpeta projects de prueba> [líneas] [líneas/s]");
  process.exit(2);
}
const count = Number(countArg);
const intervalMs = 1000 / Number(rateArg);
const session = "bbbbbbbb-4444-4444-8444-000000000004";
const cwd = "C:\\work\\demo";
const dir = path.join(root, "C--work-demo");
fs.mkdirSync(dir, { recursive: true });
const file = path.join(dir, `${session}.jsonl`);
// Relleno para que la línea tenga el tamaño mediano real (≈ 1,6 KB).
const filler = "lorem ipsum dolor sit amet ".repeat(55);

let n = 0;
const timer = setInterval(() => {
  n += 1;
  const line = {
    type: "assistant",
    sessionId: session,
    timestamp: new Date().toISOString(),
    version: "2.1.321",
    entrypoint: "cli",
    cwd,
    message: {
      id: `msg_bench_${n}`,
      model: "claude-bench",
      role: "assistant",
      stop_reason: "tool_use",
      usage: { input_tokens: 3, output_tokens: 100, cache_read_input_tokens: 1000, cache_creation_input_tokens: 10 },
      content: [
        { type: "text", text: filler },
        { type: "tool_use", id: `toolu_bench_${n}`, name: "Edit", input: { file_path: `${cwd}\\src\\f${n % 7}.rs`, old_string: "a", new_string: "b" } },
      ],
    },
  };
  fs.appendFileSync(file, JSON.stringify(line) + "\n");
  if (n >= count) {
    clearInterval(timer);
    console.log(`escritas ${n} líneas en ${file}`);
  }
}, intervalMs);
