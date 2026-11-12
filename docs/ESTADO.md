# DEV COCKPIT: estado del trabajo (2026-11-12)

Resumen para retomar el proyecto en otra conversación sin releerlo todo. La referencia normativa sigue siendo [`SPEC.md`](SPEC.md); los cambios respecto a ella están en [`CHANGELOG-spec.md`](CHANGELOG-spec.md).

## Qué es

App de escritorio para Windows (Tauri 2.12 + Rust + React + SQLite) que muestra en vivo las sesiones de Claude Code de este equipo, con un mod de Claude Code (`dev-cockpit-bridge`) como puente. Solo observa: no actúa sobre las sesiones.

## Reglas de trabajo que hay que respetar

Están en [`DEVELOPMENT_WORKFLOW.md`](DEVELOPMENT_WORKFLOW.md) (W1–W11). Las que más pesan:

- Nunca se trabaja en `main`; una rama por fase, fusión con `--no-ff`.
- Los commits **no llevan** `Co-authored-by` ni atribución a Claude (W7).
- Ningún `git push` sin autorización, y cada uno por separado (W6).
- Nada escribe en `~/.claude` sin autorización explícita (W11). El mod se carga con `--plugin-dir`, nunca instalado.
- **Acuerdo vigente con el usuario:** los pasos rutinarios (ramas, commits, fusiones de fases que cumplen sus criterios) se hacen sin preguntar. Se pregunta solo para cambios mayores: desviaciones de la SPEC, instalaciones, `push`, escrituras en `~/.claude` o fusionar una fase con criterios sin cumplir.
- Las sesiones de prueba del mod en los motores están autorizadas.

## Entorno

- **Sin permisos de administrador.** No hay MSVC: se usa el toolchain `stable-x86_64-pc-windows-gnu` (fijado en `rust-toolchain.toml`) con MinGW-w64 de WinLibs.
- Antes de compilar hay que tener en el `PATH` `~/.cargo/bin` y la carpeta `mingw64\bin` de WinLibs (bajo `%LOCALAPPDATA%\Microsoft\WinGet\Packages\BrechtSanders.WinLibs...`). En terminales nuevas ya están.
- Motores de Claude Code disponibles: **CLI 2.1.289** (`~/.local/bin/claude.exe`, con login hecho) y **Desktop 2.1.321** (`%APPDATA%\Claude\claude-code\2.1.321\d020ce1b5d30\claude.exe`). **Desktop 2.1.317 ya no está instalado.**
- Las pruebas con el CLI usan `DISABLE_AUTOUPDATER=1` para que no cambie de versión.
- En la herramienta Bash de Claude, los heredocs con apóstrofos fallan: los scripts largos se escriben a fichero y se ejecutan.

## Fases

| Fase | Estado | Dónde |
|---|---|---|
| G-pre (G1, G10) | Hecha y fusionada | `spike/g1-g10-mod-probe/` |
| F0 andamiaje | Hecha y fusionada | — |
| F1 ingesta de transcripts | Hecha y fusionada | `crates/cockpit-core`, `cockpit-store`, `cockpit-ingest` |
| F2 shell Tauri y vista En vivo | Hecha y fusionada | `src-tauri/`, `src/` |
| F3 mod puente | Hecha y fusionada | `mods/dev-cockpit-bridge`, `crates/cockpit-ingest/src/http.rs`, `wire.rs` |
| **F4a Workspace Watcher** | **Hecha en `feature/f4a-workspace-watcher`; pendiente de aprobar la calibración de G5 para fusionar** | `crates/cockpit-ingest/src/{git,watcher,workspace}.rs`, `cockpit-core/src/attribution.rs`, `cockpit-store/src/{compact,workspace}.rs`, `src/changes/` |
| F4b Task Monitor | Sin empezar | — |
| F4c Project Map | Sin empezar | — |
| F5 endurecimiento | Sin empezar | — |

`main` contiene la fusión de F3 (`git log --merges -1 main`).

## F3: lo que está hecho

Commits en `feature/f3-mod-bridge`: `220e844` (lado Rust) y `0cd9537` (mod, tests y compatibilidad).

- **`HttpIngest`** (`crates/cockpit-ingest/src/http.rs`, `wire.rs`): `/v1/hello`, `/v1/events`, `/v1/health` en `127.0.0.1`, token nuevo en cada arranque, publicado en `%APPDATA%\dev-cockpit\endpoint.json`. Los tipos rechazan campos desconocidos.
- **Mod** (`mods/dev-cockpit-bridge/hooks/register.ts`, un solo fichero): avisa de cada herramienta al empezar y al terminar, envía fin de turno y uso, cola acotada a 2.000 eventos, reconexión con vigilancia cada 15 s y política `PRE_HANDSHAKE` hasta el primer `hello`.
- **Store:** migración 2 (`mod_instances`, `turns`, `usage_snapshots`, `sessions.ended_at`).
- **Interfaz:** contexto %, límites de uso e indicador de mod conectado por sesión.
- **Compatibilidad:** `compat/engines/<versión>/claude-code.d.ts` y `compat/tsconfig.<versión>.json`.

Resultados, detallados en [`perf/2026-11-11-f3.md`](perf/2026-11-11-f3.md) (con una sección «Actualización» del 2026-11-12):

| Criterio | Resultado |
|---|---|
| `tsc` contra los tipos de 2.1.289, 2.1.317 y 2.1.321 | pasa |
| `plugin validate` y `plugin test` en CLI 2.1.289 y Desktop 2.1.321 | pasan (8 tests) |
| E2E con herramientas reales, en los dos motores | hecha: 8 herramientas reconciliadas entre mod y transcript, rutas relativas, sin contenido |
| **G4** | **resuelta: sí.** La herramienta del subagente llega con `agentId`. |
| **G3** | **respondida: sí.** El hook de `tool.call` corre antes del diálogo de permisos, así que el tiempo de la herramienta incluye la espera. |
| **B3** herramienta → pintado | p95 de 189 ms con 12 herramientas (objetivo < 300 ms) |
| Latencia mod → app | p95 de 84 ms (CLI) y 78 ms (Desktop) |
| **B11** S1 | 9,2 s (CLI) y 10,6 s (Desktop); objetivo ≤ 15 s. Una muestra por motor, no 10. |

## F3: cómo se cerró (2026-11-12)

- **Estado «esperando permiso» (G3).** `classic.PermissionRequest` no llega al mod en sesiones interactivas (en 2.1.321 lo atiende antes el plugin `cc-plugin-sec-default` del motor). El usuario eligió probar otra señal y funcionó: el mod hookea **`tool.check`**, lee el veredicto y lo devuelve intacto; un `ask` envía `permission_requested` y el store pasa la herramienta a `awaiting_permission`. Comprobado en `-p` con los dos motores y en una sesión interactiva del CLI con el diálogo abierto.
- **Límite conocido:** no hay señal de cuándo se contesta el diálogo. La espera se cierra con el fin de la herramienta, así que una herramienta larga aprobada a mano se ve como «esperando permiso» mientras corre. Sin comprobar: el diálogo interactivo en el Desktop 2.1.321.
- **En los tests del motor**, 2.1.321 comprueba el permiso por su cuenta y pasa el `ask` por `classic.PermissionRequest`; 2.1.289 no. El test de `tool.check` está escrito para los dos.
- **Marca de latencia separada para B3** en `src/live/store.ts` y `perf_mark`.
- **Desviaciones aceptadas por el usuario:** B11 con una muestra por motor y S2 sin medir en sesión real; B2 sin medir aislado; Desktop 2.1.317 sin validar con su binario; hooks `classic.Task*` y `classic.Subagent*` aplazados a F4b. Están en `CHANGELOG-spec.md`.

## F4a: lo que está hecho (2026-11-12)

- **`GitReader`** (`crates/cockpit-ingest/src/git.rs`): binario `git` con el endurecimiento de §7.2 **más `-c diff.autoRefreshIndex=false`**, lista blanca de subcomandos aplicada en código (y opciones prohibidas como `--output`), `CREATE_NO_WINDOW`, tope de salida y tiempo máximo. `resolve` (raíz, prefijo del `cwd`, worktrees), `status` porcelain v2, `numstat`, diff de un fichero (2 MB) y `count_tracked`.
- **Watcher** (`watcher.rs`): `notify-debouncer-full` 0.7 con la caché `PairCache`, exclusiones fijas, `.gitignore` por directorio + `info/exclude` + global (crate `ignore`, sin distinguir mayúsculas), modo masivo (> 500 eventos/s, pausa de 2 s), guardado atómico de Claude Code y unión de movimientos entre carpetas.
- **Gestor de raíces** (`workspace.rs`): resolución `cwd` → raíz en un hilo aparte (también del histórico al arrancar), activación por sesión viva y desactivación tras 10 min, un hilo de Git por raíz (coalescencia de 500 ms, como máximo 1 s desde el primer cambio pendiente), atribución provisional 2 s y reevaluación con cada herramienta de la raíz.
- **Atribución** (`cockpit-core/src/attribution.rs`): R1–R5 de §7.3 con la calibración de G5.
- **Store:** migración 3 (`file_events`, `file_change_daily`, `tool_usage_daily`, `aggregation_ledger`, triggers de §6.4.2) y compactación transaccional con purga (`compact.rs`), al arrancar y cada 24 h.
- **Interfaz:** pestaña «Cambios en directo» con la lista virtualizada (`@tanstack/react-virtual`), agrupada por herramienta, `+/−` de `numstat`, ficheros con cambios según `git status` y visor de diff o vista previa. Comandos Tauri `query_file_events`, `git_diff` y `read_file_preview` (solo sobre raíces activas).
- **Fallo previo corregido:** `PidProbe` no devolvía ningún proceso si un PID se repetía.

Resultados en [`perf/2026-11-12-f4a.md`](perf/2026-11-12-f4a.md):

| Criterio | Resultado |
|---|---|
| Tests de §6.4.2 (idempotencia, fallo inyectado, día tardío, purga solo en `done`, R-2 ×3, R-4 ×2) | pasan |
| Purga: ninguna ruta de más de 30 días | pasa |
| **M5** automatizado | pasa, **tras añadir `diff.autoRefreshIndex=false`**: `git diff … HEAD` reescribía el índice |
| **G5** (256 cambios) | `high` 1,00 · `medium` 0,86 · 0 externos a `claude`, **con la calibración de §7.3** (sin ella: 0,73 y 8) |
| **G7** | mismo directorio 40/40; entre carpetas con el mismo nombre 20/20; con otro nombre, borrado + creado (respaldo) |
| **B6** cambio → pintado | p95 de 317 ms con 350 cambios (objetivo < 700 ms) |
| **B7** `numstat` → pintado | p95 de 1,19 s por cambio con un cambio cada 150 ms (objetivo < 1,5 s), tras limitar la coalescencia a 1 s desde el primer cambio (con 2 s: 2,4 s) |
| Prueba en la app real (copia de la base) | raíz activada sola; un `Edit` de esta sesión, `Claude · alta` solo con el transcript |

## Siguiente paso

1. **Aprobar (o no) la calibración de G5** (CHANGELOG, entrada de F4a) y fusionar F4a.
2. **F4b, Task Monitor** (SPEC §8): rama `feature/f4b-task-monitor`. Puerta G6; incluye los hooks `classic.Task*` y `classic.Subagent*` aplazados de F3.

## Hallazgos que afectan a fases futuras

- **F4b:** en 2.1.289 y 2.1.321 el modelo usó `TaskCreate`, no `TodoWrite`. El adaptador A1 importa más de lo que decía la SPEC.
- **Reinicio de la app:** una sesión inactiva con el mod no se entera y aparece como «sin mod» hasta su siguiente evento.
- **`cost-state`:** los tokens deduplicados del transcript solo coinciden exactamente con `cost-state` en 19 de 33 sesiones.
- **Aviso del enlazador** `.rsrc merge failure: multiple non-default manifests`: sigue sin resolver; hay que mirarlo antes del instalador de F5.
- **Tabla `pricing` vacía:** no hay precios de modelos cargados.
- **Primera ejecución de F4a sobre la base real:** la compactación agregará todos los días cerrados y **purgará las rutas de más de 30 días** (D3). Es lo previsto, pero no se puede deshacer; las pruebas de F4a usaron una copia.
- **F4c (Diagnóstico):** faltan los avisos de §6.4.2 (días sin agregar, reconciliación con diferencias) y «borrar rutas ahora». `Store::reconcile` ya existe.
- **Atribución, límites conocidos:** un cambio externo durante un `Bash` de la misma raíz se atribuye a ese `Bash` (R3), y un `Bash` en segundo plano que sigue escribiendo cuando la herramienta ya ha terminado queda como `unknown`.
- **Unidades de red con letra:** no se detectan (solo rutas UNC); se vigilarían con `ReadDirectoryChangesW`, que Microsoft no garantiza en red.

## Comandos de comprobación

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run typecheck
npm run lint
npx tsc -p compat/tsconfig.2.1.289.json        (y .317, .321)
claude plugin validate mods/dev-cockpit-bridge
claude plugin test mods/dev-cockpit-bridge
npm run tauri build -- --no-bundle             (deja target\release\dev-cockpit.exe)
cargo run -p cockpit-ingest --release --example workspace_gates -- g5 g5.json   (y g7)
```

Scripts de medición en `scripts/measure/`: `gen-transcript.mjs` (B4), `sample-procs.ps1` (B8), `app-cycle.ps1` (B11) y `workspace-cycle.ps1` + `fs-ops.mjs` (B6, B7).

## Restos en el equipo

- `%APPDATA%\dev-cockpit\cockpit.db`: base real de la app.
- `%LOCALAPPDATA%\com.devcockpit.app`: datos de WebView2 del identificador antiguo, sin uso.
- `target\e2e\`: ficheros de las pruebas E2E (ignorado por Git).
- Registros obsoletos en `~/.claude/sessions` de las sesiones de prueba cerradas de golpe; Claude los limpia solo al arrancar.
- Las pruebas con prompts reales consumieron tokens de la cuenta (alrededor de 0,50 USD estimados por sesión, unas cinco sesiones).
- `%APPDATA%\dev-cockpit\endpoint.json`: quedó de una ejecución anterior que no se cerró limpiamente (estaba antes de F4a). La app lo reescribe al arrancar.
