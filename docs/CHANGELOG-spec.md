# Historial de cambios de la especificación DEV COCKPIT

Documento de referencia: [`SPEC.md`](SPEC.md). Las tablas detalladas de cada versión están en SPEC §0 (v1.1), §0.1 (v1.2), §0.2 (v1.2.1), §0.3 (v1.2.2) y §0.4 (revisión final). Las decisiones de fondo se justifican en los ADR de [`adr/`](adr/).

---

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
