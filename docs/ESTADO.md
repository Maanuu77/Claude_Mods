# DEV COCKPIT: estado del trabajo (2026-11-13)

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
| F4a Workspace Watcher | Hecha y fusionada | `crates/cockpit-ingest/src/{git,watcher,workspace}.rs`, `cockpit-core/src/attribution.rs`, `cockpit-store/src/{compact,workspace}.rs`, `src/changes/` |
| F4b Task Monitor | Hecha y fusionada (desviaciones del criterio aceptadas por el usuario) | `crates/cockpit-core/src/tasks.rs`, `cockpit-store/src/tasks.rs`, `src/tasks/` |
| F4c Project Map | Hecha y fusionada (sin «borrar rutas ahora», desviación aceptada por el usuario) | `cockpit-core/src/project_map.rs`, `cockpit-ingest/src/project_scan.rs`, `cockpit-store/src/{diagnostics,history,project_cache}.rs`, `src/{map,history,diag}/`; informe `docs/perf/2026-11-12-f4c.md` en esa rama |
| **F5 endurecimiento** | **En curso** en `feature/f5-hardening`: hecho todo menos **B9**, que el usuario ejecuta en otro equipo | `src-tauri/{build.rs,windows-app-manifest.xml}`, `cockpit-ingest/src/settings.rs`, `cockpit-store/src/compact.rs` (`clear_paths_now`), `src/diag/PrivacySettings.tsx`, `scripts/measure/{b9-soak.ps1,replay-transcript.mjs}`; informe `docs/perf/2026-11-12-f5.md` |
| **Identidad visual** | **Hecha** en `feature/visual-identity` (sale de `feature/f5-hardening`); sin fusionar hasta cerrar F5 | `docs/DESIGN.md`, `src/ui/`, `src/app/`, CSS por vista, `src-tauri/icons/` |

`main` contiene la fusión de F4c (`git log --merges -1 main`). La calibración de G5 (SPEC §7.3), las desviaciones del criterio de F4b y la de F4c («borrar rutas ahora» aplazado) las aceptó el usuario el 2026-11-12.

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

## F4b: lo que está hecho (2026-11-12)

Commits en `feature/f4b-task-monitor`: `b25f4f9` (núcleo, store, contrato y G6), `05290dc` (mod y vista «Tareas»), `2b17f4a` (relectura del historial), `9128595` (`session-owner`) y los de documentación de P1 y P2.

- **Adaptadores** (`cockpit-core/src/tasks.rs`): A1 `TaskCreate`/`TaskUpdate` y hooks `TaskCreated`/`TaskCompleted`, A2 `TodoWrite`, A3 `update_plan` del Desktop (desactivable con `DEV_COCKPIT_DISABLE_DESKTOP_PLAN`) y A4 subagentes. Cada llamada es una operación idempotente; el estado de una lista es su reproducción ordenada, así que el mod y el transcript pueden entregar la misma en cualquier orden. Solo se leen los campos de §10; los títulos, truncados a 120 caracteres (D1). Una forma inesperada es deriva, nunca una suposición.
- **Store:** migración 4 (`task_ops`, `tasks`, `agents`, `file_offsets.parser_version`) y vistas por sesión de tareas, subagentes y actividad.
- **Contrato HTTP:** eventos `task_op` y `agents`, con `deny_unknown_fields`; la app vuelve a aplicar su política.
- **Mod:** envía la operación de cada herramienta de tareas al terminar, hookea `classic.Task*` y `classic.Subagent*` (aplazados de F3) y lee `$.agent.list()` por evento.
- **Interfaz:** pestaña «Tareas» con las listas declaradas (fuente, completadas/total, paso en curso, bloqueos), el árbol de subagentes y la actividad aparte; la tarjeta «En vivo» muestra el progreso declarado.
- **Relectura del historial:** cada fichero recuerda la versión del parser con que se leyó; los leídos antes de F4b se releen aplicando solo los eventos nuevos (sin recuperar rutas purgadas).

Resultados en [`perf/2026-11-12-f4b.md`](perf/2026-11-12-f4b.md):

| Criterio | Resultado |
|---|---|
| **G6** | **resuelta: sí.** 521 de 521 estados comprobables coinciden con lo que respondió el Desktop (2.1.299–2.1.321, fixtures anonimizados) |
| Tests | 148 en verde antes de fusionar; fmt, clippy, typecheck, lint y `tsc` del mod contra 2.1.289/2.1.317/2.1.321 sin avisos |
| Prueba en la app (copia de la base) | 162 ficheros releídos; 567 operaciones (526 `update_plan`, 35 `TodoWrite`, 6 `TaskCreate`), 155 tareas, 99 subagentes; actualización en directo solo con el transcript |
| `claude plugin validate/test` con los tests nuevos | **pasan:** 10/10 en CLI 2.1.289 y Desktop 2.1.321 (autorizado) |
| E2E con el mod en los motores | **pasa con `TaskCreate`/`TaskUpdate`** (repetido a las 22:32, autorizado): en CLI 2.1.289 y Desktop 2.1.321, 5 operaciones por sesión, todas de mod + transcript, sin duplicados, y la lista reconstruida correcta. Sin rutas ni títulos antes del handshake. **`TodoWrite` no se pudo probar en vivo:** los motores no la ofrecen junto a las herramientas de tareas. Esa desviación y la de `update_plan` con el mod las **aceptó el usuario** (2026-11-12) |
| Base real | migrada a la versión 4 y releída; **copia previa** en `%APPDATA%\dev-cockpit\backups\pre-f4b-2026-11-12\` |

## F4c: lo que está hecho (2026-11-12)

Rama `feature/f4c-project-map`, fusionada en `main` con `--no-ff`.

- **Project Map** (`cockpit-core/src/project_map.rs`, `cockpit-ingest/src/project_scan.rs`): detectores de manifiestos de §9 y resumen del árbol (`git ls-files` o recorrido con `.gitignore`; solo `stat` y manifiestos). Vista «Mapa» con el árbol, los componentes, sus relaciones y el grafo.
- **G8:** grafo con SVG y layout por capas propios, sin React Flow ni dagre (desviación por delegación del usuario, en `CHANGELOG-spec.md`).
- **Vistas «Diagnóstico» e «Histórico»** (`cockpit-store/src/{diagnostics,history}.rs`, `src/{diag,history}/`).
- **Caché del mapa:** migración 5 (`project_components`, `project_deps`, `project_map_state`). Se reutiliza mientras la huella de los manifiestos no cambie y el Watcher no haya visto cambiar ninguno; si se recalcula, se guarda a través del escritor único.

Resultados en [`perf/2026-11-12-f4c.md`](perf/2026-11-12-f4c.md):

| Criterio | Resultado |
|---|---|
| **G8** (200 nodos) | **resuelta:** layout 4,6 ms, pintado 33 ms |
| Consultas < 100 ms | **pasa:** `diagnostics` 42 ms en frío y 12 ms en caliente; `history` 5–28 ms (7–90 días) |
| Caché del mapa | migración 4 → 5 sin errores sobre una copia de la base real; en 5 proyectos reales el mapa de la caché es idéntico al recalculado. **No acelera** el escaneo (≈ 30–40 ms con y sin ella): lo dominan `git ls-files` y el `stat` |
| Tests | 175 en verde (1 ignorado); fmt, clippy, typecheck y lint sin avisos |
| «Borrar rutas ahora» (§6.4.1) | **aplazado a F5**, con los ajustes de privacidad que aún no tienen interfaz. Desviación **aceptada por el usuario** (2026-11-12) |

## F5: lo que está hecho (2026-11-12)

Rama `feature/f5-hardening`. Commits `a85080a` (manifiesto, pánicos, instalador y guion de B9) y `8b324f3` (ajustes y «borrar rutas ahora»).

- **Aviso del enlazador resuelto:** el GCC de MinGW añadía un segundo manifiesto (`default-manifest.o`). `build.rs` lo sustituye por un objeto vacío y Tauri embebe uno solo con Common Controls v6, `asInvoker` y `longPathAware`.
- **Instalador NSIS** por usuario, sin administrador, con `WebView2Loader.dll`. NSIS 3.13 y `nsis_tauri_utils` se descargaron a `%LOCALAPPDATA%\tauri\NSIS` con autorización del usuario.
- **Ajustes de privacidad y retención** (§10) en `%APPDATA%\dev-cockpit\settings.json`, editables en Diagnóstico; se aplican al reiniciar.
- **«Borrar rutas ahora»** (§6.4.1) en Diagnóstico, con confirmación.
- **`panics.log`** junto a la base: hora, hilo y `fichero:línea`, sin el mensaje.
- **Tabla «Medido» de §11.2** rellena con los resultados de F1–F5.

Resultados en [`perf/2026-11-12-f5.md`](perf/2026-11-12-f5.md):

| Criterio | Resultado |
|---|---|
| **B10** instalador | **3,55 MiB** (objetivo < 15 MB); binario 10,3 MiB, bundle JS 304 KiB |
| **B8** reposo (5 min) | 194 MB en total y 6,3 MB el proceso Rust (objetivos < 250 y < 60); CPU 0,01 % (objetivo < 0,5 %); 298/300 muestras válidas |
| **B5** carga (30 min, 107 herramientas/min) | CPU **0,06 %** de media y 0,71 % de máximo (objetivo < 3 %); **224 MB** en total y 8,9 MB el proceso Rust; 3.210 de 3.210 herramientas guardadas; 1.799/1.800 muestras válidas |
| **C2** con Teams abierto (§16, paso 5) | de 18 procesos WebView2 del equipo, 5 propios y ninguno de Teams; los dos métodos coinciden en 60/60 muestras |
| Sección «Privacidad y retención» en la app | revisada con UI Automation y capturas; «Borrar rutas ahora» funciona de punta a punta |
| **B9** 24 h | **pendiente:** el usuario la lanza en otro equipo con `target\b9-kit` (instalador de `8b324f3`, `b9-soak.ps1` y `LEEME.txt`). Hay que traer `Escritorio\dev-cockpit-b9` y resumirla con `b9-soak.ps1 -Summarize -OutDir <carpeta>` |
| Tests | 180 en verde (1 ignorado); fmt, clippy, typecheck y lint sin avisos |

## Identidad visual: lo que está hecho (2026-11-13)

Rama `feature/visual-identity`, creada desde `feature/f5-hardening` porque toca `PrivacySettings.tsx`. La pidió el usuario («una de las cosas más importantes de este proyecto era la estética») con un encargo propio, sin propuesta previa: el sistema se definió desde cero y está en [`DESIGN.md`](DESIGN.md).

- **Sistema «Instrumento»:** el color solo indica estado (`accent`, `positive`, `attention`, `critical`, `neutral`), cifras tabulares, rejilla de 4 px, tres niveles de profundidad y movimiento solo para lo vivo. Tema claro y oscuro según el sistema.
- **Código:** tokens en `src/ui/tokens.css`; base, iconos SVG propios, tonos y componentes compartidos en `src/ui/`; barra lateral, cabecera de vista, avisos globales y `LiveEmpty` en `src/app/Shell.tsx`. Cada vista tiene su CSS al lado y ya no hay `App.css`.
- **Revisión:** 64 capturas en Edge headless con las respuestas IPC reales grabadas de la app (copia de la base) y escenarios simulados para vacío, cargando, error, sin conexión, lectura del historial, ajustes pendientes y anchos de 600 a 1.440 px; después, capturas de la app real por CDP en los dos temas. La pantalla estaba bloqueada, así que no sirvieron las capturas de ventana con UI Automation.
- **Arreglos que salieron de la revisión:** filas de herramientas que se salían de la tarjeta, métricas desalineadas, gráficos que escalaban el texto con la ventana (ahora 1:1 con `useWidth`), visor de diff que se abría fuera de la vista, tablas por proyecto y modelo recortadas, estados vacíos engañosos sin conexión y tarjetas de Diagnóstico vacías tras un error.
- **Comprobaciones:** typecheck, lint, fmt, clippy y tests en verde; el contraste se calculó para todos los pares de tokens (mínimo 4,80:1).

## Siguiente paso

**Cerrar F5** cuando llegue el resultado de B9: resumirlo, rellenar su fila en §11.2 y el informe, y fusionar `feature/f5-hardening` en `main` si cumple (crecimiento < 10 % y 0 pánicos). Si no cumple, investigar antes de fusionar. Después, fusionar `feature/visual-identity` (contiene F5).

Otra decisión pendiente del usuario: **qué hacer con los hooks `classic.TaskCreated`/`TaskCompleted` del mod.** Son redundantes con `TaskCreate`/`TaskUpdate`, y en el Desktop el motor los salta y avisa al modelo cada vez. Quitarlos es una desviación de §5.2. Mientras no decida, se quedan.

## Hallazgos que afectan a fases futuras

- **F4b:** en 2.1.289 y 2.1.321 el modelo usó `TaskCreate`, no `TodoWrite`. El adaptador A1 importa más de lo que decía la SPEC. En P2 se vio además que esos motores no ofrecen `TodoWrite` cuando están las herramientas de tareas.
- **Mod, hooks saltados:** en Desktop 2.1.321, un plugin del entorno de escritorio (`cc-plugin-sec-default`) se ejecuta antes que `classic.TaskCreated`/`TaskCompleted` y el motor salta los del mod, con una notificación que entra en el contexto del modelo. Puede pasar con otros hooks `classic.*` del mod.
- **Sesiones de prueba desde el Desktop:** las sesiones `-p` lanzadas desde una sesión del Desktop heredan su entorno (`CLAUDE_CODE_ENTRYPOINT=claude-desktop`, `CLAUDE_CODE_CHILD_SESSION=1`…). No son del todo iguales a una sesión abierta a mano.
- **F4b, subagentes de *workflow*:** sus ficheros no dicen su estado; 72 de los 99 subagentes del historial salen «sin estado».
- **F4b, sesiones antiguas:** las listas de sesiones que terminaron sin completar su plan conservan un paso «en curso». La vista solo muestra sesiones con actividad reciente, así que no se ve; habrá que tenerlo en cuenta en el histórico.
- **Reinicio de la app:** una sesión inactiva con el mod no se entera y aparece como «sin mod» hasta su siguiente evento.
- **`cost-state`:** los tokens deduplicados del transcript solo coinciden exactamente con `cost-state` en 19 de 33 sesiones.
- **Tabla `pricing` vacía:** no hay precios de modelos cargados.
- **Primera ejecución de F4a sobre la base real:** la compactación agregará todos los días cerrados y **purgará las rutas de más de 30 días** (D3). Es lo previsto, pero no se puede deshacer; las pruebas de F4a usaron una copia.
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
npm run tauri build                            (instalador en target\release\bundle\nsis\)
cargo run -p cockpit-ingest --release --example workspace_gates -- g5 g5.json   (y g7)
```

Scripts de medición en `scripts/measure/`: `gen-transcript.mjs` (B4), `sample-procs.ps1` (B8), `replay-transcript.mjs` (B5), `b9-soak.ps1` (B9), `app-cycle.ps1` (B11) y `workspace-cycle.ps1` + `fs-ops.mjs` (B6, B7).

## Restos en el equipo

- `%APPDATA%\dev-cockpit\cockpit.db`: base real de la app.
- `%LOCALAPPDATA%\com.devcockpit.app`: datos de WebView2 del identificador antiguo, sin uso.
- `target\e2e\`: ficheros de las pruebas E2E (ignorado por Git).
- Registros obsoletos en `~/.claude/sessions` de las sesiones de prueba cerradas de golpe; Claude los limpia solo al arrancar.
- Las pruebas con prompts reales consumieron tokens de la cuenta (alrededor de 0,50 USD estimados por sesión, unas cinco sesiones).
- `%LOCALAPPDATA%\tauri\NSIS`: NSIS 3.13 y `nsis_tauri_utils.dll`, que descarga `tauri build` para el instalador (autorizado el 2026-11-12).
- `target\b9-kit\`: instalador, guion e instrucciones de B9 (ignorado por Git).
- `%APPDATA%\dev-cockpit\endpoint.json`: quedó de una ejecución anterior que no se cerró limpiamente (estaba antes de F4a). La app lo reescribe al arrancar.
