# Spike G-pre: sonda de G1 y G10 (desechable)

Referencias: [`docs/SPEC.md`](../../docs/SPEC.md) §12, §13.2 (G1, G10) y §15 (G-pre), y [`docs/DEVELOPMENT_WORKFLOW.md`](../../docs/DEVELOPMENT_WORKFLOW.md), regla W11.
**No es código de la aplicación.** Se descartará cuando G1 y G10 estén resueltas.

## Qué comprueba

| Puerta | Pregunta | Evidencia que produce |
|---|---|---|
| **G1** | ¿Existen en cada motor las APIs y eventos del diseño (`session.usage/version/id`, `agent.list`, `http.fetch`, `fs.stat/read/write`, `clock.after/every/now`, `env.get`, `session.measure`, `classic.TaskCreated/TaskCompleted`)? | `results/core-<versión>.json` (`g1Apis`), `results/events-<versión>.json` (eventos que se registraron) y la declaración de tipos que cada motor deja en `mod-*/.claude-plugin/types/claude-code/index.d.ts` (se revisa con `grep`; no hay `tsc` instalado) |
| **G10** | ¿Puede el mod leer `%APPDATA%\dev-cockpit-probe\endpoint.json`, que está fuera de `~/.claude`, y conectar por `127.0.0.1` con Bearer? ¿Funciona la reconexión cuando la «app» aparece después de la sesión (escenario S1 de SPEC §5.4)? | `results/core-<versión>.json` (`g10`: `stat`/`read`/`health`/`hello`, `reconnect.waitedMs`) y `results/server-<etiqueta>.json` |

Diseño:
- **`mod-core`** comprueba las APIs, G10 y la reconexión. Vigila el endpoint cada 5 s y se rinde a los 180 s.
- **`mod-events`** solo registra los eventos que G1 tiene que comprobar. Va **separado** para que, si un motor antiguo rechaza algún nombre de evento, `mod-core` siga funcionando.
- **`harness/probe-server.mjs`** hace de app: `127.0.0.1`, puerto aleatorio, Bearer obligatorio, rechazo de `Origin` y comprobación de `Host`. Escribe `endpoint.json` tras `--delay-ms` y lo borra al terminar.
- Los resultados se sanean: sin rutas personales (se sustituyen por `%APPDATA%`/`%LOCALAPPDATA%`), sin el token, sin contenido.
- **Ninguna prueba envía prompts:** no hay llamadas al modelo ni consumo de tokens. Ningún hook bloquea ni reescribe nada.

## Pruebas (cada una necesita autorización explícita previa, W11)

Se ejecutan desde la raíz del repo, en PowerShell, cada una en una pestaña de Terminal.

| ID | Motor | Comando de sesión |
|---|---|---|
| **T1** | CLI 2.1.289 | `$env:DISABLE_AUTOUPDATER='1'; & "$env:USERPROFILE\.local\bin\claude.exe" --plugin-dir spike/g1-g10-mod-probe/mod-core --plugin-dir spike/g1-g10-mod-probe/mod-events` |
| **T2** | Desktop 2.1.317 | `$env:DISABLE_AUTOUPDATER='1'; & "$env:APPDATA\Claude\claude-code\2.1.317\a33aa5518497\claude.exe" --plugin-dir spike/g1-g10-mod-probe/mod-core --plugin-dir spike/g1-g10-mod-probe/mod-events` |
| **T3** | Desktop 2.1.321 | `$env:DISABLE_AUTOUPDATER='1'; & "$env:APPDATA\Claude\claude-code\2.1.321\d020ce1b5d30\claude.exe" --plugin-dir spike/g1-g10-mod-probe/mod-core --plugin-dir spike/g1-g10-mod-probe/mod-events` |

Secuencia de cada prueba:
1. Arrancar el servidor en segundo plano: `node spike/g1-g10-mod-probe/harness/probe-server.mjs --label <T#-versión> --delay-ms 20000 --timeout-ms 240000`.
2. Arrancar la sesión interactiva antes de que pasen esos 20 s. Así, la sesión arranca **sin** endpoint, como en S1.
3. Esperar a que aparezcan `results/core-<versión>.json` con `g10.reconnect` y `results/events-<versión>.json`, unos 30–60 s.
4. Cerrar la sesión con `/exit` (o cerrando la pestaña) **sin enviar ningún prompt**. El servidor termina solo y borra `endpoint.json`.

`DISABLE_AUTOUPDATER=1` solo afecta a ese proceso: evita que el CLI se actualice a mitad de la prueba y cambie la versión que se está probando. No modifica ninguna configuración.

### Lo que escribe cada prueba (declarado antes de pedir la autorización)

| Dónde | Qué | ¿Persiste? |
|---|---|---|
| `~/.claude/` | Datos de ejecución normales de cualquier sesión: `sessions/<pid>.json`, `session-env/<id>/`, `shell-snapshots/` y, posiblemente, un transcript en `projects/<carpeta>/<sesión>.jsonl`, aunque no se envíe ningún prompt. No es seguro que se cree. | Sí, como en cualquier sesión. No se instala nada ni se cambian `settings.json`, plugins ni marketplaces. |
| `~/.claude.json` (fuera de `~/.claude/`) | Contadores y metadatos de arranque que Claude actualiza al iniciarse: número de arranques, última sesión del proyecto… | Sí. La carpeta ya es de confianza (`hasTrustDialogAccepted: true`), así que no aparecerá el diálogo de confianza. |
| `%APPDATA%\dev-cockpit-probe\endpoint.json` | Puerto y token aleatorios de la sonda | No: se borra al terminar el servidor |
| Repo: `spike/g1-g10-mod-probe/results/*.json` | Informes saneados | Sí, sin commit hasta que lo autorices |
| Repo: `mod-*/.claude-plugin/types/` | Tipos generados por el motor, con su propio `.gitignore` | Sí, sin commit |

Además, como cualquier sesión, el arranque se comunica con los servidores de Anthropic (autenticación, políticas). No se envía ningún prompt.

### Prueba opcional T-x (no incluida en T1–T3)
Que `session.measure` y `classic.TaskCreated` **se disparen** exige un turno del modelo (consume tokens) y que el modelo cree una tarea. G1 solo pide que **existan**, y eso lo cubren los tipos y el registro de los eventos. T-x solo se ejecutaría con una autorización aparte.

## Si G10 falla
Se detiene el trabajo y se presentan las alternativas A–E de SPEC §13.2. **No se elige ninguna automáticamente.** Las alternativas D y E escriben en `~/.claude` y necesitan tu autorización.
