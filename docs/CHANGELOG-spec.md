# Historial de cambios de la especificación DEV COCKPIT

Documento de referencia: [`SPEC.md`](SPEC.md). Las tablas detalladas de cada versión están en SPEC §0 (v1.1), §0.1 (v1.2), §0.2 (v1.2.1), §0.3 (v1.2.2) y §0.4 (revisión final). Las decisiones de fondo se justifican en los ADR de [`adr/`](adr/).

---

## Precisiones de F4c: Project Map (2026-11-12)

No cambian la arquitectura ni el alcance. Resultados en [`perf/2026-11-12-f4c.md`](perf/2026-11-12-f4c.md).

- **G8: grafo de componentes con SVG propio y layout por capas propio, sin React Flow ni dagre (desviación de §9 y de la alternativa de G8, decidida por delegación del usuario: «Haz lo que creas mejor», 2026-11-12).** §9 pedía React Flow (`@xyflow/react`) + dagre, y G8 preveía «SVG propio + dagre» si no rendía. Se usa SVG con un Sugiyama simplificado escrito para el proyecto (`src/map/layout.ts`): capas por camino más largo, nodos virtuales para las aristas que saltan capas, baricentros y posición por regresión isotónica. Motivos:
  1. no añade dependencias, así que no hace falta instalar nada ni crece el bundle;
  2. el grafo es de solo lectura: no necesita el arrastre ni la edición de React Flow;
  3. rinde con margen: 200 nodos y 398 aristas, layout en 4,6 ms y pintado en 33 ms.

  La lista de componentes que hay junto al grafo es su alternativa en texto.

## Precisiones de F4a: Workspace Watcher (2026-11-12)

No cambian la arquitectura ni el alcance. Resultados en [`perf/2026-11-12-f4a.md`](perf/2026-11-12-f4a.md).

- **Calibración de la atribución tras G5 (desviación de §7.3, aceptada por el usuario el 2026-11-12).** Con las reglas de §7.3 al pie de la letra, G5 no llegaba a los objetivos (`medium` 0,73 y 8 cambios externos atribuidos a `claude`). Siguiendo la vía que prevé G5 para ese caso («rebajar la regla afectada o mostrar solo `unknown`»), se cambian tres cosas:
  1. **R2 solo se aplica a ediciones sin ruta comparable** (`PRE_HANDSHAKE`, `record_paths = false` o fuera del `cwd`). Un `Edit`/`Write` con ruta conocida que no coincide deja de ser candidato, porque esas herramientas solo escriben su propio fichero. Ese cambio queda en R3, R4 o R5.
  2. **Desempate por intervalo real:** si alguna candidata contiene el cambio en `[started_at, finished_at]` (sin ε ni δ), solo cuentan esas. La cola δ de una herramienta ya terminada deja de empatar con la que está en curso.
  3. **Subida tardía:** una atribución definitiva también puede pasar a R1 si la confianza sube (por ejemplo, un R2 o un R3 que resulta ser un `Edit` del mismo fichero cuyo aviso llegó tarde). Sigue sin rebajarse nunca una `high`.
  
  ε (250 ms) y δ (1.500 ms) no cambian. (SPEC §7.3, §13.2 G5)
- **Endurecimiento de Git ampliado con `-c diff.autoRefreshIndex=false`.** El test M5 automatizado demostró que `git diff … HEAD` (el `numstat` y el diff de un fichero) **reescribe `.git/index`** cuando hay ficheros con `stat` obsoleto, aunque lleve `--no-optional-locks` (Git 2.51.2.windows.1). `status`, `rev-parse` y `ls-files` no lo hacen. Con la opción desactivada, el índice no cambia y la salida es la misma. Es más estricto que §7.2, no una relajación. (SPEC §7.2, R14)
- **`notify` 8.2.2 en Windows** (lectura de su código): usa un buffer de 16 KiB (la SPEC decía 64 KiB), **ningún camino emite `Rescan`**, y un código de error desconocido de `ReadDirectoryChangesW` deja de vigilar la carpeta sin avisar. Una sonda de 20.000 ficheros a unos 5.000/s no llegó a desbordarlo (40.000 de 40.000 eventos), así que la pérdida no está reproducida. La red de seguridad es el modo «cambio masivo» y el `git status` posterior, que es la verdad del estado del árbol. Detectar un watcher caído queda para F5. (SPEC §7.1, R4)
- **Coalescencia de Git con tope:** 500 ms sin cambios, y como máximo 1 s **desde el primer cambio pendiente**. Sin tope, con actividad continua Git no se consultaría nunca; con un tope de 2 s contado desde el aviso, B7 por cambio daba un p95 de 2,4 s. (SPEC §7.2, B7)
- **Caché de file-IDs propia (`PairCache`)** en lugar de la `FileIdMap` recomendada, que recorre el árbol entero (siguiendo enlaces) al vigilar una raíz y con cada directorio nuevo, y abre cada fichero para leer su id. `PairCache` empareja cada `RENAMED_NEW_NAME` con el `RENAMED_OLD_NAME` inmediatamente anterior, que es como los entrega Windows. (SPEC §7.1)
- **G7:** dentro de un directorio, los renombrados se unen con su origen. Un movimiento **entre carpetas** llega de Windows como `REMOVED` + `ADDED`, y el debouncer no los une. F4a los une si, en el mismo lote, hay un único borrado y un único creado con el mismo nombre de fichero y a menos de 20 ms. Si además cambia el nombre, queda como borrado + creado sin `from`: el respaldo previsto en G7. (SPEC §7.1, §13.2)
- **Guardado atómico de Claude Code** (medido con Desktop 2.1.321): `Edit` y `Write` escriben `<nombre>.tmp.<pid>.<hex>`, borran el original si existe y renombran el temporal sobre él. El Watcher descarta los temporales y deja un único `modified` (o `created`, si el fichero es nuevo). Para distinguirlos usa la fecha de creación, que NTFS conserva del fichero sustituido (*tunneling*). (SPEC §7.1)
- **`file_events`:** columnas añadidas `id` (para actualizar la atribución), `ext` (la agregación no depende de la ruta) y `settled` (atribución definitiva, §7.3). `rel_path` admite `NULL` para `privacy.record_paths = false`. Un `CHECK` exige que `actor_session` y `actor_tool_use_id` vayan juntos (R-4). (SPEC §6.4)
- **Rutas de las herramientas:** el mod y el parser las relativizan al `cwd` de la sesión, y el Watcher a la raíz de Git. La atribución las traduce con el prefijo de `git rev-parse --show-prefix`. Una edición fuera del `cwd` pero dentro de la raíz llega como `<externo>` y cae en R2. (SPEC §7.3, §10)
- **Raíces sin Git:** no se vigilan si son una unidad entera, la carpeta del usuario o una que la contenga (sin `.gitignore`, sería vigilar medio disco). Las unidades de red se detectan solo por ruta UNC; una unidad de red con letra no se detecta. (SPEC §7.1)
- **Raíces del histórico:** al arrancar se resuelve en segundo plano la raíz del `cwd` de todas las sesiones guardadas, para que la agregación sea por proyecto. Los triggers de §6.4.2 recalculan lo que ya estaba agregado con `root_id = 0`.
- **Estado de Git** (rama, `HEAD`, ficheros con cambios y `numstat`) solo en memoria, publicado a la interfaz. No se guarda en SQLite. (SPEC §7.2)
- **Vista «Cambios en directo»:** agrupa por herramienta y sesión, no por turno, porque los eventos de herramienta del mod no llevan el turno. El diff y la vista previa solo se admiten sobre raíces activas. (SPEC §7.4)
- **Compactación:** al arrancar (tras el backfill) y cada 24 h. La primera ejecución sobre la base real agrega todos los días cerrados y purga las rutas de más de 30 días (D3).
- **Fallo previo corregido:** `PidProbe` no devolvía ningún proceso vivo si la lista tenía un PID repetido.

## Cierre de F3: G3, G4 y desviaciones aceptadas (2026-11-12)

No cambia la arquitectura ni el alcance. Resultados en [`perf/2026-11-11-f3.md`](perf/2026-11-11-f3.md), sección «Actualización».

- **G4 resuelta: sí.** Las herramientas de un subagente llegan al mod con `agentId`. (SPEC §13.2)
- **G3 resuelta: sí**, el tiempo de la herramienta incluye el diálogo de permisos. **Cambia la señal** del estado `awaiting_permission`: la SPEC preveía `classic.PermissionRequest`, pero el motor no lo entrega al mod en una sesión interactiva (en 2.1.321 lo atiende antes un plugin del propio motor). El mod observa el veredicto de `tool.check` y lo devuelve intacto; un `ask` marca la espera. No hay señal del momento en que se contesta: la espera se cierra con el fin de la herramienta. (SPEC §5.2, §13.2)
- **Contrato:** eventos nuevos `permission_requested` y `permission_resolved` en `/v1/events`, y estado `awaiting_permission` en `tool_calls.status`. Sin migración. (SPEC §6.2)
- **B3:** `perf_mark` distingue la latencia de una herramienta nueva (`tool`) del resto de actualizaciones (`other`, B4).
- **Desviaciones de los criterios de F3, aceptadas por el usuario (2026-11-12):**
  - B11 con una muestra por motor en lugar de 10, y S2 sin medir en una sesión real (cubierto por el test del mod).
  - B2 (coste propio del mod < 1 ms) sin medir de forma aislada; cota superior de 1,3 ms por llamada con todo incluido.
  - Desktop 2.1.317 sin validar con su binario: ya no está instalado. El mod compila contra sus tipos.
  - Hooks `classic.Task*` y `classic.Subagent*` aplazados a F4b.

## Precisiones de F3: mod puente (2026-11-11)

No cambian la arquitectura ni el alcance. Resultados en [`perf/2026-11-11-f3.md`](perf/2026-11-11-f3.md).

- **Matriz de motores:** Desktop 2.1.317 ya no está instalado (la aplicación se actualizó a 2.1.321). El mod compila contra sus tipos, pero no se puede validar ni probar con su binario. (SPEC §12)
- **G1:** `session.measure` se dispara en 2.1.289, además de existir.
- **G3 y G4 seguían abiertas** en esta fecha, junto con B3 y la prueba E2E, por falta de sesión iniciada. Se cerraron después: ver la entrada del 2026-11-12.
- **El mod es un solo fichero** (`hooks/register.ts`), sin `privacy.ts` aparte: no está comprobado que el motor cargue módulos relativos. La lista blanca del mod es un espejo de `cockpit-core/src/privacy.rs`. (SPEC §5.1)
- **Hooks aplazados a F4b:** `classic.TaskCreated/TaskCompleted` y `classic.SubagentStart/SubagentStop` alimentan el Task Monitor y se añaden con él. El `hello` ya envía `agents`, que la app acepta y todavía no guarda. `turn.start` no se envía. (SPEC §5.2)
- **Reloj:** el mod usa `Date.now()` en lugar de `$.clock.now()`, que es asíncrono y añadiría una espera dentro del hook.
- **`hook .catch`:** el validador de 2.1.321 marca `tool.call` como hook que puede bloquear. Lleva un `.catch` que deja pasar la herramienta si el código del mod falla.
- **Orden de `usage_snapshots`:** el último es el último recibido, no el de marca de tiempo más alta.
- **Tablas nuevas (migración 2):** `mod_instances`, `turns` y `usage_snapshots`, y la columna `sessions.ended_at`.
- **`DEV_COCKPIT_HTTP_LOG`:** registro de depuración de las peticiones del mod, solo para pruebas.

## Precisiones de F2: shell Tauri y vista En vivo (2026-11-11)

No cambian la arquitectura ni el alcance. Mediciones en [`perf/2026-11-11.md`](perf/2026-11-11.md).

- **G9 resuelta:** `GetProcessInfos` funciona desde Tauri 2.12 con `webview2-com` 0.39. (SPEC §13.2)
- **Criterio de validez de C2:** `GetProcessInfos` no incluye el proceso `crashpad-handler`. Se excluye de la comparación entre métodos y se suma a la memoria. (SPEC §11.4)
- **Identificador de la app:** pasa de `com.devcockpit.app` a `dev-cockpit`, para que los datos de WebView2 queden en `%LOCALAPPDATA%\dev-cockpit\EBWebView` y los de la app en `%APPDATA%\dev-cockpit\`. (SPEC §4, §11.4)
- **Escritor único:** el hilo de la canalización en vivo es el único que escribe en SQLite. No hay un `DbWriter` aparte, porque en F2 solo hay una fuente que escribe. El estado en vivo es una caché de consultas a SQLite, no un agregado paralelo.
- **`unsafe_code`:** el lint del workspace pasa de `forbid` a `deny`. El único uso permitido es el COM de WebView2 en `src-tauri/src/procs.rs`.
- **Variables `DEV_COCKPIT_*`:** redirigen las carpetas de entrada y la base de datos, solo para pruebas y mediciones.
- **Sin hacer todavía:** las notificaciones nativas y el plugin `log`. El nombre de sesión del registro no se lee, porque no está en la lista blanca.

## Precisiones de F1: ingesta de transcripts (2026-11-11)

No cambian la arquitectura ni el alcance. Salen de implementar F1 contra los transcripts reales.

- **Desempate de la sesión canónica (R-4):** las copias de historial conservan las marcas de tiempo originales, así que `started_at` empata entre la sesión original y la reanudada. Se añade `sessions.file_created_at` y el orden pasa a ser `(started_at, file_created_at, id)`. Con esta regla, el backfill real marca 399 copias, la misma cifra que M8. (SPEC §6.4)
- **`cost-state` en `sessions`:** la última línea `cost-state` de cada transcript se guarda en columnas `cost_state_*` de `sessions`, para la prioridad de coste de SPEC §3 y para la verificación de F1.
- **El criterio «deduplicación igual a `cost-state`» no es una igualdad general:** coincide exactamente en 19 de 33 sesiones reales. En el resto, `cost-state` incluye consumo que el transcript no registra. Detalle en [`perf/2026-11-11.md`](perf/2026-11-11.md). (SPEC §15, F1)
- **Aplazado dentro de F1:** la tabla `pricing` se crea vacía (el coste estimado no se usa hasta que haya interfaz), y `file_offsets` detecta truncados por tamaño, sin `file_id`. La vigilancia en vivo con `notify` y el hilo `DbWriter` pasan a F2, que es donde hay fuentes en vivo.
- **Validado con el toolchain GNU:** `rusqlite bundled` compila y pasa sus tests.

## Desviación de F0: toolchain GNU en lugar de MSVC (2026-11-11)

Cambia el toolchain de compilación. No cambia la arquitectura, ni las dependencias de la app, ni el alcance.

- VS Build Tools exige permisos de administrador y este equipo no los tiene. F0 usa `stable-x86_64-pc-windows-gnu` con MinGW-w64 portátil (WinLibs), instalados en el perfil de usuario. Queda fijado en `rust-toolchain.toml`. (SPEC §2, §15 F0)
- `src-tauri` compila solo como `rlib`: con el target GNU, la DLL de depuración (`cdylib`, que solo hace falta para móvil) supera el límite de símbolos exportados del formato PE.
- **Aviso conocido, sin resolver:** al enlazar el ejecutable, `ld` emite `.rsrc merge failure: multiple non-default manifests`. La ventana se abre igualmente. Hay que revisarlo antes del instalador de F5.
- **Pendiente de validar con este toolchain:** G9 (`webview2-com`, F2), `rusqlite bundled` (F1) y el instalador NSIS (F5).

## Corrección documental: W6 y remoto (2026-11-10)

Corrige un dato del estado del repositorio. No cambia la arquitectura, ni las reglas, ni el alcance.

- W6 decía «no hay remoto», pero el repositorio tiene configurado `origin` (GitHub). Se corrige el texto y se mantiene la regla: ningún `push` ni ningún cambio de remotos sin autorización. (SPEC §17 W6, [`DEVELOPMENT_WORKFLOW.md`](DEVELOPMENT_WORKFLOW.md) W6)
- El estado de partida de W9 menciona ahora el remoto. En `DEVELOPMENT_WORKFLOW.md` §2 se deja constancia de que W9 ya se completó (`0eb84f5`). (SPEC §17 W9, [`DEVELOPMENT_WORKFLOW.md`](DEVELOPMENT_WORKFLOW.md) §2)

## v1.2.2 + revisión correctiva final: **aprobada** (2026-11-10)

No cambia la arquitectura, ni añade funcionalidades, dependencias o fases.

### v1.2.2
- **`tool_calls` con definición completa**, que incluye las columnas `host`, `root_id` y `day`. Las rutas y el host solo pueden ir en las columnas `rel_path` y `host`, nunca en `meta_json`. (SPEC §6.4, §6.4.1)
- **Agregación transaccional e idempotente:**
  - claves primarias de `file_change_daily` `(root_id, day, actor, change, ext)` y de `tool_usage_daily` `(root_id, day, tool)`;
  - registro `aggregation_ledger`;
  - una transacción por `(target, root_id, day)`;
  - la purga solo actúa sobre días en estado `done`.
  
  (SPEC §6.4.2, ADR 0011)
- **Flujo de desarrollo formalizado** con las reglas W1–W11 y el procedimiento W9 para inicializar `main`. (SPEC §17, [`DEVELOPMENT_WORKFLOW.md`](DEVELOPMENT_WORKFLOW.md))
- **La instalación permanente del plugin requiere autorización previa**, porque modifica `~/.claude`. Las pruebas usan `--plugin-dir`. (SPEC §5.3, §12, §15)

### Revisión correctiva final (SPEC §0.4)
- **R-1:** autorización **explícita**, por prueba y por motor, antes de cualquier sesión de prueba o comando que pueda escribir en `~/.claude`. Avisar no basta. (SPEC §4.3, §12 paso 1, §15 G-pre, W11)
- **R-2:** `COALESCE(root_id, 0)` en las agrupaciones, en el registro, en la purga, en la reconciliación y en los triggers.
  - Los triggers `AFTER UPDATE` marcan como sucio el target anterior (`OLD`) y el nuevo (`NEW`).
  - La purga no dispara triggers.
  - Los días ya purgados de `file_events` son inmutables.
  
  (SPEC §6.4.2)
- **R-3:** F4a cita §6.4.1 (retención) y §6.4.2 (agregación y purga). (SPEC §15)
- **R-4:** `tool_use_id` **no es único entre sesiones**. La medición M8 encontró 399 ids repetidos, todos copias de historial. Por eso:
  - la clave de `tool_calls` pasa a ser `(session_id, tool_use_id)`, con `is_copy`;
  - `api_messages` conserva su PK `message_id` y añade la regla de sesión canónica (la más antigua).
  
  (SPEC §3, §6.3, §6.4, §6.4.2, §7.3, §11.1)

## v1.2.1

- Contrato documentado de `POST /v1/hello`: autenticación, petición, respuesta, política de privacidad e idempotencia. Idempotencia de `/v1/events` por `(instance_id, seq)`. (SPEC §6.2.1, ADR 0012)
- Retención de 30 días para **todo** el detalle de rutas. Los agregados sin rutas se conservan indefinidamente. `file_change_daily` deja de guardar `top_dir`. Nuevo `tool_usage_daily`. El ajuste pasa a llamarse `retention.path_detail_days`. (SPEC §6.4, §6.4.1, §10, ADR 0011)
- Política `PRE_HANDSHAKE` en el mod: sin títulos y sin rutas (`cwd` incluido) hasta el primer `hello` correcto. Después se aplica la política autorizada por la app. (SPEC §5.4, §10, ADR 0008)
- G10: si falla, **se detiene el trabajo** y se presentan las alternativas A–E. Ninguna se elige automáticamente, y escribir en `~/.claude` está prohibido sin autorización. (SPEC §13.2)
- Atribución de cambios por **intervalo completo** `[ToolStarted − ε, ToolFinished + δ]`, con ventana de asentamiento. G5 con objetivos de precisión. (SPEC §7.3)
- Primera versión de `DEVELOPMENT_WORKFLOW.md`. (SPEC §17)

## v1.2

- **D1, cerrada:** títulos de tarea truncados a 120 caracteres. Se pueden desactivar con `privacy.capture_task_titles`.
- **D2, cerrada:** el ejecutable de Bash no se registra por defecto (`privacy.capture_shell_program = false`).
- **D3, cerrada:** 30 días de detalle de cambios de fichero. Las métricas agregadas se conservan a largo plazo.
- **C1:** reconexión automática del mod sin reiniciar Claude (máquina de estados y vigilancia de `endpoint.json` solo mientras no hay conexión). (SPEC §5.4, ADR 0012)
- **C2:** los benchmarks de memoria aíslan el grupo WebView2 de DEV COCKPIT (`GetProcessInfos` contrastado con el árbol de procesos y `--user-data-dir`; métrica: *private bytes*). (SPEC §11.4, G9)
- Revisión de coherencia: timers del mod, tamaño de `file_events`, decisiones abiertas y cerradas.

## v1.1

- **Workspace Watcher**, con Git de solo lectura (`--no-optional-locks`) y diffs bajo demanda, sin guardar contenido. (SPEC §7, ADR 0007)
- **Live Tool Monitor:** `ToolStarted` antes de `await next(e)` y `ToolFinished` en `finally`. Flush bajo demanda. (SPEC §5.2)
- **Task Monitor:** progreso calculado con las tareas declaradas por el agente (TaskCreate/TaskUpdate, TodoWrite, `update_plan` del Desktop) y el estado de los subagentes. **Nunca** con el número de llamadas. (SPEC §8, ADR 0010)
- **Project Map** ligero, basado en manifiestos. El análisis de imports queda para v2. (SPEC §9)
- **Compatibilidad:** matriz de motores reales y versión mínima formal. (SPEC §12)
- **Privacidad:** lista blanca de metadatos aplicada en tres puntos. (SPEC §10, ADR 0008)
- **Rendimiento:** `session.measure` (push) sustituye el sondeo de 5 s. Se eliminan los sondeos innecesarios. (SPEC §11.3, ADR 0009)
- Presupuestos separados en **Objetivo** y **Medido**, con las mediciones reales M1–M7 y la batería B1–B10. (SPEC §11)

## v1 (base)

- Arquitectura Tauri 2 + Rust + React/WebView2 + SQLite y mod ligero de Claude Code como puente. (ADR 0001, 0002, 0005)
- IPC por Channel (ADR 0003). rusqlite con un solo escritor en WAL (ADR 0004). Reconciliación de fuentes de datos (ADR 0006).
- Alcance: solo observabilidad, uso personal en Windows, métricas y metadatos.
