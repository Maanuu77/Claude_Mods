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
| **F4a Workspace Watcher** | **Siguiente, sin empezar** | — |
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

## Siguiente paso

**F4a, Workspace Watcher** (SPEC §7, §6.4.1, §6.4.2): rama nueva `feature/f4a-workspace-watcher` desde `main`. Puertas G5 y G7; mediciones B6 y B7.

## Hallazgos que afectan a fases futuras

- **F4b:** en 2.1.289 y 2.1.321 el modelo usó `TaskCreate`, no `TodoWrite`. El adaptador A1 importa más de lo que decía la SPEC.
- **Reinicio de la app:** una sesión inactiva con el mod no se entera y aparece como «sin mod» hasta su siguiente evento.
- **`cost-state`:** los tokens deduplicados del transcript solo coinciden exactamente con `cost-state` en 19 de 33 sesiones.
- **Aviso del enlazador** `.rsrc merge failure: multiple non-default manifests`: sigue sin resolver; hay que mirarlo antes del instalador de F5.
- **Tabla `pricing` vacía:** no hay precios de modelos cargados.

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
```

Scripts de medición en `scripts/measure/`: `gen-transcript.mjs` (B4), `sample-procs.ps1` (B8) y `app-cycle.ps1` (B11).

## Restos en el equipo

- `%APPDATA%\dev-cockpit\cockpit.db`: base real de la app.
- `%LOCALAPPDATA%\com.devcockpit.app`: datos de WebView2 del identificador antiguo, sin uso.
- `target\e2e\`: ficheros de las pruebas E2E (ignorado por Git).
- Registros obsoletos en `~/.claude/sessions` de las sesiones de prueba cerradas de golpe; Claude los limpia solo al arrancar.
- Las pruebas con prompts reales consumieron tokens de la cuenta (alrededor de 0,50 USD estimados por sesión, unas cinco sesiones).
