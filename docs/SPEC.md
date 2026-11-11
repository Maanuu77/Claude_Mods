# DEV COCKPIT: especificación técnica v1.2.2 (arquitectura)

> **Estado:** v1.2.2, **APROBADA** como documento oficial de arquitectura (2026-11-10), con la revisión correctiva final de §0.4 incluida. La arquitectura y las tecnologías están aprobadas desde la v1.2. Las versiones v1.2.1 (§0.2), v1.2.2 (§0.3) y la revisión final (§0.4) solo cierran detalles: no cambian decisiones ni añaden funcionalidades.
> **Documentos relacionados:** [`CHANGELOG-spec.md`](CHANGELOG-spec.md) · [`DEVELOPMENT_WORKFLOW.md`](DEVELOPMENT_WORKFLOW.md) · [`adr/`](adr/), con los ADR 0001–0012.
> **Situación del repositorio:** solo documentación. Todavía no hay código de la aplicación. `main` no tiene commits hasta que se aplique el procedimiento W9 de §17.
> **Investigación:** 2026-11-10. Se basa en:
> - la documentación oficial (lista en §16);
> - las declaraciones de tipos que genera el motor Claude Code 2.1.321;
> - una inspección de solo lectura de `~/.claude`;
> - benchmarks reales de solo lectura ejecutados en este equipo (§11).
>
> **Leyenda de respaldo**, aplicada a cada funcionalidad:
> - **[DOC]**: API o contrato documentado.
> - **[INT]**: depende de un formato interno o no documentado de Claude Code.
> - **[EXP]**: necesita validación experimental antes de darla por buena. Cada punto [EXP] tiene una puerta de validación `G#` en §13.

---

## 0. Cambios v1 → v1.1

| # | Petición | Cambio | Decisiones v1 afectadas y justificación |
|---|---|---|---|
| 1 | Workspace Watcher | Nuevo componente §7: vigila los ficheros reales de los proyectos activos (crear, modificar, borrar, renombrar). Diffs en vivo mediante Git **solo lectura** (`--no-optional-locks`). No se guarda contenido en SQLite. | Amplía el principio v1 «solo lectura respecto a `~/.claude`» a **solo lectura respecto a todo** (proyectos y Git incluidos). |
| 2 | Live Tool Monitor | `tool.call`: emite `ToolStarted` **antes** de `await next(e)` y `ToolFinished` en un `finally` después (§5.2). El envío ya no espera al temporizador fijo: el flush es inmediato. | Sustituye «encolar tras `next`» y el flush periódico de 250 ms. Motivo: en v1 la herramienta solo se veía al terminar. |
| 3 | Task Monitor | Nueva fuente §8, con tareas declaradas por el propio agente (`TaskCreate`/`TaskUpdate`, `TodoWrite`, `update_plan` del Desktop) y estados de subagentes (`AgentStatus`). El progreso se calcula con tareas completadas / total, **nunca** con número de llamadas. | Añade una tabla, `tasks`. Las tablas de v1 se mantienen. |
| 4 | Project Map | Nuevo componente §9, ligero: árbol que respeta `.gitignore`, componentes detectados por manifiestos y dependencias declaradas. El análisis de imports se aplaza a v2. | Ninguna. |
| 5 | Compatibilidad | Nueva §12: matriz de motores reales (CLI 2.1.289 y Desktop 2.1.317/2.1.321), tipos generados por cada uno, validación con el binario de cada motor y versión mínima formal. | v1 solo citaba «≥ 2.1.287». Ahora hay un mínimo verificado. |
| 6 | Privacidad | Nueva §10: lista blanca de metadatos por herramienta, aplicada en el mod, en el sobre HTTP (`deny_unknown_fields`) y en el parser de transcripts. Argumentos y resultados nunca se guardan ni se transmiten. | Endurece la política v1 «métricas + metadatos». `path_hint` pasa de opcional a necesario (ruta **relativa**) para atribuir los cambios del Workspace Watcher. Está justificado en §10. |
| 7 | Rendimiento | `session.measure` (push) sustituye el sondeo de `$.session.usage()` cada 5 s. El flush pasa a hacerse bajo demanda. El registro de sesiones se vigila con `notify` en vez de sondearlo. El polling de transcripts se limita a los ficheros de sesiones activas. | Se eliminan 3 sondeos de v1. Lista en §11.3. |
| 8 | Presupuestos | §11 separa **Objetivo** de **Medido**. Incluye benchmarks reales ya ejecutados y la batería B1–B10 para la implementación. | Se conservan todos los objetivos de v1. Se añaden nuevos para el Watcher y para Git. |

## 0.1 Cambios v1.1 → v1.2

| # | Decisión o petición | Cambio | Secciones afectadas |
|---|---|---|---|
| D1 | **Cerrada:** se guardan títulos de tarea truncados a 120 caracteres. Nunca descripciones ni prompts. | Ajuste `privacy.capture_task_titles` (activado por defecto). Desactivado, el Task Monitor muestra «Tarea n.º k». | §8, §10, §15 |
| D2 | **Cerrada:** el nombre del ejecutable de `Bash` **no** se registra por defecto. | Ajuste `privacy.capture_shell_program` (desactivado por defecto). | §10 |
| D3 | **Cerrada:** 30 días de retención para los eventos individuales de cambio de fichero. Los agregados históricos se conservan a largo plazo. | Nueva tabla `file_change_daily` y tarea diaria de compactación y purga. | §6.4, §7.4, §10 |
| C1 | Reconexión automática del mod cuando DEV COCKPIT se abre con sesiones ya iniciadas, sin reiniciar Claude. | Máquina de estados de conexión, *resync* (`hello`) al reconectar y vigilancia de `endpoint.json` **solo mientras no hay conexión**. | §5.3, §5.4, §11.2, §11.3, §13, §15, §16 |
| C2 | Los benchmarks de memoria aíslan los procesos WebView2 de DEV COCKPIT. | Identificación por `GetProcessInfos` de WebView2, contrastada con el árbol de procesos y el `--user-data-dir`. Métrica: *private bytes*. | §11.2, §11.4 (B8, B9), §13 (G9) |
| — | Revisión de coherencia | Corregidas tres contradicciones de la v1.1: «0 timers» frente a la vigilancia de reconexión, «1–5 MB/mes» de `file_events` frente a la retención y «decisiones abiertas» frente a cerradas. | §6.4, §11.2, §15 |

## 0.2 Cambios v1.2 → v1.2.1 (sin cambios de arquitectura)

| # | Detalle | Cambio | Secciones |
|---|---|---|---|
| 1 | `POST /v1/hello` | Se documenta como ruta de `HttpIngest`, con su contrato completo: autenticación, petición, respuesta, política de privacidad e idempotencia. También se define la idempotencia de `/v1/events` (`eid` por instancia y secuencia). | §5.4, §6.2.1 |
| 2 | Retención coherente de rutas | **Todo** el detalle de rutas de fichero (`file_events.rel_path`, `tool_calls.rel_path`, `host` de WebFetch, rutas en `ingest_errors`; las rutas ya no pueden ir dentro de `meta_json`) tiene una retención de **30 días**. Los agregados se conservan indefinidamente **sin rutas individuales**. `file_change_daily` deja de guardar `top_dir`. Se añade `tool_usage_daily`. El ajuste `retention.file_events_days` se renombra a `retention.path_detail_days`. | §6.4, §7.4, §10 |
| 3 | Política previa al handshake | Hasta recibir la respuesta del primer `hello`, el mod captura y transmite con la política **más restrictiva**: sin títulos de tarea y sin rutas de ningún tipo, `cwd` incluido. Después aplica la política que autoriza la app. Lo que se pierda antes del handshake lo recupera la app desde el transcript, aplicando su propia política. | §5.4, §10 |
| 4 | G10 | Si falla, **se detiene el trabajo y se presentan alternativas**. Escribir en `~/.claude` queda **prohibido** sin autorización previa expresa, y ninguna alternativa se elige automáticamente. Se elimina la excepción a «solo lectura» que mencionaba la v1.2. | §4, §13.2 |
| 5 | Atribución por intervalo completo | El evento de fichero se relaciona con el intervalo `[ToolStarted − ε, ToolFinished + δ]` (o `[ToolStarted − ε, ahora]` si la herramienta sigue en curso), además de la coincidencia de ruta. Hay una ventana de asentamiento y reevaluación, se conserva `confidence` y G5 exige precisión medida. | §7.3, §13.2 (G5) |
| — | Reglas de trabajo | Nuevo `docs/DEVELOPMENT_WORKFLOW.md`, con la política de ramas, commits, pruebas, integración y autoría. | §15, §17 |

## 0.3 Cambios v1.2.1 → v1.2.2 (sin cambios de arquitectura ni funcionalidades nuevas)

| # | Corrección | Cambio | Secciones |
|---|---|---|---|
| 1 | Columna `host` | Definición completa y explícita de `tool_calls`, incluidas `host`, `root_id` y `day`. Se comprueba que coincide con la tabla de retención. | §6.4, §6.4.1 |
| 2 | Agregación idempotente | Claves primarias de `file_change_daily` y `tool_usage_daily`. Registro transaccional `aggregation_ledger` para saber qué días están agregados. Agregación por `(tabla, root_id, day)` en una transacción atómica. La purga solo actúa sobre días ya agregados. Los datos tardíos marcan el día como sucio. | §6.4.2 |
| 3 | Flujo de desarrollo | §17 formalizada con todas las reglas obligatorias, incluido el procedimiento para inicializar `main`, que no tiene commits. Se crea `docs/DEVELOPMENT_WORKFLOW.md`. | §15, §17 |
| 4 | Instalación del plugin | La instalación permanente (`claude plugin marketplace add` / `claude plugin install`) modifica `~/.claude`, así que **requiere tu autorización previa** y no se hace sin ella. Las pruebas usan `--plugin-dir`, que no instala nada. | §5.3, §12, §15 |

## 0.4 Revisión correctiva final de v1.2.2 (sin cambios de arquitectura, funcionalidades, dependencias ni fases)

| # | Corrección | Cambio | Secciones |
|---|---|---|---|
| R-1 | Autorización explícita para escribir en `~/.claude` | Cualquier sesión de prueba (G1, G10, compatibilidad) y cualquier comando `claude` cuyo efecto en `~/.claude` no esté verificado necesitan **autorización explícita, por prueba y por motor**. Avisar no basta y la autorización no se presupone: sin ella, la prueba no se ejecuta. | §4, §12, §15, §17 (W11) |
| R-2 | Normalización de `root_id` en la agregación | `COALESCE(root_id, 0)` en las agrupaciones, en la clave del registro, en la purga y en la reconciliación. Los triggers `AFTER UPDATE` marcan como sucios el target **anterior** y el **nuevo**. La purga no dispara triggers. Los días ya purgados de `file_events` son inmutables. | §6.4, §6.4.2 |
| R-3 | Referencia de F4a | Ahora apunta a §6.4.1 (retención) **y** a §6.4.2 (agregación y purga). | §15 |
| R-4 | Unicidad de `tool_use_id` | **No es único entre sesiones** (evidencia en M8, §11.1). La clave de `tool_calls` pasa a ser `(session_id, tool_use_id)`, con la marca `is_copy` para no contar dos veces las copias de historial. Con la misma evidencia se fija la sesión canónica de `api_messages`, que conserva su PK `message_id`. | §3, §6.3, §6.4, §6.4.2, §7.3, §11.1 |

---

## 1. Contexto y objetivo

Ahora mismo no hay ninguna vista unificada, en vivo e histórica, de las sesiones de Claude Code de este equipo: qué hacen, cuánto consumen, qué tareas llevan, qué ficheros tocan y cómo es el proyecto sobre el que trabajan. **DEV COCKPIT** es una app de escritorio para Windows (Tauri 2 + Rust + React/WebView2 + SQLite) que se apoya en un **mod ligero de Claude Code** como puente de telemetría.

| Eje | Decisión (confirmada por ti) |
|---|---|
| Capacidad | **Solo observabilidad.** No hay acciones sobre las sesiones ni escrituras en proyectos o en Git. |
| Rol del mod | **Puente + indicador mínimo** (`$.ui.status` y `/cockpit`). La app es la interfaz principal. |
| Distribución | **Uso personal, un solo Windows 11.** Instalador NSIS sin firmar y sin auto-update en v1. |
| Privacidad | **Métricas + metadatos**, con la lista blanca de §10. El contenido solo se lee bajo demanda y nunca se persiste. |

**Fuera de alcance en v1:** macOS/Linux, multiusuario, lanzar u orquestar sesiones (Agent SDK), facturación real, búsqueda de texto completo, análisis de imports o arquitectura avanzada (v2) y OTel (v1.1+, como verificación cruzada).

---

## 2. Datos verificados del entorno (inspección de solo lectura)

| Hecho | Valor medido | Implicación |
|---|---|---|
| Motores instalados | CLI `~/.local/bin/claude` → **2.1.289** (también hay 2.1.285 en `versions/`). Desktop: `%APPDATA%\Claude\claude-code\` → **2.1.317** y **2.1.321**. | Hay que validar con 3 motores reales. La 2.1.285 queda **por debajo** del mínimo documentado de mods (2.1.287). |
| `claude plugin validate` / `plugin test` | Presentes en 2.1.289, 2.1.317 y 2.1.321 | Se puede validar el mod con el binario de cada motor. |
| Rust / Cargo | **No instalados** en el momento de la inspección. Instalados en F0 (2026-11-11): rustc/cargo 1.99.1, target `x86_64-pc-windows-gnu`, con MinGW-w64 de WinLibs (gcc 16.2.0). | Prerrequisito de F0. Sin permisos de administrador no se puede instalar MSVC (§15, F0). |
| Node / Git | Node v22.23.4 · Git 2.51.2.windows.1 | Git ≥ 2.15 soporta `--no-optional-locks`. |
| `~/.claude/projects` | 521 MB en disco. **471,7 MB en 144 JSONL**, 82.146 líneas, 0 líneas corruptas. | El backfill tiene que ser incremental. |
| JSONL más grande | 42 MB. Línea p50 = 1,6 KB; **p99 = 640 KB; máx = 854 KB**. | Parseo selectivo y por streaming. |
| Filas `assistant` | 894 filas para **361 `message.id` únicos** con `usage` idéntico. En total, 9.925 mensajes únicos. | **Deduplicar por `message.id`**: si no, los tokens salen unas 2,5 veces inflados. |
| Tipos de línea | `user`, `assistant`, `attachment`, `system:{api_error,stop_hook_summary}`, `queue-operation`, `file-history-*`, `custom-title`, `agent-name`, `last-prompt`, `cost-state` | **[INT]**: el parser debe ser tolerante y detectar deriva de esquema. |
| Subagentes | `projects/<p>/<session>/subagents/agent-<id>.jsonl` (+ `.meta.json`) | Vigilancia recursiva. |
| Registro de sesiones vivas | `~/.claude/sessions/<pid>.json` con `pid`, `sessionId`, `cwd`, `status`, `kind`, `entrypoint`, `updatedAt` y una named pipe | **[INT]**: señal opcional de liveness. |
| `cwd` por sesión | 78 `cwd` distintos en 12 «proyectos», incluidos subdirectorios y **worktrees** (`.claude/worktrees/...`) | La raíz del proyecto se resuelve con `git rev-parse --show-toplevel`, no con el `cwd`. Los worktrees cuentan como raíces propias. |
| Uso real de herramientas de tareas | `TaskCreate`/`TaskUpdate` = **0**, `TodoWrite` = **35**, `mcp__ccd_turn__update_plan` (Desktop) = **426** | El Task Monitor necesita **tres adaptadores**. En este equipo, la fuente dominante es la del Desktop, que es **[INT]**. |
| Ficheros versionados frente a ficheros en disco | Benchify 565 / 35.563 · MyOrganizationSchedule 139 / 70.502 · Python 118 / 7.322 | El Watcher **tiene que** respetar `.gitignore` y filtrar `node_modules`/`target`/`dist`. |

---

## 3. Fuentes de datos: capacidades reales y elección

| Fuente | Latencia | Respaldo | Qué aporta | Uso |
|---|---|---|---|---|
| **Mod** (`hooks.json → modules`, `on(event, hook)`) | Push en el propio proceso | **[DOC]**, pero *early access* («puede cambiar entre releases»). Tipos generados por build. | `tool.call` (`tool`, `tool_use_id`, `agentId`, `isError`), `turn.start`/`turn.complete` (`turnId`, `durationMs`, `usage` + `model`, `isAborted`, `agentId`), `session.start`/`session.end`, **`session.measure`** (contexto, rate limits, coste; push), `$.agent.list()` (`AgentStatus`), `classic.TaskCreated`/`TaskCompleted`/`SubagentStart`/`SubagentStop`, `$.session.version()` | **Primaria en vivo** |
| **Transcripts JSONL** | 0,2–2 s (tail) | **[INT]**: la documentación avisa de que «cambia entre versiones» | Histórico, `usage` por mensaje, `tool_use`, subagentes, errores, `cost-state` | **Fuente de verdad del histórico y del backfill**, y **modo degradado** cuando no hay mod |
| **Ficheros del proyecto** | ~50–500 ms (debounce) | **[DOC]** Win32 `ReadDirectoryChangesW` mediante `notify` | Creaciones, modificaciones, borrados y renombrados reales | Workspace Watcher §7 |
| **Git CLI, solo lectura** | 70–700 ms por consulta (medido) | **[DOC]** git-scm (`--no-optional-locks`, `--porcelain=v2`, `--numstat`) | Estado, diffs, rama, HEAD | Workspace Watcher §7 |
| **Manifiestos del proyecto** | Bajo demanda | Formatos públicos de cada ecosistema **[DOC]** | Componentes y dependencias declaradas | Project Map §9 |
| Registro `sessions/<pid>.json` | Evento de `notify` | **[INT]** | Liveness y busy/idle | Auxiliar, desactivable |
| OpenTelemetry | 5 s (logs) / 60 s (métricas) | **[DOC]** | `api_request.cost_usd`, `tool_result.duration_ms` | **Diferido** (verificación cruzada) |
| Hooks clásicos `http`/`command` | Push síncrono | **[DOC]** | Equivalente al mod, pero arranca un proceso por evento | **Plan B** si se rompe la API de mods |
| Status line | 300 ms | **[DOC]** | `rate_limits`, `context_window` | **Descartado**: el slot es único |
| Agent SDK | Stream | **[DOC]** | Sesiones lanzadas por la app | **Fuera de alcance** |

**Reglas de reconciliación** (sin cambios desde v1, con dos añadidos):
- **Tokens por mensaje y modelo:** manda el transcript, deduplicado por `message.id`. El `usage` de `turn.complete` sirve para la vista en vivo hasta que llega la fila del transcript.
- **Contexto %, rate limits y coste «como `/cost`»:** solo los da el mod, mediante `session.measure`. Sin mod se muestra «n/d», o el coste estimado con la tabla de precios, marcado como *estimación*.
- **Herramientas:** identidad `(session_id, tool_use_id)` y upsert idempotente (R-4, §6.4). El mod aporta inicio, fin, duración y error. El transcript confirma y rellena huecos. Las copias de historial en otras sesiones se marcan `is_copy` y no cuentan en los agregados.
- **Coste:** siempre es una **estimación del cliente**. Prioridad: mod `cost.usd` > `cost-state.totalCostUSD` [INT] > `usage × pricing`.
- *(nuevo)* **Tareas:** identidad `(session_id, source, task_id)`. Manda el último evento por orden temporal. Ver §8.
- *(nuevo)* **Cambios de fichero:** el evento del sistema de ficheros es la verdad. La atribución a Claude es una inferencia con confianza explícita (§7.3).

---

## 4. Arquitectura general

```
┌──────────── Claude Code (CLI 2.1.289 / Desktop 2.1.317–2.1.321) ──────────────┐
│ mod "dev-cockpit-bridge" (sin DOM ni Node; todo pasa por $)                    │
│  tool.call ─► ToolStarted ─► await next(e) ─► finally ToolFinished             │
│  turn.* / session.* / session.measure / classic.Task* / Subagent*              │
│  cola acotada ─► flush bajo demanda ($.clock.after) ─► $.http.fetch POST       │
│  $.ui.status("◉ cockpit · ctx 43% · 5h 37%")   /cockpit (diagnóstico)          │
└───────────────────────┬───────────────────────────────────────────────────────┘
                        │ HTTP/JSON · Bearer · 127.0.0.1 · lista blanca
                        │ POST /v1/hello · POST /v1/events · GET /v1/health
┌───────────────────────▼──────── DEV COCKPIT · proceso Rust (Tauri 2.12) ──────┐
│ Ingesta (tokio)                                                               │
│  ├ HttpIngest (axum)              ├ TranscriptTailer (notify + offsets)       │
│  ├ SessionRegistryWatch (notify)  ├ WorkspaceWatcher (notify + ignore)  §7    │
│  └ GitReader (git CLI, solo lectura, 1 en vuelo por repo)               §7    │
│        ▼ mpsc<RawEvent> acotado (back-pressure)                               │
│ Normalizer → CockpitEvent + clave idempotente + PrivacyFilter §10             │
│        ▼ broadcast                                                            │
│  ├ LiveState (agregados, ring buffers, tareas, herramientas en curso)         │
│  ├ DbWriter (hilo único, rusqlite WAL, lotes de 250 ms o 500 eventos)         │
│  ├ ProjectMapper (manifiestos + árbol, cache invalidada por eventos)    §9    │
│  └ UiPublisher (Channel, deltas coalescidos cada 100 ms, solo con ventana     │
│    visible)                                                                   │
│ Comandos Tauri de solo lectura · bandeja · single-instance                    │
└───────────────────────┬───────────────────────────────────────────────────────┘
                        │ invoke (consultas) + Channel (stream ordenado)
┌───────────────────────▼──────── WebView2 · React 19 + Vite + TS ──────────────┐
│ Zustand (vivo) · TanStack Query (histórico) · TanStack Virtual · uPlot ·      │
│ React Flow (mapa)                                                             │
└───────────────────────────────────────────────────────────────────────────────┘
```

Principios (se mantienen los de v1 y se amplía el 3):
1. **Rust es la única fuente de estado.** El WebView es una vista desechable. En Windows `backgroundThrottling` no está soportado y los timers de JS se frenan cuando la ventana está oculta.
2. **La app funciona sin el mod** (solo transcripts). **El mod funciona sin la app**: falla en silencio, la cola es acotada y nunca bloquea la sesión.
3. **Solo lectura absoluta:** sobre `~/.claude`, sobre los ficheros de proyecto y sobre Git (§7.2). La app solo escribe en `%APPDATA%\dev-cockpit\`. **No hay excepciones.** Cualquier escritura en `~/.claude` necesita tu autorización previa y expresa (§13.2, G10). La regla vale igual para el **proceso de desarrollo y pruebas**: ninguna sesión de prueba ni ningún comando que pueda escribir en `~/.claude` se ejecuta sin tu autorización explícita (§12, §15, W11).
4. **Push antes que polling** *(nuevo)*: cada sondeo que queda está justificado en §11.3.

---

## 5. Componente A: mod `dev-cockpit-bridge`

### 5.1 Estructura (contrato oficial de mods) [DOC]
```
mods/dev-cockpit-bridge/
  .claude-plugin/plugin.json       {name, version, description, types, userConfig}
  .claude-plugin/marketplace.json  (marketplace de carpeta local)
  hooks/hooks.json                 {"modules": ["./register.ts"]}
  hooks/register.ts                export const register: Register = (on, options) => …
  hooks/privacy.ts                 lista blanca de metadatos por herramienta (§10)
  hooks/*.test.ts                  claude plugin test
  types/index.d.ts                 contrato de $.state (si se usa)
compat/engines/<versión>/claude-code.d.ts   tipos generados por cada motor (§12)
```

### 5.2 Hooks: todos de observación pura, ninguno bloqueante

| Evento | Comportamiento | Respaldo |
|---|---|---|
| `tool.call` | **Ver el pseudocódigo de abajo.** `ToolStarted` antes de `next`, `ToolFinished` en `finally`. | [DOC] |
| `turn.start` / `turn.complete` | `{turnId, agentId, durationMs, isAborted, usage{input, output, cache_read, cache_creation, model}}`. En cada uno también se llama a `$.agent.list()` (por evento, no por temporizador). | [DOC] |
| `session.measure` | Envía `UsageSnapshot{context, rateLimits, cost, changed}`. Sustituye el sondeo de 5 s de v1. | [DOC] · disponibilidad en 2.1.289 → **G1 [EXP]** |
| `classic.TaskCreated` / `classic.TaskCompleted` | `{task_id, subject (§10), teammate?}` | [DOC] |
| `classic.SubagentStart` / `classic.SubagentStop` | `{agent_id, agent_type}` | [DOC] |
| `session.start` / `session.end` | Lee el endpoint, encola `SessionStarted{cwd, surface, version, isInteractive}` y hace el flush final dentro del límite de tiempo de `session.end`. | [DOC] |
| `command.run {command:'cockpit'}` | `/cockpit`: estado de conexión, cola, descartes, último envío y versión del motor | [DOC] |

```ts
// Pseudocódigo normativo del Live Tool Monitor (no es código final)
on('tool.call', async ($, e, next) => {
  const meta = extractMeta(e.tool, e)          // §10: lista blanca; nunca e.input entero
  enqueue({ k: 'tool_started', id: e.tool_use_id, agentId: e.agentId, tool: e.tool, meta, t: $.clock.now() })
  scheduleFlush($)                              // flush inmediato, sin esperar a ningún timer
  let status: 'ok' | 'error' | 'interrupted' = 'error'
  try {
    const r = await next(e)                     // la herramienta se ejecuta aquí; el presupuesto propio del hook no corre
    status = r.isError ? 'error' : 'ok'
    return r                                    // devuelve el resultado intacto: nunca deny ni rewrite
  } catch (err) {
    status = next.signal.aborted ? 'interrupted' : 'error'
    throw err                                   // no se altera el comportamiento del motor
  } finally {
    enqueue({ k: 'tool_finished', id: e.tool_use_id, status, t: $.clock.now(), resultMeta: resultMeta(e.tool, status) })
    scheduleFlush($)
  }
})
```

- **«Trabajando» visible al instante:** la UI pinta la herramienta en estado *running* con `ToolStarted` y calcula el tiempo transcurrido en local (rAF, solo con la ventana visible). No pide nada a Rust.
- **Herramientas huérfanas** (hay `started` pero no llega `finished` por un reload del mod o un cierre): las cierra el `tool_result` del transcript, o `session.end` las marca como `unknown`.
- **Llamadas en paralelo:** cada una se identifica por su propio `tool_use_id`.
- **[EXP] G3:** hay que comprobar si el tiempo que `tool.call` pasa en `next` incluye la espera del diálogo de permisos. Si la incluye, se añade el estado `awaiting_permission` observando `classic.PermissionRequest` (sin responder).

### 5.3 Envío y cola
- **Flush bajo demanda:** cuando la cola pasa de vacía a no vacía, `$.clock.after(50)` programa **un** `$.http.fetch(POST /v1/events)` con todo el lote. No hay temporizador permanente. Con la sesión inactiva y conectado, el mod tiene 0 timers. Desconectado, tiene como máximo uno: la vigilancia de §5.4.
- **Cola** acotada a 2.000 eventos: descarta los más antiguos y cuenta los descartes. Los `tool_started` y `tool_finished` tienen prioridad sobre los snapshots.
- **Si el envío falla:** se pasa al estado `disconnected` (§5.4). El endpoint se vuelve a leer en cada fallo.
- **Presupuesto del hook** [DOC]: 10 s de tiempo propio (las llamadas a `$` y a `next` no cuentan). Los hooks solo encolan; el `fetch` se hace fuera del camino crítico.
- **Descubrimiento del endpoint:** la app escribe `%APPDATA%\dev-cockpit\endpoint.json` con `{port, token, pid, schema}`. El mod lo lee con `$.fs.read`.
- **Indicador** `$.ui.status(...)`. Se mantiene v1: no hay banda ni pane en v1.1.
- **Instalación permanente:** marketplace de carpeta local (`claude plugin marketplace add <repo>/mods`, después `claude plugin install dev-cockpit-bridge`), a nivel de usuario, para que cargue en el CLI y en las sesiones locales del Desktop.
  - **v1.2.2:** estos comandos **modifican `~/.claude`** (`plugins/known_marketplaces.json`, la caché de plugins y `enabledPlugins` en `settings.json`). Por eso la instalación permanente **requiere tu autorización previa y expresa** en el momento de hacerla, y **no se ejecuta nunca sin ella**. Si no la autorizas, la app funciona en modo degradado (solo transcripts, §4.2).
  - Durante el desarrollo y las pruebas (G-pre, F3), el mod se carga **sin instalarlo**, con `claude --plugin-dir <carpeta>` (§12).
  - No se usa la carpeta de mods de la sesión (`~/.claude/dev-mods/…`), porque está dentro de `~/.claude`.

### 5.4 Conexión y reconexión automática (C1)

El mod se carga con la sesión **tenga o no la app abierta**. La conexión con la app es un estado que el mod gestiona solo. **Nunca hace falta reiniciar Claude Code** para conectar.

```
            ┌──────── evento/flush OK ────────┐
            ▼                                 │
  [connected] ──fallo (ECONNREFUSED/401/timeout)──► [disconnected]
      ▲                                               │ vigilancia ligera:
      │  hello OK                                     │ $.clock.every(15 s) → $.fs.stat(endpoint.json)
      │                                               │ (además: intento inmediato si llega un evento)
  [resyncing] ◄── endpoint.json nuevo (pid/mtime distintos) y /v1/health 200 ──┘
```

| Escenario | Comportamiento | Tiempo hasta conectar |
|---|---|---|
| **S1** Claude ya en marcha, la app se abre después | La app escribe `endpoint.json`. El mod, en `disconnected`, lo detecta en su siguiente `stat` (≤ 15 s) o en el siguiente evento, lo que llegue antes. | **≤ 15 s** sin actividad; **≤ 1 s** tras el siguiente evento |
| **S2** La app se reinicia (puerto y token rotados) | El siguiente envío recibe `ECONNREFUSED` o `401`, así que se vuelve a leer el endpoint al momento y se reintenta. Si no existe todavía, se pasa a `disconnected` (S1). | ≤ 1 s si la app ya escribió el endpoint; si no, ≤ 15 s |
| **S3** La app se cierra o muere | Cola acotada (2.000 eventos, se descartan los más antiguos). La sesión no se ve afectada nunca. | No aplica |
| **S4** La app se cierra limpiamente | Borra `endpoint.json` y el mod pasa a `disconnected` en el siguiente fallo. Si muere de forma abrupta, `endpoint.json.pid` apunta a un proceso inexistente. El mod no puede comprobarlo (sin Node), así que lo resuelve el fallo de conexión. | No aplica |

**Resync al reconectar (`POST /v1/hello`, idempotente).** El contrato completo está en §6.2.1. El mod envía:
1. la identidad de la sesión y del motor;
2. `$.session.usage()` como snapshot, porque `session.measure` solo se dispara con cambios;
3. `$.agent.list()`;
4. las **herramientas en curso**, mantenidas en un `Map` del módulo desde `ToolStarted` hasta `ToolFinished`, para que la UI las pinte como *running* con su hora de inicio real.

Después, y solo si la respuesta es `200`, vacía la cola acumulada por `/v1/events`.

**Política de privacidad previa al handshake (v1.2.1).** Desde que se carga el mod hasta que recibe la respuesta `200` de su **primer** `hello` en la sesión, la política activa es `PRE_HANDSHAKE`, la más restrictiva:
- `capture_task_titles = false`
- `record_paths = false` (ni `rel_path`, ni `cwd`, ni `host`)
- `capture_shell_program = false`

Esta política se aplica **en el momento de la captura**: los eventos encolados antes del handshake nunca contienen títulos ni rutas, ni siquiera en la memoria del mod. Tras el primer `hello` correcto, el mod adopta la política de la respuesta (`privacy`) para lo que capture después. Lo capturado antes no se «mejora» con datos que nunca se guardaron. Esos huecos (rutas de herramientas y títulos de tareas) los rellena la app desde el transcript, con su `PrivacyFilter` y su política, así que el resultado final es el mismo. La política autorizada **no se cachea entre sesiones**: cada sesión empieza en `PRE_HANDSHAKE`. Un `hello` posterior (reconexión) puede cambiar la política, y el cambio se aplica desde ese momento.

**La parte de la app:**
- Al arrancar, descubre las sesiones ya en curso por el registro `sessions/*.json` y por el backfill de transcripts. Aparecen **en modo degradado al instante** y se enriquecen cuando llega el `hello` de cada mod.
- Diagnóstico muestra por sesión: «mod conectado», «mod sin conectar (esperando ≤ 15 s)» o «mod no instalado / motor no soportado».

**Lo que no cubre:**
- Instalar o actualizar el mod **durante** una sesión exige `/reload-plugins` en esa sesión. Es una instalación, no una reconexión, y Claude Code lo documenta así.
- Las sesiones con motor < `minEngine` se quedan en modo degradado.

**Hot-reload del mod:** `register` se ejecuta otra vez, así que se pierden la cola y el `Map` de herramientas en curso, y el mod vuelve a `disconnected`, lo que provoca un `hello` inmediato. Las herramientas que estaban en curso se cierran con el transcript (§5.2, herramientas huérfanas).

---

## 6. Componente B: núcleo Rust (se mantiene v1 y se amplía)

**Workspace Cargo:**
```
crates/cockpit-core/    dominio, parsers JSONL, normalizador, PrivacyFilter, precios, agregación, detectores de manifiestos
crates/cockpit-store/   rusqlite (bundled), migraciones, DbWriter, consultas
crates/cockpit-ingest/  TranscriptTailer, SessionRegistryWatch, WorkspaceWatcher, GitReader, HttpIngest
src-tauri/              setup, comandos, Channels, bandeja, single-instance
```

**Versiones fijadas** (registros a 2026-11-10):
- `tauri` 2.12.x (no la 3.0-alpha).
- `notify` 8.2 + `notify-debouncer-full` 0.7.
- `ignore` (el filtro de ripgrep, para `.gitignore`). Versión exacta → F0.
- `rusqlite` 0.41 `bundled`, `axum` 0.8, `tokio` 1.5x, `serde_json` 1.0, `sysinfo`, `tracing`.
- Plugins: `single-instance`, `window-state`, `notification`, `log`.
- **No** se usan `plugin-fs`, `plugin-shell` ni `plugin-sql` (igual que en v1: no se expone SQL ni se dan permisos de fs/shell al WebView).

### 6.1 TranscriptTailer [INT]
Se mantiene v1:
- offsets `(file_id, size, offset, mtime)`;
- apertura con lectura, escritura y borrado compartidos;
- se retiene la última línea si llega incompleta;
- se detectan truncados y rotaciones;
- parseo selectivo con `IgnoredAny`;
- PK `message_id`;
- detector de deriva de esquema;
- backfill de lo más reciente a lo más antiguo, con progreso.

Cambios:
- Polling de respaldo de 2 s **solo para los JSONL de sesiones activas**. Los demás dependen únicamente de `notify`, con un reescaneo completo cada 60 s o cuando llega un `Rescan` (§11.3).
- Los bloques `tool_use` pasan por el **mismo `PrivacyFilter`** que el mod (§10). Del `tool_result` solo se leen `tool_use_id` e `is_error`.

### 6.2 HttpIngest
Se mantiene v1:
- `127.0.0.1`, puerto aleatorio, Bearer de 256 bits rotado en cada arranque;
- rechazo de cualquier petición con cabecera `Origin`;
- cuerpo máximo de 512 KB;
- rutas: **`POST /v1/hello`** (v1.2.1), `POST /v1/events` y `GET /v1/health`.

*(nuevo)* El esquema de cada evento usa `deny_unknown_fields`. Si un evento trae campos fuera de la lista blanca, **se rechaza el evento entero**, así que la privacidad se aplica también en esta frontera.

#### 6.2.1 Contrato de las rutas (v1.2.1)

**Autenticación común a todas las rutas:**
- `Authorization: Bearer <token>`, con el token de `endpoint.json`.
- Comparación en tiempo constante.
- Petición con cabecera `Origin` → `403`. Sin token, o token incorrecto → `401`. Esto indica al mod que debe volver a leer el endpoint (§5.4, S2).
- `Content-Type: application/json` obligatorio; si falta, `415`.
- `Host` debe ser `127.0.0.1:<port>`; si no, `421`, lo que protege contra DNS rebinding.
- Cuerpo de más de 512 KB → `413`. Esquema inválido o campos fuera de la lista blanca → `422`.

**`POST /v1/hello`: handshake y resync**

Petición. Se envía siempre bajo la política activa, que en el primer `hello` es `PRE_HANDSHAKE`, así que no lleva rutas ni títulos:
```json
{
  "schema": 1,
  "mod": { "name": "dev-cockpit-bridge", "version": "0.1.0", "instance_id": "<uuid v4 por carga del módulo>" },
  "engine": { "version": "2.1.321", "base": "2.1.321" },
  "session": { "id": "<session id>", "surfaces": ["desktop"], "is_interactive": true, "started_at": 1762790000000 },
  "reason": "startup | reconnect | reload",
  "hello_seq": 3,
  "applied_policy": { "id": "pre_handshake | <policy_id>", "capture_task_titles": false, "record_paths": false, "capture_shell_program": false },
  "snapshot": {
    "usage": { "context": { "tokens": 0, "window": 0, "percent": 0 }, "rate_limits": [ { "kind": "five_hour", "percent_used": 0, "resets_at": "…" } ], "cost_usd": 0 },
    "agents": [ { "id": "…", "type": "…", "status": "running", "parent_id": null } ],
    "running_tools": [ { "tool_use_id": "…", "tool": "Edit", "agent_id": null, "started_at": 0, "meta": {} } ]
  },
  "queue": { "pending": 12, "dropped": 0, "next_seq": 120 }
}
```
- `meta` de `running_tools` sigue la lista blanca de §10 y la política `applied_policy`. Con `PRE_HANDSHAKE` es `{}` o, como mucho, contiene booleanos.
- **No lleva** `cwd` mientras `record_paths = false`. La app resuelve la raíz del proyecto por `session.id`, consultando `sessions/<pid>.json` o el campo `cwd` del transcript.

Respuesta `200`:
```json
{
  "schema": 1,
  "server": { "app_version": "0.1.0", "accepted_schemas": [1] },
  "privacy": { "policy_id": "p-2026-11-10-1", "capture_task_titles": true, "record_paths": true, "capture_shell_program": false },
  "session": { "known": true, "first_seen": "…" },
  "events": { "ack_seq": 107 },
  "min_engine": "2.1.289", "engine_supported": true
}
```
- `privacy` es la política **autorizada** por la configuración de la app (§10). El mod la aplica a todo lo que capture desde ese momento.
- `events.ack_seq` es la última secuencia de esta `instance_id` que la app ya ha guardado. El mod descarta de su cola los eventos con `seq ≤ ack_seq` antes de enviar el resto.
- Si `engine_supported` es `false`, el mod pasa a `disconnected` permanente para esa sesión y `/cockpit` explica el motivo.

**Idempotencia:**
- `hello` es un **upsert** por `session.id`. Repetirlo con el mismo contenido no crea filas nuevas.
- La clave de deduplicación es `(instance_id, hello_seq)`. Un `hello_seq` menor o igual al último visto se responde `200` con el mismo cuerpo y no hace nada más.
- El snapshot `usage` se registra en `usage_snapshots` solo si es distinto del último.
- `agents` se fusiona por `id`.
- `running_tools`: cada herramienta **se inserta** como `running` si no existía. Una herramienta que la app tiene como `running` y **no aparece** en el snapshot **no se cierra**: tras un hot-reload, el `Map` del módulo está vacío. Solo la cierran `ToolFinished`, el `tool_result` del transcript o `session.end` (que la marca `unknown`).

**`POST /v1/events`: lote de eventos**
- Petición: `{ "schema": 1, "instance_id": "…", "session_id": "…", "events": [ { "seq": 108, "kind": "tool_started", … } ] }`.
- Respuesta `200 { "ack_seq": 115 }`.
- `409` si la `instance_id` no ha hecho `hello`. El mod repite el `hello` y después reintenta.
- **Idempotencia:** `eid = (instance_id, seq)`. Las secuencias son estrictamente crecientes por instancia. La app guarda la marca de agua (`ack_seq`) por `instance_id` e ignora cualquier evento con `seq ≤ ack_seq`. Un reintento de un lote que ya se guardó no duplica nada.
- Además, cada evento conserva su clave de dominio (`tool_use_id`, `turnId`, `message_id`…), con upsert (§6.3).

**`GET /v1/health`**
- `200 { "ok": true, "app_version": "…", "schema": [1] }`.
- `?debug=procs` añade los PIDs de WebView2 (C2).

### 6.3 Modelo canónico
`CockpitEvent { source: Mod|Transcript|Registry|Fs|Git, session_id?, agent_id?, ts, engine_version?, kind }`

Valores de `kind`:
- `SessionStarted`, `SessionEnded`, `TurnStarted`, `TurnCompleted`
- `ToolStarted`, `ToolFinished`
- `ApiMessage`, `ApiError`
- `UsageSnapshot`
- `TaskUpserted`, `AgentStatusChanged`
- `FileChanged{root, rel_path, change: Created|Modified|Deleted|Renamed{from}, size?, actor}`
- `GitStateChanged{root, branch, head, dirty_count}`
- `Liveness`

Claves idempotentes de dominio:
- `(session_id, tool_use_id)` para herramientas (R-4);
- `message_id` para mensajes de API;
- `(session_id, turnId)` para turnos;
- `(session_id, source, task_id)` para tareas;
- `(instance_id, seq)` para el transporte (§6.2.1).

### 6.4 SQLite (`%APPDATA%\dev-cockpit\cockpit.db`)
Se mantiene v1: WAL, `synchronous=NORMAL`, un solo hilo escritor y un pool de 2 lectores.

**Tablas de v1 (se mantienen):** `projects`, `sessions`, `agents`, `turns`, `api_messages`, `tool_calls`, `usage_snapshots`, `api_errors`, `file_offsets`, `ingest_errors`, `pricing`, `schema_meta`.

**Cambios:**
- `projects(id, root, git_dir?, is_worktree, main_root?, last_active)`: la raíz se resuelve con Git.
- `tool_calls`, definición completa (v1.2.2):
  ```
  tool_calls(
    session_id    TEXT NOT NULL REFERENCES sessions(id),
    tool_use_id   TEXT NOT NULL,                         -- 'toolu_…' (no único entre sesiones: M8)
    is_copy       INTEGER NOT NULL DEFAULT 0,            -- 1 = copia de historial de otra sesión (R-4)
    agent_id      TEXT NULL,
    root_id       INTEGER NULL REFERENCES projects(id),  -- dimensión de agregación
    tool          TEXT NOT NULL,                         -- 'Edit', 'Bash', 'mcp__srv__tool'…
    status        TEXT NOT NULL,                         -- running|ok|error|interrupted|unknown
    started_at    INTEGER NOT NULL,                      -- ms (reloj de pared)
    finished_at   INTEGER NULL,
    duration_ms   INTEGER NULL,
    day           TEXT NOT NULL,                         -- fecha civil local 'YYYY-MM-DD' de started_at, fijada al insertar
    rel_path      TEXT NULL,                             -- detalle de ruta: retención 30 días (§6.4.1)
    host          TEXT NULL,                             -- host de WebFetch: retención 30 días (§6.4.1)
    meta_json     TEXT NULL,                             -- solo la lista blanca de §10, NUNCA rutas ni host
    source_mask   INTEGER NOT NULL,                      -- bits: mod | transcript
    PRIMARY KEY (session_id, tool_use_id)                -- R-4
  )
  INDEX (tool_use_id), INDEX (session_id, started_at), INDEX (day), INDEX (started_at) WHERE rel_path IS NOT NULL OR host IS NOT NULL
  ```
  **Copias de historial (R-4):**
  - Cuando se inserta `(S, T)` y ya existe `(S', T)` en otra sesión, la ocurrencia **canónica** es la de la sesión con el `sessions.started_at` más antiguo, la que ejecutó la herramienta. Las demás quedan con `is_copy = 1`.
  - Si la nueva fila resulta ser la canónica (algo posible porque el backfill va de lo más reciente a lo más antiguo), se intercambian las marcas en la misma transacción, y el trigger de §6.4.2 marca como sucios los targets afectados.
  - Los agregados y las vistas de actividad **excluyen** `is_copy = 1`. La vista de una sesión reanudada o bifurcada sí muestra su historial copiado, identificado como tal.
- `api_messages(message_id PK, session_id, …)`: **se mantiene la PK `message_id`** de v1. La misma respuesta de la API copiada en otra sesión es un único consumo de tokens (M7, M8). `session_id` es la sesión **canónica**, con la misma regla de la más antigua: `ON CONFLICT(message_id) DO UPDATE SET session_id = excluded.session_id WHERE` la sesión entrante empezó antes. Así se evita que el backfill, que va de lo más reciente a lo más antiguo, atribuya los tokens a la sesión copia.
- `file_events.actor_tool_use_id` siempre va acompañado de `actor_session` (la referencia es compuesta, R-4).
  Las rutas y el host **solo** existen en las columnas `rel_path` y `host`. Por eso la purga de §6.4.1 es una única sentencia `UPDATE` y no hay que analizar JSON. `PrivacyFilter` rechaza cualquier `meta_json` que contenga claves de ruta (`path`, `file_path`, `url`, `host`, `cwd`…).
- `file_events` también lleva `day` (fecha civil local fijada al insertar), para que la agregación sea determinista aunque cambie la zona horaria o el horario de verano. Su `root_id` es `NOT NULL`, porque el Watcher siempre conoce la raíz.
- `tasks(session_id, source, task_id, subject?, status, owner?, blocked_by_json, created_at, updated_at, completed_at)`, con PK `(session_id, source, task_id)`.
- `file_events(root_id, rel_path, change, from_rel_path?, size?, ts, actor, actor_session?, actor_tool_use_id?, confidence)`: **sin contenido**. **Retención: 30 días (D3).** Se ajusta con `retention.path_detail_days` (30 por defecto).
- `file_change_daily(root_id, day, actor, change, ext, files_touched, events)`, con **PRIMARY KEY `(root_id, day, actor, change, ext)`** (v1.2.2). **Agregado indefinido (D3), sin rutas individuales.** `ext` es la extensión (`.ts`, `.rs`…, `''` si no tiene), un dato que no es una ruta. *v1.2.1:* se quita `top_dir`, que era un fragmento de ruta.
- `tool_usage_daily(root_id, day, tool, calls, errors, interrupted, p50_ms, p95_ms, total_ms)`, con **PRIMARY KEY `(root_id, day, tool)`** (v1.2.2). **Agregado indefinido** de `tool_calls`, sin rutas (*nuevo en v1.2.1*). `root_id` usa `0` para «sin proyecto resuelto», nunca `NULL`, para que la clave sea única.
- `aggregation_ledger(target, root_id, day, state, source_rows, aggregated_at, purged_at)`, con **PRIMARY KEY `(target, root_id, day)`** (v1.2.2). `target` vale `file_change_daily` o `tool_usage_daily`. `state` vale `done` o `dirty`. Es el registro de qué días están agregados (§6.4.2).
- `project_components(root_id, kind, name, rel_dir, manifest_rel_path, version?)` y `project_deps(root_id, from_component, to_name, to_component?, scope)`: solo metadatos. Es una **instantánea actual** que se recalcula con los cambios. No es un histórico, y al dejar de estar activa la raíz se conserva solo la última.

##### 6.4.1 Retención (D3, coherente para todas las rutas en v1.2.1)

| Dato | Detalle durante | Después de `retention.path_detail_days` (30) |
|---|---|---|
| `file_events` (filas completas) | 30 días | Se borran. Quedan los agregados de `file_change_daily`. |
| `tool_calls.rel_path`, `tool_calls.host` (columnas definidas en §6.4) | 30 días | `UPDATE tool_calls SET rel_path = NULL, host = NULL WHERE day < corte AND (COALESCE(root_id,0), day)` tiene `done` en `aggregation_ledger` (SQL completo en §6.4.2; las filas `is_copy` se purgan igual). La fila sigue (herramienta, estado, duración), sin ruta. |
| `ingest_errors` (pueden contener rutas de transcripts) | 30 días | Se borran |
| `tool_calls` (filas sin ruta), `sessions`, `turns`, `api_messages`, `usage_snapshots`, `tasks` (títulos, D1) | Indefinido | Indefinido: son métricas y metadatos sin rutas de fichero |
| `file_change_daily`, `tool_usage_daily` | Indefinido | Indefinido (sin rutas) |
| `projects.root`, `project_components` | Mientras la raíz exista en la base | Raíz del proyecto: es la **dimensión** que permite agregar por proyecto, no un detalle de fichero. Se puede borrar a mano un proyecto entero desde Diagnóstico. |
| `file_offsets` (estado de la ingesta sobre `~/.claude`) | Mientras exista el JSONL | La fila se borra cuando el JSONL desaparece |

**Compactación diaria:** al arrancar y cada 24 h, en el hilo escritor único. El mecanismo transaccional e idempotente está en §6.4.2.

##### 6.4.2 Agregación transaccional e idempotente (v1.2.2)

**Qué días se agregan:** solo los días **cerrados**, `day < hoy_local`, y una vez que han pasado 2 h desde la medianoche local (margen para eventos tardíos y para la ventana de asentamiento de §7.3). **Normalización de `root_id` (R-2).** El valor centinela `0` significa «proyecto sin resolver». Se aplica de la misma forma en todos los sitios:
- `rk := COALESCE(root_id, 0)` en las agrupaciones, en la selección de candidatos, en la clave de `aggregation_ledger` (cuya columna `root_id` es `NOT NULL` y guarda siempre `rk`), en la purga, en la reconciliación y en los triggers.
- Las columnas `root_id` de `file_change_daily`, `tool_usage_daily` y `aggregation_ledger` son `NOT NULL` y guardan `rk`.
- En las tablas crudas, `tool_calls.root_id` admite `NULL` y `file_events.root_id` es `NOT NULL`. A esta última se le aplica `COALESCE` igualmente, para que el código y el SQL sean uniformes.
- **Coherencia target ↔ tabla cruda:** `file_change_daily` solo se calcula desde `file_events`, y `tool_usage_daily` solo desde `tool_calls` con `is_copy = 0`.

**Días candidatos** para el target `X` alimentado por la tabla cruda `R`:
```sql
SELECT COALESCE(root_id,0) AS rk, day FROM R
 WHERE day < :today_local /* y han pasado 2 h desde medianoche */ [AND is_copy = 0  -- solo para tool_calls]
 GROUP BY rk, day
EXCEPT
SELECT root_id, day FROM aggregation_ledger WHERE target = :X AND state = 'done'
UNION
SELECT root_id, day FROM aggregation_ledger WHERE target = :X AND state = 'dirty';
```
La rama `dirty` incluye targets que **ya no tienen filas crudas**, por ejemplo el proyecto anterior tras una reasignación. Su recálculo deja el agregado vacío y elimina el dato obsoleto.

**Unidad de trabajo:** una transacción `BEGIN IMMEDIATE … COMMIT` por cada `(target, rk, day)`:
1. `DELETE FROM <target> WHERE root_id = :rk AND day = :day`. Así el recálculo es completo, no incremental, y repetirlo no duplica nada.
2. Leer las filas crudas del día. Agrupar en Rust, porque SQLite no tiene percentiles; p50 y p95 se calculan sobre las duraciones del día.
   ```sql
   -- tool_usage_daily
   SELECT tool, status, duration_ms FROM tool_calls
    WHERE COALESCE(root_id,0) = :rk AND day = :day AND is_copy = 0;
   -- file_change_daily
   SELECT actor, change, rel_path FROM file_events
    WHERE COALESCE(root_id,0) = :rk AND day = :day;
   -- rel_path solo se usa en memoria: para derivar ext y contar ficheros distintos (files_touched); no se guarda
   ```
3. `INSERT` de las filas agregadas con `root_id = :rk`. La clave primaria impide duplicados aunque haya un error de lógica.
4. Registro:
   ```sql
   INSERT INTO aggregation_ledger(target, root_id, day, state, source_rows, aggregated_at)
   VALUES (:X, :rk, :day, 'done', :n, :now)
   ON CONFLICT(target, root_id, day) DO UPDATE
     SET state='done', source_rows=excluded.source_rows, aggregated_at=excluded.aggregated_at;
   ```
5. `COMMIT`.

Si el proceso se interrumpe (cierre, apagado, pánico), SQLite en WAL deshace la transacción entera: no quedan agregados parciales ni el registro marcado como `done`. En la siguiente ejecución, ese día vuelve a ser candidato. **No se duplica ni se pierde nada.**

**Purga (otra transacción, posterior y condicionada):** solo afecta a filas cuyo `(target, COALESCE(root_id,0), day)` tenga en el registro `state='done'` **y** `day < hoy − path_detail_days`. Para `tool_calls` se anulan `rel_path` y `host`; para `file_events` se borra la fila. Después se escribe `purged_at`.
```sql
UPDATE tool_calls SET rel_path = NULL, host = NULL
 WHERE day < :cutoff AND (rel_path IS NOT NULL OR host IS NOT NULL)
   AND EXISTS (SELECT 1 FROM aggregation_ledger l
                WHERE l.target='tool_usage_daily' AND l.state='done'
                  AND l.root_id = COALESCE(tool_calls.root_id,0) AND l.day = tool_calls.day);
-- también purga rutas de filas is_copy=1: su target es el de la fila canónica, ya agregado
UPDATE tool_calls SET rel_path = NULL, host = NULL
 WHERE day < :cutoff AND is_copy = 1 AND (rel_path IS NOT NULL OR host IS NOT NULL);
DELETE FROM file_events
 WHERE day < :cutoff
   AND EXISTS (SELECT 1 FROM aggregation_ledger l
                WHERE l.target='file_change_daily' AND l.state='done'
                  AND l.root_id = COALESCE(file_events.root_id,0) AND l.day = file_events.day);
UPDATE aggregation_ledger SET purged_at = :now
 WHERE day < :cutoff AND state='done' AND purged_at IS NULL;
```
La purga **no dispara triggers de suciedad**:
- El `UPDATE` de `tool_calls` solo cambia `rel_path` y `host`, que no están en la lista `UPDATE OF` del trigger.
- `file_events` no tiene trigger `AFTER DELETE`, porque el borrado solo ocurre en la purga, sobre días ya agregados.
- Un día no agregado **nunca se purga**: si la agregación falla de forma repetida, el detalle se conserva más de 30 días y Diagnóstico lo avisa.
- La purga solo borra o anula datos que ya están representados en los agregados, así que repetirla no tiene ningún efecto adicional.
- La purga de `ingest_errors` (que no tiene agregado) es un `DELETE … WHERE ts < corte` independiente.

**Datos tardíos y reasignaciones (R-2).** Pueden llegar por un backfill de días antiguos, por un upsert de atribución de §7.3, por un cambio de `is_copy` o porque `root_id` pase de `NULL` a un proyecto resuelto. Los triggers marcan como `dirty` el target afectado. En un `UPDATE` que cambia una dimensión, marcan **el anterior (`OLD`) y el nuevo (`NEW`)**, ambos normalizados con el centinela `0`:
```sql
-- tool_calls → tool_usage_daily
CREATE TRIGGER tc_ai AFTER INSERT ON tool_calls BEGIN
  UPDATE aggregation_ledger SET state='dirty'
   WHERE target='tool_usage_daily' AND root_id=COALESCE(NEW.root_id,0) AND day=NEW.day;
END;
CREATE TRIGGER tc_au AFTER UPDATE OF root_id, day, tool, status, duration_ms, is_copy ON tool_calls BEGIN
  UPDATE aggregation_ledger SET state='dirty'
   WHERE target='tool_usage_daily'
     AND ((root_id=COALESCE(OLD.root_id,0) AND day=OLD.day)
       OR (root_id=COALESCE(NEW.root_id,0) AND day=NEW.day));
END;
CREATE TRIGGER tc_ad AFTER DELETE ON tool_calls BEGIN   -- solo «borrar proyecto» (Diagnóstico)
  UPDATE aggregation_ledger SET state='dirty'
   WHERE target='tool_usage_daily' AND root_id=COALESCE(OLD.root_id,0) AND day=OLD.day;
END;
-- file_events → file_change_daily
CREATE TRIGGER fe_ai AFTER INSERT ON file_events BEGIN
  UPDATE aggregation_ledger SET state='dirty'
   WHERE target='file_change_daily' AND root_id=COALESCE(NEW.root_id,0) AND day=NEW.day;
END;
CREATE TRIGGER fe_au AFTER UPDATE OF root_id, day, actor, change, rel_path ON file_events BEGIN
  UPDATE aggregation_ledger SET state='dirty'
   WHERE target='file_change_daily'
     AND ((root_id=COALESCE(OLD.root_id,0) AND day=OLD.day)
       OR (root_id=COALESCE(NEW.root_id,0) AND day=NEW.day));
END;
-- sin trigger AFTER DELETE en file_events (ver Purga)
```
- Si todavía no existe fila en el registro, el `UPDATE` no afecta a nada y el día ya es candidato por no estar `done`.
- **Invariante de los días purgados de `file_events`:** a partir de la purga, sus filas ya no existen y el agregado es definitivo. Recalcularlo desde la tabla cruda los vaciaría, así que **el escritor rechaza** cualquier inserción o actualización de `file_events` con `day < cutoff`.
  - Esto no limita nada en la práctica: `file_events` solo procede del Watcher en vivo (no hay backfill), y las actualizaciones de atribución ocurren en minutos u horas (§7.3), nunca a más de 30 días.
- `tool_calls` no tiene esa restricción, porque la purga conserva las filas (solo anula las rutas). Recalcular un día purgado desde la tabla cruda da el mismo resultado. Un dato tardío de un día antiguo, por ejemplo de un backfill, se agrega y se purga en el mismo ciclo.

**Reconciliación (Diagnóstico).** Compara cada agregado con su tabla cruda **con la misma normalización** y solo en días no purgados:
```sql
SELECT COALESCE(root_id,0) AS rk, day, COUNT(*) FROM tool_calls WHERE is_copy=0 AND day >= :cutoff GROUP BY rk, day
-- frente a
SELECT root_id, day, SUM(calls) FROM tool_usage_daily WHERE day >= :cutoff GROUP BY root_id, day;
```
Si hay diferencias en días `done`, se marcan `dirty` y aparece un aviso.

**Backfill de días antiguos** (de más de 30 días): se inserta con rutas, se agrega en el mismo ciclo de compactación y se purga a continuación. Los agregados quedan completos y no se conserva ninguna ruta fuera del plazo.

Al terminar el ciclo: `PRAGMA wal_checkpoint(TRUNCATE)`.

**Tests obligatorios (F4a):**
- Ejecutar la compactación dos veces seguidas produce un resultado idéntico.
- Un fallo inyectado entre los pasos 3 y 5 hace que el día se vuelva a agregar en la siguiente ejecución, con un resultado idéntico al de una ejecución limpia.
- Una inserción tardía marca el día como sucio y se recalcula.
- Ningún día se purga sin estar en `done`.
- *(R-2)* Al reasignar `root_id` de `NULL` a `7`, se marcan como sucios `(0, día)` y `(7, día)`. Tras recalcular, el agregado de `0` ya no contiene esas filas y el de `7` sí.
- *(R-2)* Al cambiar `day` (por una corrección de zona horaria), se marcan los dos días.
- *(R-2)* La purga no deja ningún día como `dirty`.
- *(R-4)* Insertar la misma herramienta en dos sesiones (canónica y copia) cuenta 1 en `tool_usage_daily`, inserte primero la que inserte.
- *(R-4)* Un mismo `message_id` en dos sesiones se cuenta una vez, en la sesión más antigua.

Con `privacy.record_paths = false`, no se escribe ninguna ruta nueva. Las existentes siguen la retención normal, salvo que se pulse «borrar rutas ahora» en Diagnóstico, que adelanta el paso 2.

Tamaño estimado: menos del 3 % de los transcripts, más **≤ 5 MB acotados** de detalle de rutas (ventana de 30 días) y menos de 1 MB al año entre los dos agregados.

### 6.5 Publicación a la UI
Se mantiene v1:
- un único `Channel<LiveDelta>`, con orden garantizado;
- deltas coalescidos cada 100 ms;
- no se publica nada con la ventana oculta y, al volver a mostrarla, se envía un `Snapshot`.

Consultas añadidas: `query_tasks`, `query_file_events`, `git_status(root)`, `git_diff(root, rel_path, base?)`, `project_map(root)` y `read_file_preview(root, rel_path)`. Las vistas previas y los diffs viajan **solo en la respuesta**: no se guardan en disco ni en caché.

### 6.6 Proceso y ventana
Sin cambios desde v1: single-instance, cerrar la ventana la oculta en la bandeja, `window-state` y notificaciones nativas opcionales (solo avisos).

---

## 7. Componente D (nuevo): Workspace Watcher

### 7.1 Qué se vigila
- **Raíces activas:** el `git rev-parse --show-toplevel` del `cwd` de cada sesión viva (que llega por el mod, por `sessions/*.json` o por el transcript). Cada worktree cuenta como raíz propia. Si no hay Git, se usa el `cwd` de inicio de sesión.
- Una **raíz se desactiva** cuando pasan 10 minutos sin sesiones vivas en ella. Ese margen es configurable y evita reabrir el watcher en cada sesión corta.
- **Un watcher `notify` recursivo por raíz** [DOC]. `ReadDirectoryChangesW` cubre el subárbol con un único handle (buffer de 64 KiB). El filtrado ocurre en el espacio de usuario:
  1. Siempre se excluyen `.git/` (salvo `HEAD`, `index` y `refs/` para detectar commits y checkouts), `node_modules`, `target`, `dist`, `build`, `.venv`, `Library/` (Unity) y `.claude/worktrees`, que se vigilan como raíces aparte.
  2. Después se aplican las reglas de `.gitignore`, `.git/info/exclude` y la configuración global (crate `ignore`).
- **Cambios detectados:** `notify-debouncer-full` 0.7 agrupa los eventos en ventanas de 200 ms, une las parejas de renombrado (on Windows usa file-IDs) y suprime los duplicados de crear seguido de modificar. Así salen eventos `Created | Modified | Deleted | Renamed{from}`.
- **Tormentas de eventos** (npm install, builds): si una raíz supera los 500 eventos/s, el watcher pasa a modo «cambio masivo en <dir>», pausa el procesamiento fichero a fichero durante 2 s y luego resincroniza con una sola llamada a `git status`.
- **Overflow (`Rescan`)** [DOC]: se resincroniza con `git status` (unos 100 ms medidos) en lugar de recorrer el árbol.
- **Unidades de red o UNC:** el watcher se desactiva y aparece un aviso en Diagnóstico. Microsoft documenta que el buffer de 64 KB falla en red y que `PollWatcher` sería costoso.

### 7.2 Git exclusivamente de solo lectura [DOC]
- Se usa el **binario `git`** del sistema, no `git2` ni `gix`. Motivo: paridad exacta con la configuración del usuario (autocrlf, filtros, LFS, safe.directory, worktrees). Los procesos se lanzan desde Rust (`std::process`/`tokio::process`), no con `plugin-shell`.
- **Endurecimiento obligatorio de cada invocación:**
  ```
  git --no-optional-locks -c core.fsmonitor=false -c gc.auto=0 -c maintenance.auto=false -C <root> <cmd>
  ```
  - `--no-optional-locks` [DOC git ≥ 2.15] evita que `status` reescriba el índice. **Medido:** el mtime de `.git/index` no cambia en 4 repos reales (§11).
  - `env GIT_TERMINAL_PROMPT=0`, `GIT_OPTIONAL_LOCKS=0`.
  - Nunca se ejecuta `fetch`, `pull`, `checkout`, `add`, `stash`, `gc` ni ningún comando de escritura. La lista blanca de subcomandos se aplica en el código: `status`, `diff`, `rev-parse`, `ls-files`, `log -n`, `show --stat`, `worktree list`, `cat-file -s`.
- **Consultas:**
  - Estado: `status --porcelain=v2 -z --branch --untracked-files=all` → rama, ahead/behind, ficheros modificados o sin seguimiento.
  - Resumen de cambios: `diff --no-ext-diff --no-textconv --numstat -z HEAD` → +/− por fichero.
  - Diff de un fichero, bajo demanda: `diff --no-ext-diff --no-textconv --no-color HEAD -- <rel_path>`. Se limita a 2 MB y, si el fichero es binario, se marca como tal.
  - Ficheros sin seguimiento: vista previa con `read_file_preview` (primeros 256 KB, en memoria, nunca persistida).
- **Planificación:** una consulta en vuelo por repositorio y coalescencia de 500 ms tras el último evento de fichero. No se sondea nunca. Si el repo tiene más de 20.000 ficheros versionados, `--untracked-files=normal`.
- **Lo que no se guarda:** ni contenido ni diffs en SQLite. Solo `file_events`, el resumen `numstat` (contadores) y `GitStateChanged`.

### 7.3 Atribución del cambio por intervalo completo (v1.2.1)

**Intervalo de cada herramienta** `I(t) = [started_at − ε, finished_at + δ]`. Si la herramienta sigue en curso, `I(t) = [started_at − ε, ahora]`.
- `ε = 250 ms`: tolerancia de reloj entre `$.clock.now()` del mod y el reloj de Rust, que están en la misma máquina. El valor se calibra en G5.
- `δ = 1.500 ms`: cubre el retardo de escritura a disco, el debounce de 200 ms y la latencia del Watcher.
- Hora del evento de fichero: la marca del **primer** evento crudo que agrupa el debouncer (`notify-debouncer-full` la conserva), convertida a reloj de pared cuando se recibe. No se usa la hora de emisión del evento ya agrupado.

| Regla, por orden de prioridad | `actor` | `confidence` |
|---|---|---|
| **R1.** `t ∈ I` de una herramienta `Edit`/`Write`/`MultiEdit`/`NotebookEdit` **y** `rel_path` del evento = `rel_path` de la herramienta (`Renamed`: coincide `from` o `to`) | `claude` + `session` + `tool_use_id` (la referencia es el par `(actor_session, actor_tool_use_id)`, R-4; las filas `is_copy` no son candidatas) | `high` |
| **R2.** `t ∈ I` de una herramienta de edición, pero la ruta no coincide **o** la herramienta no lleva ruta (política `PRE_HANDSHAKE` o `record_paths=false`) | `claude` + `session` + `tool_use_id` | `medium` |
| **R3.** `t ∈ I` de un `Bash`/`PowerShell` (o MCP) de una sesión **cuya raíz es esta** | `claude-shell` + `session` + `tool_use_id` | `medium` |
| **R4.** Hay una sesión activa en la raíz, pero `t` no cae en ningún intervalo | `unknown` | `low` |
| **R5.** No hay ninguna sesión activa en la raíz | `external` | `high` |

**Desempates:**
- Si varias herramientas cumplen la misma regla (llamadas en paralelo, varias sesiones en la misma raíz), se elige la que tiene coincidencia de ruta.
- Si sigue habiendo empate, la de inicio más reciente, y la confianza **baja un nivel**.
- Si las candidatas son de sesiones distintas sin coincidencia de ruta, el actor es `unknown` con confianza `low`.

**Ventana de asentamiento.** Un evento de fichero puede llegar **antes** que su `ToolStarted`, porque el mod tarda unos 50 ms o más en enviarlo (§5.3). Por eso:
- la atribución es **provisional** durante 2 s y se vuelve a evaluar con cada `ToolStarted` o `ToolFinished` de esa raíz;
- después pasa a **definitiva**, con un upsert de `actor`, `actor_tool_use_id` y `confidence`;
- un `ToolFinished` que llega más tarde de la ventana (por una reconexión, §5.4) puede **subir** una atribución `unknown` a R1 o R2, pero nunca rebaja una que ya era `high`.

Respaldo: [DOC] para los eventos y el intervalo. La heurística es propia, así que la precisión se valida experimentalmente en **G5** (§13.2), con objetivos numéricos.

### 7.4 Vista «Cambios en directo»
- Lista virtualizada por raíz: fichero, tipo de cambio, actor, +/− y hora.
- Agrupada por turno y herramienta cuando el actor es `claude`.
- Al hacer clic se abre el diff (bajo demanda) en un visor de solo lectura con resaltado.
- El histórico de más de 30 días solo se muestra agregado (`file_change_daily`: por día, actor, tipo de cambio y extensión, **sin rutas**). El detalle por fichero se ve en los últimos 30 días (D3, §6.4.1).

---

## 8. Componente E (nuevo): Task Monitor

**Principio:** el progreso de una tarea es **lo que el agente declara** sobre su lista de tareas (completadas / total, `in_progress` actual, bloqueos) y el estado de sus subagentes. **El número de llamadas a herramientas nunca se muestra como progreso**: solo aparece como «actividad» (ritmo de herramientas por minuto, en una sparkline aparte).

| Adaptador | Origen real | Campos usados | Respaldo |
|---|---|---|---|
| **A1 Task tools** | `tool.call` de `TaskCreate` (resultado `task.id`), `TaskUpdate {taskId, status: pending\|in_progress\|completed\|deleted, addBlockedBy, owner}`, `classic.TaskCreated/TaskCompleted {task_id, task_subject}`. En el transcript, los `tool_use` equivalentes. | `subject` (§10, D1), `status`, `blockedBy`, `owner` | Herramientas: [DOC], tipos del motor. Hooks: [DOC]. Transcript: [INT]. |
| **A2 TodoWrite** | `tool.call TodoWrite {todos[{content, status, activeForm}]}`. Cada llamada es la lista **completa**, así que se reemplaza el snapshot entero. | `content` → `subject` (§10), `status` | [DOC] (tipos del motor). Es la fuente activa en tu historial (35 usos). |
| **A3 Desktop plan** | MCP `mcp__ccd_turn__update_plan {items[{content, status}] \| advance \| done/start}`. Fuente dominante aquí (426 usos). | `content`, `status` | **[INT] + [EXP] G6**: es una herramienta MCP propia del Desktop con un esquema no publicado. El adaptador es aislado y desactivable, y si falla no rompe el resto. |
| **A4 Subagentes** | `$.agent.list()` → `AgentInfo{id, type, status: pending\|running\|waiting\|idle\|completed\|failed\|killed, parentId}`, consultado en `turn.start`, `turn.complete`, `SubagentStart` y `SubagentStop` (por evento). En el transcript: `subagents/agent-*.jsonl` y `.meta.json`. | `id`, `type`, `status`, `parentId`, `durationMs` | Mod: [DOC]. Ficheros: [INT]. |

**Vista Task Monitor:**
- Por sesión: lista de tareas con su estado y bloqueos, barra «completadas / total» con la fuente indicada, tarea `in_progress` resaltada y árbol de subagentes con su estado.
- Por separado: actividad (ritmo de herramientas, herramienta en curso, último evento).
- Si una sesión **no declara tareas**, se muestra «sin lista de tareas declarada», **nunca** un progreso inventado.
- Con `privacy.capture_task_titles = false` (D1), las tareas aparecen como «Tarea n.º k», con su estado y su progreso intactos.

---

## 9. Componente F (nuevo): Project Map v1 (ligero)

- **Estructura:** si hay Git, el árbol sale de `git ls-files -z` (medido: entre 118 y 565 ficheros en tus repos). Si no hay Git, se recorre con `ignore`, con un tope de 50.000 entradas. Por directorio se muestran el número de ficheros, los bytes (con `stat`, **sin leer contenido**) y el lenguaje según la extensión.
- **Componentes** (detectores de manifiestos [DOC], que solo leen los ficheros de manifiesto y nunca el código):

| Ecosistema | Manifiestos | Componentes y relaciones internas |
|---|---|---|
| Node | `package.json` (+ `workspaces`), `pnpm-workspace.yaml` | Paquetes y dependencias `workspace:`/`file:` |
| Rust | `Cargo.toml` (`[workspace] members`) | Crates y dependencias `path =` |
| Java | `pom.xml` (`<modules>`), `settings.gradle(.kts)` | Módulos |
| Python | `pyproject.toml`, `requirements*.txt` | Paquete |
| .NET | `*.sln`, `*.csproj` (`ProjectReference`) | Proyectos y referencias |
| Unity | `Packages/manifest.json`, `ProjectSettings/ProjectVersion.txt` | Proyecto y paquetes UPM |
| Roblox / Rojo | `default.project.json` | Árbol de servicios |

- **Dependencias:** las externas declaradas (nombre, versión y ámbito dev/prod) y las aristas internas entre componentes. **No hay análisis de imports** en v1.
- **Capas superpuestas:** mapa de calor de los `file_events` de la sesión o del día y actividad de herramientas por directorio.
- **Render:** árbol virtualizado y grafo de componentes con **React Flow (`@xyflow/react`) + dagre** para el layout. El tamaño del bundle y el rendimiento con 200 nodos o más se validan en F4 (**[EXP] G8**). Alternativa si no rinde: SVG propio con dagre.
- **Caché:** se calcula en Rust y se guarda en `project_components` y `project_deps`. Se invalida solo cuando el Watcher ve cambiar un manifiesto.
- **v2 (fuera de alcance):** grafo de imports con tree-sitter, métricas de acoplamiento y detección de capas o arquitectura.

---

## 10. Privacidad: lista blanca de metadatos (nueva sección normativa)

**Regla:** los argumentos (`e.input`) y resultados de las herramientas **nunca** se guardan ni se transmiten completos. Solo salen del mod, y solo entran en SQLite, los campos de esta tabla, ya derivados. Hay **tres puntos de aplicación independientes**:
1. `privacy.ts` en el mod;
2. el esquema `deny_unknown_fields` de `HttpIngest`;
3. el `PrivacyFilter` del parser de transcripts.

| Herramienta | Metadatos permitidos | Nunca |
|---|---|---|
| `Read`, `Edit`, `MultiEdit`, `Write`, `NotebookEdit` | `rel_path` relativa a la raíz. Si queda fuera de la raíz: `"<externo>"` y la extensión. También `replace_all` (bool). | `old_string`, `new_string`, `content`, celdas |
| `Bash`, `PowerShell` | `run_in_background` (bool). El nombre del ejecutable (primer token) **solo** si `privacy.capture_shell_program = true`, que está **desactivado por defecto (D2)**. | `command`, `description`, argumentos, salida |
| `Grep`, `Glob` | Ninguno | `pattern`, `path`, `glob` |
| `WebFetch` | `host` | URL completa, `prompt` |
| `WebSearch` | Ninguno | `query` |
| `Agent` | `subagent_type`, `run_in_background` | `prompt`, `description` |
| `TaskCreate`, `TaskUpdate`, `TodoWrite`, `update_plan` | `task_id`, `status`, `blockedBy`, `owner`; `subject`/`content` **truncado a 120 caracteres**, solo si `privacy.capture_task_titles = true` (activado por defecto, **D1**) | `description`, `metadata`, `activeForm`, prompts |
| MCP `mcp__<srv>__<tool>` | `server`, `tool` | Argumentos y resultado |
| Cualquier herramienta desconocida | Solo `tool` | Todo lo demás |
| **Resultados** | `isError`, `status`, `durationMs` | Texto del resultado, mensajes de error |
| **Errores de API** | Estado HTTP, tipo de error | Cuerpo del mensaje |

**`rel_path` deja de ser opcional** (cambio respecto a v1). Es imprescindible para atribuir los cambios del Workspace Watcher (§7.3). Se guarda relativa, nunca absoluta, y existe un ajuste «no registrar rutas» que desactiva a la vez la atribución.

**Ajustes de privacidad y retención:**
- Los lee el `PrivacyFilter` de Rust.
- El mod recibe los que le afectan en la respuesta de `/v1/hello` (§6.2.1), de modo que el filtrado ocurre **en la captura y antes de transmitir**.
- Hasta esa respuesta, el mod aplica la política `PRE_HANDSHAKE` (§5.4): sin títulos y sin rutas de ningún tipo. Es más restrictiva que cualquier combinación de ajustes.

| Ajuste | Por defecto (app) | `PRE_HANDSHAKE` (mod) | Efecto |
|---|---|---|---|
| `privacy.capture_task_titles` | `true` (D1) | `false` | Títulos de tarea truncados a 120 caracteres. Con `false` se usa «Tarea n.º k». |
| `privacy.capture_shell_program` | `false` (D2) | `false` | Nombre del ejecutable de Bash/PowerShell |
| `privacy.record_paths` | `true` | `false` | `rel_path`, `cwd` y `host`. Con `false` en la app, también se desactiva la atribución por ruta (R1 → R2, §7.3). |
| `retention.path_detail_days` (antes `file_events_days`) | `30` (D3) | — | Retención de **todo** el detalle de rutas (§6.4.1). Los agregados sin rutas se conservan indefinidamente. |

Cambiar un ajuste **no reescribe hacia atrás**. Pasar `capture_task_titles` a `false` sí borra los títulos ya guardados (`UPDATE tasks SET subject = NULL`), porque es la opción más protectora.

Se mantienen las medidas de v1:
- el visor de transcript y de diffs lee bajo demanda y no persiste nada;
- no hay ningún tráfico saliente a Internet;
- el endpoint de loopback tiene las mitigaciones de §6.2.

---

## 11. Rendimiento

### 11.1 Resultados medidos (2026-11-10, este equipo, solo lectura, antes de implementar)
| ID | Medición | Resultado | Lectura |
|---|---|---|---|
| M1 | Lectura y `JSON.parse` **completos** de 471,7 MB (144 JSONL, 82.146 líneas) con Node 22, un hilo, caché del SO caliente | **3,46 s → 136 MB/s** | Es una referencia pesimista: Rust con parseo selectivo debería superarla. El objetivo de backfill < 60 s tiene margen de unas 17 veces. |
| M2 | Pico de llamadas a herramientas | **107/min** en una sesión (≈ 1,8/s); p99 = 39/min | Dimensiona el pipeline: unos pocos eventos por segundo. Lotes de 50 ms son suficientes. |
| M3 | `git --no-optional-locks status --porcelain=v2 -z --untracked-files=all` en 4 repos | En frío: **103–117 ms**. En caliente: **72–98 ms**. | La coalescencia de 500 ms es apropiada. |
| M4 | `git diff --no-ext-diff --no-textconv --numstat HEAD` | **128–687 ms** | Una consulta en vuelo por repo; `numstat` va fuera del camino de la UI. |
| M5 | Escrituras de Git durante las lecturas | mtime de `.git/index` **sin cambios** en los 4 repos | Confirma que el modo es solo lectura. |
| M6 | Ficheros en disco frente a versionados | Hasta **70.502 / 139** | El filtrado por `.gitignore` es obligatorio. |
| M7 | Duplicación de `usage` | 894 filas para 361 ids (×2,48) | El test de deduplicación es bloqueante. |
| M8 | Unicidad entre sesiones (R-4), en los 144 JSONL | **399 de 13.769 `tool_use_id`** y **242 de 9.945 `message.id`** aparecen en **más de un fichero de sesión**. En los 399 casos, la fila tiene el **mismo `timestamp` y el mismo `uuid`** en todas las sesiones, y el campo `sessionId` está reescrito con el de cada fichero. | Son **copias de historial** (reanudación o bifurcación), no ejecuciones nuevas. `tool_use_id` **no es único como clave de fila entre sesiones** → PK `(session_id, tool_use_id)` + `is_copy`. |

### 11.2 Objetivos (se mantienen los de v1 y se añaden los nuevos; «Medido» se rellena en F5)
| Métrica | Objetivo | Medido | Benchmark |
|---|---|---|---|
| Latencia `ToolStarted` en el mod → UI (p95) | **< 300 ms** (v1) | pendiente | B3 |
| Latencia transcript → UI (p95) | < 2,5 s (v1) | pendiente | B4 |
| Latencia cambio de fichero → UI (p95, sin `numstat`) | < 700 ms (nuevo) | pendiente | B6 |
| Actualización `numstat` tras un cambio (p95) | < 1,5 s (nuevo) | pendiente (M4: 128–687 ms solo para git) | B7 |
| CPU en reposo, sin sesiones | < 0,5 % (v1) | pendiente | B8 |
| CPU con una sesión al pico de 107 herramientas/min | < 3 % (nuevo) | pendiente | B5 |
| RAM del proceso Rust | < 60 MB (v1) | pendiente | B8 |
| RAM total con WebView2 **de DEV COCKPIT únicamente** (Rust + su grupo de procesos WebView2, *private bytes*; C2) | < 250 MB (v1) | pendiente (referencia de terceros: unos 109 MB para una app mínima) | B8 |
| Reconexión del mod, S1, sin actividad en la sesión | ≤ 15 s (nuevo, C1) | pendiente | B11 |
| Reconexión del mod, S1/S2, con actividad | ≤ 1 s tras el siguiente evento (nuevo, C1) | pendiente | B11 |
| Backfill de 471,7 MB | < 60 s con UI fluida (v1) | Referencia M1: 3,46 s con Node, parseo completo | B1 |
| Throughput del parser Rust | ≥ 150 MB/s (nuevo) | pendiente (referencia M1: 136 MB/s) | B1 |
| Coste propio del mod por herramienta | < 1 ms (v1) | pendiente | B2 |
| Timers activos del mod con la sesión inactiva y **conectado** | **0** (nuevo) | pendiente | B2 |
| Timers activos del mod con la sesión inactiva y **desconectado** | **≤ 1** (`stat` cada 15 s; nuevo, C1) | pendiente | B2 |
| Instalador NSIS | < 15 MB (v1) | pendiente | B10 |
| Prueba de 24 h | Crecimiento de RAM < 10 %, 0 pánicos (nuevo) | pendiente | B9 |

### 11.3 Sondeos: eliminados y conservados (punto 7)
| Sondeo | v1 | v1.1 | Justificación |
|---|---|---|---|
| `$.session.usage()` cada 5 s | Sí | **Eliminado**, sustituido por `session.measure` (push) | [DOC]: se dispara tras cada turno principal y cuando un rate limit se mueve un punto. Si G1 falla en 2.1.289: `$.session.usage()` **en `turn.complete`** (por evento, sin timer). |
| Flush del mod cada 250 ms | Sí | **Eliminado**: flush bajo demanda | 0 timers con la sesión inactiva y conectada. |
| Vigilancia de `endpoint.json` | — | **Nuevo (C1)**: `$.fs.stat` cada 15 s, **solo en estado `disconnected`**. Se cancela al conectar. | Es la única forma de que un mod en una sesión inactiva sepa que la app se ha abierto: la app no puede hacer push al mod sin canales internos (§14). Cuesta un `stat` local cada 15 s. |
| `stat` de todos los JSONL cada 2 s | Sí | **Reducido** a los JSONL de sesiones activas | `ReadDirectoryChangesW` puede retrasar el aviso de un append hasta que el dato llega a disco [DOC MS]. Sin el respaldo, el riesgo es una latencia indefinida. |
| `sessions/*.json` cada 2 s | Sí | **Eliminado**, sustituido por `notify` sobre el directorio y comprobación de PID cada 30 s | Hace falta detectar procesos muertos que no borraron su fichero. |
| Reescaneo de transcripts | — | Cada 60 s y con `Rescan` | Red de seguridad frente a eventos perdidos [DOC notify]. |
| Git | — | **Nunca se sondea** | Solo se consulta tras eventos de fichero o de `.git/HEAD`. |
| Contexto a mitad de turno | — | Del `usage` del transcript (el último mensaje del asistente) | `session.measure` solo se dispara al final del turno. Hookear `turn.step` (un stream) añadiría un coste por chunk, así que se descarta salvo que G2 demuestre que es barato. |

### 11.4 Batería de benchmarks (se ejecuta en F1–F5; los resultados se publican en `docs/perf/<fecha>.md`)
| ID | Qué mide | Cómo |
|---|---|---|
| B1 | Throughput del parser y del backfill | `criterion` sobre fixtures anonimizados, más `cockpit-ingest --backfill --dry-run` sobre `~/.claude/projects` real (solo lectura) |
| B2 | Coste del mod por evento y número de timers | Marcas `$.clock.now()` dentro del hook, contador en `/cockpit`, sesión inactiva de 10 min **en los dos estados** (conectado y desconectado) |
| B3 | Latencia mod → UI | Marca de tiempo del mod frente a `performance.now()` al pintar (con reloj común) |
| B4 | Latencia transcript → UI | Generador que añade líneas reales a un JSONL de prueba a 2 líneas/s |
| B5 | Carga sostenida | Reproductor de un transcript real a ritmo de pico (107 herramientas/min) durante 30 min |
| B6/B7 | Watcher y Git | Guion que crea, modifica, renombra y borra 100 ficheros en un repo de prueba y mide eventos → UI y `numstat` |
| B8 | Recursos **aislados de DEV COCKPIT (C2)** | Ver «Aislamiento de procesos» más abajo. Muestreo cada 1 s de CPU y *private bytes*, solo de los PIDs del conjunto de DEV COCKPIT. |
| B9 | Prueba de 24 h | B8 durante 24 h con uso real. El conjunto de PIDs se recalcula en cada muestra, porque WebView2 puede recrear renderers. |
| B10 | Artefactos | Tamaño del instalador, del bundle JS y del binario |
| B11 | Reconexión del mod (C1) | Escenarios S1–S4 de §5.4, con 10 repeticiones cada uno. Se mide la diferencia entre la escritura de `endpoint.json` y la recepción del `hello`, y se comprueba que la sesión de Claude **nunca** se reinicia. |

**Aislamiento de procesos para B8/B9 (C2).** Otras aplicaciones también usan WebView2 (por ejemplo Outlook, Teams o widgets), así que un `Get-Process msedgewebview2` las contaría. El conjunto de DEV COCKPIT se define así, con contraste de dos métodos:
1. **Fuente autoritativa [DOC MS]:** `ICoreWebView2Environment8::GetProcessInfos()`, más el evento `ProcessInfosChanged` del entorno WebView2 de la propia app, a través de `webview2-com`. Devuelve los PIDs y el tipo (browser, renderer, GPU, utility…) de **nuestro** grupo. La app lo publica en Diagnóstico y en `GET /v1/health?debug=procs` (solo loopback y con token). La implementación en Tauri está pendiente de **G9 [EXP]**.
2. **Contraste independiente:** `Get-CimInstance Win32_Process`, filtrando los `msedgewebview2.exe` que **descienden** del PID de `dev-cockpit.exe` (por `ParentProcessId`) **y** cuyo `CommandLine` contiene el `--user-data-dir` propio de la app (`%LOCALAPPDATA%\<identifier>\EBWebView`).
3. **Criterio de validez:** los conjuntos 1 y 2 deben coincidir. Si no coinciden, la muestra se invalida y se registra.
4. **Métrica:** se suman los *private bytes* (`PrivatePageCount`), no el *working set*, porque las páginas compartidas se contarían dos veces. El *working set* se reporta aparte como referencia.
5. **Desglose del informe:** proceso Rust, browser, renderer(s), GPU, utility y total. Cada entrada lleva la versión del runtime de WebView2.

---

## 12. Compatibilidad del mod (punto 5)

| Motor | Ruta | Mods (≥ 2.1.287 [DOC]) | Estado |
|---|---|---|---|
| CLI 2.1.285 | `~/.local/share/claude/versions/2.1.285` | **No** | Sin soporte |
| **CLI 2.1.289** | `~/.local/bin/claude` | Sí | Objetivo. Tipos pendientes de generar (G1). |
| Desktop 2.1.317 | `%APPDATA%\Claude\claude-code\2.1.317\…\claude.exe` | Sí | Objetivo |
| **Desktop 2.1.321** | `%APPDATA%\Claude\claude-code\2.1.321\…\claude.exe` | Sí | Tipos ya disponibles (los usa esta especificación) |

**Procedimiento (G-pre con el mod sonda; F3 con el mod real):**
1. Para **cada** motor: abrir una sesión interactiva con `--plugin-dir <carpeta del mod>`, que **no instala nada**. Para los motores del Desktop se ejecuta directamente su binario, `%APPDATA%\Claude\claude-code\<versión>\…\claude.exe --plugin-dir …`. No se usa `CLAUDE_CODE_PLUGIN_DIRS` en `~/.claude/settings.json`, porque escribiría en `~/.claude`.
   - **Autorización obligatoria (R-1).** Como cualquier sesión, una de prueba **escribe en `~/.claude`**: su transcript en `projects/`, su entrada en `sessions/` y otros datos de ejecución como `session-env/`, `shell-snapshots/`, `file-history/` o `history.jsonl`.
     - Por eso, **antes de cada sesión de prueba** se te pide **autorización explícita**, indicando la prueba (G1, G10 o compatibilidad), el motor (CLI 2.1.289, Desktop 2.1.317 o Desktop 2.1.321), el comando exacto y lo que escribirá.
     - **Avisar no basta y la autorización no se presupone**, ni se extiende a otra prueba o a otro motor.
     - **Sin una autorización expresa, la prueba no se ejecuta.**
   - Lo mismo vale para cualquier comando `claude` (`plugin validate`, `plugin test`…) cuyo efecto en `~/.claude` no esté verificado: se trata como si escribiera, y necesita autorización.
   - La CLI 2.1.285 queda fuera de la matriz (es anterior al mínimo de mods) y no se prueba. El motor escribe `.claude-plugin/types/claude-code/index.d.ts`, cuya primera línea es `// Written by Claude Code <versión>`. Se copia a `compat/engines/<versión>/claude-code.d.ts`, se versiona en el repo y se regenera, nunca se edita.
2. `tsc --noEmit -p compat/tsconfig.<versión>.json` contra cada declaración.
3. `"<binario del motor>" plugin validate mods/dev-cockpit-bridge` y `plugin test` con el binario **de ese motor**. Criterio: 0 errores y `gatingHooks` vacío.
4. Prueba E2E manual por motor: 3 herramientas, 1 subagente y 1 `TodoWrite`. Se comprueba en la app.

**Versión mínima soportada:** `minEngine = máx(2.1.287, la versión más baja en la que pasan los pasos 1–4)`. Valor **provisional: 2.1.289**, pendiente de G1. Si 2.1.289 no tiene `session.measure` u otra API usada, hay dos opciones:
- (a) recurrir al plan alternativo por evento de §11.3, o
- (b) subir `minEngine` a 2.1.317.

Lo decides tú en función del resultado de G1.

**En tiempo de ejecución:**
- El mod envía `$.session.version()` en `SessionStarted`.
- La app avisa si la versión es menor que `minEngine` o mayor que la última probada («no verificado»).
- El mod no usa APIs fuera de la intersección de los tipos de todos los motores soportados.
- `autoUpdatesChannel: latest` hará que lleguen motores nuevos: con cada uno se repiten los pasos 1–4 (es una lista de verificación de release).

---

## 13. Riesgos y puertas de validación experimental

### 13.1 Riesgos
Se mantienen R1–R11 de v1 (resumidos) y se añaden R12–R17.

| # | Riesgo | Prob./Imp. | Mitigación |
|---|---|---|---|
| R1 | La API de mods cambia entre releases | Alta/Media | §12, adaptador único, la app funciona sin mod, plan B con hooks `http` |
| R2 | El formato JSONL cambia | Alta/Alta | Parser tolerante, fixtures por versión, detector de deriva, `engine_version` por fila |
| R3 | Doble conteo de tokens | Cierta/Alta | PK `message_id`, test bloqueante (M7) |
| R4 | Eventos de fichero perdidos o tardíos | Media/Media | §11.3, `Rescan` → resincronización |
| R5 | Rate limits solo con mod y en cuentas de suscripción | Cierta/Baja | Mostrar «n/d» |
| R6 | El coste es una estimación | Cierta/Media | Etiquetado explícito |
| R7 | WebView2: memoria y throttling | Media/Media | Ring buffers, sin publicar con la ventana oculta, B9 |
| R8 | `sessions/*.json` no documentado | Media/Baja | Señal opcional |
| R9 | SmartScreen con un binario sin firmar | Media/Baja | Construcción local |
| R10 | Toolchain ausente | Cierta/Baja | F0 |
| R11 | Crecimiento de SQLite y WAL | Baja/Baja | Checkpoint, VACUUM |
| R12 | Tormentas de eventos (npm install, builds) | Alta/Media | Exclusiones duras, `.gitignore`, modo masivo (§7.1) |
| R13 | Git lento en repos grandes o con LFS | Media/Media | Una consulta en vuelo por repo, coalescencia, `--untracked-files=normal` con más de 20.000 ficheros, `--no-textconv` |
| R14 | Escritura accidental en el repo | Baja/Alta | Lista blanca de subcomandos, `--no-optional-locks`, test M5 automatizado (mtime del índice) |
| R15 | Atribución de cambios incorrecta | Media/Baja | Columna `confidence`, G5 |
| R16 | El adaptador `update_plan` del Desktop se rompe | Alta/Media | Aislado, desactivable, G6 |
| R17 | Fuga de datos en metadatos | Baja/Alta | Tres puntos de aplicación (§10), tests de propiedades con entradas «trampa» |
| R18 | El mod no se reconecta y la sesión se queda en modo degradado | Media/Media | §5.4: máquina de estados, vigilancia de 15 s, relectura con `401`/`ECONNREFUSED`, B11 |
| R19 | Mediciones de memoria contaminadas por otras apps WebView2 | Alta/Media | C2: dos métodos de identificación con contraste obligatorio, *private bytes* |

### 13.2 Puertas [EXP]
| Puerta | Pregunta | Método | Si falla |
|---|---|---|---|
| **G1** | ¿Existen `session.measure`, `classic.Task*`, `$.agent.list`, `$.session.version`, `$.http.fetch`, `$.fs.stat/read` y `$.clock.after/every` en 2.1.289, 2.1.317 y 2.1.321? | **Antes de F0** (a petición tuya): tipos generados por cada motor con un mod sonda + `tsc` + `validate` | Plan alternativo §11.3 o subir `minEngine`. Se presenta para que decidas. |
| **G2** | ¿Cuánto cuesta hookear `turn.step` (passthrough)? | B2 con y sin el hook | No se usa (por defecto) |
| **G3** | ¿El tiempo en `next` de `tool.call` incluye el diálogo de permisos? | Herramienta con aprobación manual cronometrada | Estado `awaiting_permission` con `classic.PermissionRequest` |
| **G4** | ¿Ve el mod las herramientas de los subagentes con `agentId`? | E2E con 1 subagente | Completar desde `subagents/*.jsonl` |
| **G5** | Precisión de la atribución por intervalo (§7.3) | Guion con verdad conocida de ≥ 200 cambios: ediciones de Claude (Edit/Write), cambios hechos por `Bash` de Claude, cambios externos durante una sesión activa, herramientas en paralelo y dos sesiones en la misma raíz. Se publica la matriz de confusión por regla y se calibran ε y δ. **Objetivos:** precisión ≥ 0,95 en `high`, ≥ 0,80 en `medium`, 0 casos `external` mal atribuidos a `claude`. | Ajustar ε y δ. Si aun así no se llega, rebajar la regla afectada un nivel de confianza o mostrar solo `unknown`/`external`. |
| **G6** | Esquema estable de `update_plan` en el Desktop | Fixtures de 2.1.317 y 2.1.321 | Desactivar A3 |
| **G7** | ¿`notify-debouncer-full` une los renombrados en NTFS de forma fiable? | B6 con renombrados y movimientos entre carpetas | Eventos Delete + Create sin `from` |
| **G8** | React Flow + dagre con 200 nodos o más dentro del presupuesto | B8 en la vista Mapa | SVG propio + dagre |
| **G9** | ¿Se puede obtener `ICoreWebView2Environment8::GetProcessInfos` desde Tauri 2.12 / wry con `webview2-com`? | Prototipo en F2 | Solo el método 2 de C2 (árbol de procesos + `--user-data-dir`), documentado como menos autoritativo |
| **G10** | ¿`$.fs.stat` / `$.fs.read` de un mod pueden leer `%APPDATA%\dev-cockpit\endpoint.json`, fuera del proyecto? | Prueba **antes de F0** (a petición tuya) con los 3 motores, usando un mod sonda desechable que no sale del directorio de pruebas | **DETENERSE** y presentarte las alternativas de la lista de abajo. **No se elige ninguna automáticamente.** Escribir en `~/.claude` está prohibido sin tu autorización previa y expresa. |

**Alternativas si falla G10** (se presentan, no se aplican; ninguna está preseleccionada):

| Alt. | Mecanismo | ¿Escribe en `~/.claude`? | Coste o riesgo |
|---|---|---|---|
| A | Otra ruta legible por el mod fuera de `~/.claude`, por ejemplo `%LOCALAPPDATA%\dev-cockpit\`. Depende de qué rutas permita `$.fs`. | No | Necesita repetir G10 con esa ruta |
| B | Leer `endpoint.json` con `$.process.run(["cmd","/c","type",ruta])`, que se ejecuta con los permisos del usuario | No | Arranca un proceso por cada lectura. Con la vigilancia de 15 s en `disconnected`, unos 4 procesos por minuto. |
| C | Puerto fijo, más el token en una variable de entorno de usuario (`$.env.get`) | No (cambia el entorno de usuario de Windows) | Hay que reiniciar Claude la primera vez. Contradice C1 en la instalación inicial. |
| D | `userConfig` del mod (puerto y token) guardado en `pluginConfigs` de `~/.claude/settings.json` | **Sí**: necesita tu autorización | Lo modifica la app o lo modificas tú a mano. El token ya no se puede rotar en cada arranque. |
| E | Fichero dentro de `~/.claude/dev-cockpit/` | **Sí**: necesita tu autorización | Rompe la regla de «solo lectura en `~/.claude`» |

---

## 14. Alternativas descartadas

Se mantienen las de v1: Electron, sidecar de Node, `tauri-plugin-sql`, `emit` para streaming, OTel como fuente primaria, status line y el mod como interfaz principal. Se añaden:
- **`git2`/`gix` para Git:** se descartan en v1 por la paridad con la configuración real del usuario y por el riesgo de que alguna operación toque el índice. Se pueden reconsiderar si B7 muestra que lanzar procesos es un cuello de botella.
- **`PollWatcher` como modo general:** es caro con decenas de miles de ficheros. Solo se usaría en unidades de red, y quedan fuera de v1.
- **Hookear `turn.step` por defecto:** añade un coste por chunk en cada respuesta del modelo (G2).
- **Guardar diffs o contenido para histórico de código:** lo prohíbe la política de privacidad. Git ya es ese histórico.
- **Grafo de imports en v1:** requiere parsers por lenguaje (tree-sitter) y su coste es incierto. Pasa a v2.

---

## 15. Plan por fases

**Paso inmediato tras aprobar esta especificación (y lo único que se hará):**
0. Crear la rama `docs/spec-v1.2.2` sobre el `main` sin nacer, siguiendo el procedimiento W9, pasos 1–3 (§17). Sin commit.
1. Escribir en el repo `docs/SPEC.md`, con el contenido íntegro de la v1.2.2.
2. Escribir `docs/CHANGELOG-spec.md`, con v1 → v1.1 → v1.2 → v1.2.1 → v1.2.2 (§0, §0.1, §0.2 y §0.3).
3. Escribir `docs/DEVELOPMENT_WORKFLOW.md`, con las reglas W1–W11 y el procedimiento W9 de §17.
4. Escribir `docs/adr/0001` a `0012`, un ADR por decisión clave:
   1. Tauri frente a Electron
   2. Rust como única fuente de estado
   3. Channel frente a `emit`
   4. rusqlite frente a sqlx/plugin-sql
   5. Mod como puente
   6. Reconciliación de fuentes
   7. Git CLI de solo lectura
   8. Lista blanca de privacidad
   9. Push antes que polling
   10. Task Monitor por tareas declaradas
   11. Retención de rutas y agregados (D3, v1.2.1)
   12. Reconexión y handshake `/v1/hello` (C1, v1.2.1)
5. Comprobar las referencias cruzadas (§ y ADR) y la coherencia entre documentos.
6. **Detenerse y presentarte los archivos.** No se crea código, no se instalan herramientas, no se hace ningún commit y no se instala ningún plugin.

**Antes de F0, y tras tu aprobación de los documentos (fase previa «G-pre»):**
- Resolver **G1** y **G10** con un mod sonda desechable en una rama `spike/g1-g10-mod-probe`, sin código de aplicación.
- Si G10 falla: **detenerse** y presentar las alternativas A–E de §13.2.
- Toda escritura en `~/.claude` requiere tu autorización previa. Esto incluye instalar el mod sonda con `claude plugin install` o `marketplace add`, que escriben en `~/.claude`. Por eso la sonda se cargará solo con `--plugin-dir`, que **no instala nada**, y ejecutando directamente el binario de cada motor (§12, paso 1). No se usará `CLAUDE_CODE_PLUGIN_DIRS` desde `~/.claude/settings.json`.
- **Aunque no instale nada, cada sesión de prueba escribe en `~/.claude`** (transcript, registro de sesión, datos de ejecución). Por eso **cada prueba G1 y G10, en cada uno de los tres motores, necesita tu autorización explícita previa** (R-1, §12 paso 1, W11). Sin ella no se ejecuta. Avisar no basta.

| Fase | Entregable | Criterio de «hecho» |
|---|---|---|
| **F0** Toolchain y andamiaje | rustup con el target **GNU** (`stable-x86_64-pc-windows-gnu`) + MinGW-w64 portátil (WinLibs: `gcc`, `windres`), Rust ≥ 1.90, `create-tauri-app` (React-TS), workspace de 4 crates, clippy y eslint. *Desviación (2026-11-11):* el plan original era rustup MSVC + VS Build Tools, pero su instalador exige permisos de administrador y este equipo no los tiene. | `npm run tauri dev` abre una ventana vacía |
| **F1** Núcleo de ingesta sin UI | Parser, `PrivacyFilter`, store, tailer, CLI `cockpit-ingest` | Fixtures dorados en verde. Deduplicación igual a `cost-state`. **B1** registrado. |
| **F2** Shell Tauri y vista En vivo | Channel, Zustand, tarjetas, bandeja, aislamiento de procesos de C2 | Sesión real en vivo. **G9** resuelta. **B4** y una primera B8 aislada registradas. |
| **F3** Mod puente y compatibilidad | Mod + `HttpIngest` (`/v1/hello`, `/v1/events`, `/v1/health`) + Live Tool Monitor + reconexión §5.4 + política `PRE_HANDSHAKE` + §12 en los 3 motores | **G3, G4** resueltas (G1 y G10 ya resueltas en G-pre). `validate`/`test` en verde ×3. **B2, B3, B11** registrados. **C1 aprobada:** S1–S4 sin reiniciar Claude. Test de que no salen rutas ni títulos antes del primer `hello`. |
| **F4a** Workspace Watcher | §7 completo (atribución por intervalo §7.3), retención §6.4.1 y agregación y purga transaccionales §6.4.2 (con los tests de §6.4.2) | **G5** (con los objetivos de precisión) y **G7** resueltas. **B6, B7** y test M5 en verde. Test de purga: no queda ninguna ruta con más de 30 días. |
| **F4b** Task Monitor | Adaptadores A1–A4 | **G6** resuelta. E2E con `TodoWrite` y `update_plan`. |
| **F4c** Project Map, histórico, límites y diagnóstico | §9 + vistas de v1 | **G8** resuelta. Consultas < 100 ms. |
| **F5** Endurecimiento | NSIS, tabla «Medido» completa | **B5, B8, B9, B10** con memoria aislada (C2) y desviaciones documentadas |

### Decisiones cerradas (v1.2)
- **D1, aceptada:** títulos de tarea truncados a 120 caracteres. Nunca descripciones ni prompts. Se pueden desactivar con `privacy.capture_task_titles` (§10).
- **D2, aceptada:** el ejecutable de Bash no se registra por defecto (`privacy.capture_shell_program = false`).
- **D3, aceptada:** 30 días de detalle en `file_events`. Los agregados de `file_change_daily` y las demás métricas se conservan a largo plazo (§6.4).
- **C1, añadida:** reconexión automática del mod sin reiniciar Claude (§5.4, B11).
- **C2, añadida:** benchmarks de memoria aislados del grupo WebView2 de DEV COCKPIT (§11.4, G9).

### Revisión de coherencia (v1.2)
- Los timers del mod son 0 con la sesión conectada e inactiva, y ≤ 1 con la sesión desconectada. Coinciden en §5.3, §5.4, §11.2 y §11.3.
- El tamaño de `file_events` está acotado por la retención de 30 días. Coinciden §6.4, §7.4 y §10.
- Los ajustes de privacidad aparecen solo en §10 y se citan en §8 y §15. Los valores por defecto coinciden con D1–D3.
- «Solo lectura absoluta» (§4.3) **no tiene excepciones** (v1.2.1). Las alternativas de G10 que escribirían en `~/.claude` (D y E) están marcadas como «necesita tu autorización» y no se eligen nunca automáticamente.
- *(v1.2.1)* La retención de rutas es de 30 días **en todas las tablas**: `file_events`, `tool_calls.rel_path/host` e `ingest_errors` (§6.4.1). Los agregados `file_change_daily` y `tool_usage_daily` no contienen rutas (§6.4, §7.4). El ajuste se llama `retention.path_detail_days` en §6.4, §6.4.1 y §10.
- *(v1.2.1)* La política `PRE_HANDSHAKE` es igual en §5.4, §6.2.1 y §10, y la R2 de §7.3 cubre las herramientas sin ruta capturadas antes del handshake.
- *(v1.2.1)* Las rutas de `HttpIngest` son las mismas en §4 (diagrama), §6.2 y §6.2.1: `/v1/hello`, `/v1/events` y `/v1/health`.
- *(v1.2.1)* G1 y G10 se resuelven en G-pre (antes de F0), no en F3. Coinciden §13.2 y §15.
- *(v1.2.2)* `tool_calls.host` y `tool_calls.rel_path` están definidas en §6.4, se purgan según §6.4.1 y el mecanismo de §6.4.2. `meta_json` nunca contiene rutas (§6.4, §10).
- *(v1.2.2)* Las claves primarias de `file_change_daily` `(root_id, day, actor, change, ext)` y `tool_usage_daily` `(root_id, day, tool)` coinciden en §6.4 y §6.4.2. La purga depende siempre de `aggregation_ledger.state='done'`.
- *(v1.2.2)* La instalación permanente del plugin requiere autorización previa: §5.3, §12 (paso 1, `--plugin-dir`), §15 (G-pre) y §17 (W11).
- *(v1.2.2)* El procedimiento W9 se cita igual en §15 (paso 0) y §17. La rama de documentación se llama `docs/spec-v1.2.2`.
- *(rev. final, R-1)* La autorización explícita para escribir en `~/.claude` dice lo mismo en §4.3, §12 (paso 1), §15 (G-pre) y §17 (W11). Abarca la instalación, las sesiones de prueba en los tres motores y los comandos con un efecto no verificado.
- *(rev. final, R-2)* El centinela `0` y `COALESCE(root_id,0)` se usan igual en §6.4 (definición de los agregados), §6.4.2 (candidatos, unidad de trabajo, registro, purga, triggers y reconciliación) y §6.4.1 (purga de `tool_calls`).
- *(rev. final, R-3)* F4a cita §6.4.1 y §6.4.2.
- *(rev. final, R-4)* La clave `(session_id, tool_use_id)` es la misma en §3, §6.3, §6.4 (`tool_calls`, `file_events.actor_*`), §6.4.2 (`is_copy`) y §7.3. `running_tools` en `/v1/hello` y los eventos de `/v1/events` ya llevan `session_id` en el sobre (§6.2.1).
- La métrica de RAM de §11.2 es la misma que define C2 en §11.4: *private bytes* del grupo propio.
- El volumen de transcripts es de 471,7 MB en JSONL (521 MB en disco con el resto de ficheros). Se usa la misma cifra en §2, §11.1 y §11.2.

---

## 16. Verificación y fuentes

**Verificación de la especificación:**
- Las mediciones M1–M7 se pueden reproducir con scripts de solo lectura (irán en `scripts/measure/`).
- La sección de compatibilidad se verifica con §12, pasos 1–4.
- Cada puerta G# tiene su método en §13.2.
- La prueba E2E final se hace en este repo:
  1. Abrir una sesión de Claude Code.
  2. Ejecutar 3 herramientas, 1 subagente, 1 `TodoWrite` y 1 edición de fichero.
  3. Comprobar en la app: herramienta en curso visible antes de que termine, tareas con su progreso declarado, el fichero en «Cambios en directo» con actor `claude` y su diff, contexto % y coste (comparado con `/cost`).
  4. **Reconexión (C1):** cerrar DEV COCKPIT con la sesión abierta, seguir trabajando, volver a abrir la app y comprobar que la sesión vuelve a «mod conectado» en ≤ 15 s, con las herramientas en curso y el snapshot de uso, **sin reiniciar Claude**.
  5. **Memoria (C2):** ejecutar B8 con Outlook o Teams abiertos y comprobar que el conjunto de PIDs medido coincide con `GetProcessInfos` y excluye sus procesos WebView2.
  6. **Privacidad previa al handshake (v1.2.1):** con la app cerrada, hacer una edición y crear una tarea. Abrir la app y comprobar en un registro de depuración de `HttpIngest` que ni el primer `hello` ni los eventos encolados contienen `rel_path`, `cwd`, `host` ni títulos. Después, comprobar que la app los rellena desde el transcript.

---

## 17. Flujo de desarrollo (normativo; se replica en `docs/DEVELOPMENT_WORKFLOW.md`)

Estas reglas se aplican a **todas** las fases de DEV COCKPIT, y la documentación también está sujeta a ellas.

| # | Regla | Cómo se cumple |
|---|---|---|
| W1 | **Nunca se trabaja directamente sobre `main`.** | Antes de modificar cualquier fichero, se comprueba con `git branch --show-current` que la rama actual no es `main`. |
| W2 | **Una rama descriptiva por fase o funcionalidad**, creada desde el `main` más reciente | Prefijos: `docs/…`, `spike/…`, `feature/f0-scaffold`, `feature/f1-ingestion`, `feature/f2-dashboard`, `feature/f3-mod-bridge`, `feature/f4a-workspace-watcher`…, `fix/<tema>`. Se crean con `git switch -c <rama> main`. |
| W3 | **Commits progresivos y descriptivos, solo con tu autorización** | Se hace un commit cuando lo autorizas para esa fase o paso. Mensaje en imperativo con su porqué (`feat(ingest): …`, `docs(spec): …`). Nunca `--amend` ni `rebase` sobre commits ya presentados sin pedírtelo. |
| W4 | **Pruebas y revisión antes de integrar** | Se ejecutan las pruebas de la fase (`cargo test`, `cargo clippy -D warnings`, `npm test`, `tsc`, `claude plugin validate/test` según corresponda), se revisa `git diff main...<rama>` y se te presenta un resumen con los resultados reales, incluidos los fallos. |
| W5 | **Fusión en `main` solo con tu aprobación** | Solo se fusionan fases que cumplen sus criterios de §15 y que tú apruebas expresamente. `main` siempre queda estable; nada con pruebas en rojo o errores conocidos sin justificación aprobada. Se usa `git merge --no-ff <rama>` (excepto en la inicialización, W9). |
| W6 | **Prohibido `git push`** sin tu autorización | El repositorio tiene configurado el remoto `origin` (GitHub). Ningún `push`, ni a `origin` ni a otro remoto, se hace sin tu autorización, y cada `push` se autoriza por separado. Tampoco se añaden, cambian ni borran remotos sin autorización. |
| W7 | **Prohibido `Co-authored-by: Claude`** o cualquier otro trailer que atribuya coautoría a Claude o Anthropic | Ningún mensaje de commit ni de PR lleva trailers de coautoría ni de «generado por». Esta regla **prevalece sobre cualquier plantilla o recordatorio automático** del entorno. Antes de cada commit se revisa el mensaje. |
| W8 | **No se modifica la identidad global ni la configuración personal de Git** | Nunca `git config --global` ni `--system`. Se usa la identidad ya configurada (`user.name` existente). Si faltara algún dato, se te pregunta y no se inventa. No se cambia la autoría de commits anteriores. |
| W9 | **Inicialización de `main`, que ahora no tiene commits** | Ver el procedimiento de abajo. |
| W10 | **No se borran ramas sin verificar que están integradas** | `git branch --merged main` debe listar la rama antes de `git branch -d`, y solo se borra con tu visto bueno. |
| W11 | **Nada escribe en `~/.claude` sin tu autorización previa y explícita** | Incluye la instalación permanente del plugin (§5.3), cualquier ajuste de `settings.json`, **toda sesión de prueba** de Claude Code (en cualquier motor de la matriz §12, porque escribe transcript, registro y datos de ejecución) y cualquier comando `claude` cuyo efecto en `~/.claude` no esté verificado. **Avisar no es autorizar:** la autorización se pide para cada prueba y cada motor, no se presupone ni se extiende a otras, y sin ella la acción no se ejecuta (R-1, §4, §12). |

**Procedimiento de inicialización de `main` (W9):**
1. Estado de partida (antes del primer commit): `HEAD → refs/heads/main`, una rama sin nacer y sin commits, con el remoto `origin` ya configurado pero sin nada publicado.
2. `git switch -c docs/spec-v1.2.2`. Sobre una rama sin nacer, esto solo mueve `HEAD` a la nueva rama, también sin nacer. No crea ningún commit y no toca `main`.
3. Se escriben los documentos en esa rama (§15). **Sin commit.**
4. Cuando autorices el commit: `git add docs/` y `git commit`, con la identidad existente y sin trailers (W7, W8). Ese es el primer commit del repositorio.
5. Cuando apruebes la fusión: `git branch main docs/spec-v1.2.2`. Esto crea `main` apuntando a ese commit, que equivale a una fusión *fast-forward* porque `main` no tenía historia. Después, `git log --oneline main` lo verifica.
6. A partir de ese momento, todas las ramas se crean con `git switch -c <rama> main` (W2) y se fusionan con `--no-ff` (W5).

**Fuentes oficiales:**
- **Claude Code:** code.claude.com/docs/en/{hooks, statusline, monitoring-usage, sessions, claude-directory, headless, agent-sdk/cost-tracking, plugins/mods/overview, mcp, settings}, y los tipos generados `claude-code.d.ts` de la build 2.1.321 (`session.measure`, `SessionUsage`, `TurnUsage`, `AgentStatus`, `TaskCreate`/`TaskUpdate`/`TodoWrite`, `HookBudget`, `$.http.fetch`, `$.process`, `$.store` ≤ 4 MiB).
- **Tauri:** v2.tauri.app/{develop/calling-frontend, security/capabilities, security/csp, plugin/*, distribute/windows-installer, distribute/sign/windows, reference/config}.
- **Microsoft:** learn.microsoft.com/microsoft-edge/webview2/concepts/{distribution, process-model, performance} y ReadDirectoryChangesW.
- **Git:** git-scm.com/docs/git (`--no-optional-locks`), git-status (`--porcelain=v2`), git-diff (`--numstat`, `--no-ext-diff`, `--no-textconv`).
- **Crates:** docs.rs/{notify, notify-debouncer-full, ignore, rusqlite, tauri::ipc::Channel}, sqlite.org/wal.html.
- **Frontend:** react.dev (`useSyncExternalStore`), tanstack.com/virtual, github.com/leeoniya/uPlot, reactflow.dev.
