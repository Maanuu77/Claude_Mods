// B6/B7 (SPEC §11.4): crea, modifica, renombra y borra 100 ficheros en un repo de
// prueba mientras DEV COCKPIT lo vigila, para medir cambio → pintado (B6) y
// `numstat` → pintado (B7) con `perf_mark`.
//
// Uso (la app se lanza aparte con DEV_COCKPIT_SESSIONS_DIR apuntando a <registro>):
//   node scripts/measure/fs-ops.mjs <carpeta-de-prueba> <registro>
//
// Escribe en <registro> una entrada de sesión con el PID de este proceso y el
// repo como `cwd`, para que la app active la raíz. No toca ~/.claude.

import { execFileSync } from "node:child_process";
import { mkdirSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [base, registry] = process.argv.slice(2);
if (!base || !registry) {
  console.error("uso: node fs-ops.mjs <carpeta-de-prueba> <registro>");
  process.exit(1);
}
const repo = join(base, "repo");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const git = (...args) =>
  execFileSync(
    "git",
    ["-c", "user.name=b6", "-c", "user.email=b6@example.invalid", "-c", "commit.gpgsign=false", "-C", repo, ...args],
    { stdio: "ignore" },
  );

mkdirSync(join(repo, "src"), { recursive: true });
for (let i = 0; i < 100; i++) writeFileSync(join(repo, "src", `base${i}.txt`), `línea ${i}\n`);
git("init", "-q", "-b", "main");
git("add", "-A");
git("commit", "-q", "-m", "base");

mkdirSync(registry, { recursive: true });
writeFileSync(
  join(registry, `${process.pid}.json`),
  JSON.stringify({ pid: process.pid, sessionId: "b6b6b6b6-0000-4000-8000-0000000000b6", cwd: repo, status: "busy" }),
);
console.log(`registro escrito; esperando a que la app active la raíz…`);
await sleep(8000);

const ops = [];
const op = async (kind, fn) => {
  ops.push({ kind, t: Date.now() });
  fn();
  await sleep(150);
};
for (let i = 0; i < 100; i++) await op("create", () => writeFileSync(join(repo, "src", `nuevo${i}.txt`), "x\n"));
for (let i = 0; i < 100; i++) await op("modify", () => writeFileSync(join(repo, "src", `base${i}.txt`), `cambio ${i}\n`));
for (let i = 0; i < 50; i++)
  await op("rename", () => renameSync(join(repo, "src", `nuevo${i}.txt`), join(repo, "src", `renombrado${i}.txt`)));
for (let i = 50; i < 100; i++) await op("delete", () => rmSync(join(repo, "src", `nuevo${i}.txt`)));
for (let i = 0; i < 50; i++) await op("delete", () => rmSync(join(repo, "src", `base${i}.txt`)));
await sleep(3000);
writeFileSync(join(base, "ops.json"), JSON.stringify(ops));
rmSync(join(registry, `${process.pid}.json`));
console.log(`${ops.length} operaciones hechas`);
