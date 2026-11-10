# Resultados de G-pre: G1 y G10 (2026-11-10)

Las pruebas T1–T3 se ejecutaron con la autorización explícita del usuario (W11), según el [README](README.md). No se envió ningún prompt (sin llamadas al modelo), no se instaló nada y no se modificaron `settings.json` ni los plugins.

## Veredicto

| Puerta | Resultado | Motores |
|---|---|---|
| **G1** | **SUPERADA** | CLI 2.1.289, Desktop 2.1.317 y Desktop 2.1.321 |
| **G10** | **SUPERADA**, alternativas A–E innecesarias | CLI 2.1.289, Desktop 2.1.317 y Desktop 2.1.321 |

**`minEngine` (SPEC §12): 2.1.289 queda confirmado.** Es el motor más antiguo de la matriz con todas las APIs del diseño y supera G1 y G10.

## G1: APIs y eventos

| Comprobación | 2.1.289 | 2.1.317 | 2.1.321 | Evidencia |
|---|---|---|---|---|
| `session.version`, `session.id`, `session.usage` (`startedAt`, `context`, `rateLimits`, `cost`) | ✔ | ✔ | ✔ | `results/core-*.json` → `g1` |
| `agent.list` | ✔ | ✔ | ✔ | ídem |
| `clock.now`, `clock.after`, `clock.every` | ✔ | ✔ | ✔ | ídem |
| `env.get`, `fs.exists`, `fs.stat`, `fs.read`, `fs.write`, `http.fetch`, `ui.status`, `command.register` | ✔ | ✔ | ✔ | ídem, y `g10` / `fsWrite` |
| Registro de `session.measure`, `classic.TaskCreated` y `classic.TaskCompleted` (el módulo cargó) | ✔ | ✔ | ✔ | `results/events-*.json` |
| Declaraciones en los tipos generados por cada motor (`session.measure`, `TaskCreated`, `TaskCompleted`, `agent.list`, `http.fetch`, `clock.every`, `tool.call`, `turn.complete`) | ✔ | ✔ | ✔ | `types-snapshots/<versión>/claude-code.d.ts`: 15.197, 17.305 y 17.719 líneas |

- `rateLimits` vino vacío en las tres sesiones. Es lo esperado: no hubo ninguna respuesta de la API, porque no se envió ningún prompt, y además la sesión de la CLI no tenía login (SPEC R5).
- **No se ha comprobado que los eventos se disparen** (`session.measure` y `classic.Task*`), porque eso exige un turno del modelo. Es la prueba opcional T-x, que no está autorizada.
- No había `tsc` disponible. Instalarlo sería instalar una herramienta, así que G1 se verificó en tiempo de ejecución y con `grep` sobre los tipos de cada motor.

## G10: `endpoint.json` fuera de `~/.claude` y reconexión (S1)

| Medición | 2.1.289 | 2.1.317 | 2.1.321 |
|---|---|---|---|
| Sesión arrancada **sin** endpoint (S1) | ✔ (ENOENT en el primer intento) | ✔ | ✔ |
| `$.fs.stat` / `$.fs.read` de `%APPDATA%\dev-cockpit-probe\endpoint.json` | ✔ | ✔ | ✔ |
| `GET /health` con Bearer / sin Bearer | 200 / **401** | 200 / **401** | 200 / **401** |
| `POST /hello` | 200 | 200 | 200 |
| Reconexión: de que se escribe el endpoint a que llega la primera petición | **1,06 s** | **2,31 s** | **1,35 s** |
| Ticks de vigilancia (5 s) | 4 | 4 | 4 |
| Reinicio de Claude necesario | **No** | **No** | **No** |

Se usó la ruta `%APPDATA%`, la principal del diseño. Las alternativas A–E de SPEC §13.2 no hacen falta.

## Hallazgos para la implementación (F3)

1. **Analizador estático de módulos (los tres motores).** Antes de cargar un módulo, el motor rechaza:
   - pasar `$` a una función que **no** esté declarada al nivel superior del fichero;
   - leer `$` como valor, en vez de escribirlo siempre como `$.nombre.método(...)`;
   - guardar o devolver el valor de `on(...)`, que solo admite `.catch(...)`.
   
   La primera versión de la sonda se rechazó por eso en 2.1.289. Hay que tenerlo en cuenta en el código del mod real.
2. **Detección de capacidades:** no se puede detectar si una API existe leyendo `$` de forma dinámica (por ejemplo con `typeof $.x`). La compatibilidad entre motores se valida con los tipos de cada motor (SPEC §12), no en tiempo de ejecución.
3. **Registros obsoletos en `~/.claude/sessions/`:** al cerrar una sesión de golpe (cerrando la pestaña) queda `sessions/<pid>.json` con un PID muerto. **Claude lo limpia solo en el siguiente arranque**, como se observó en T2 y T3. Confirma que `SessionRegistryWatch` tiene que comprobar si el PID sigue vivo (SPEC §11.3, R8).
4. **El arranque escribe `~/.claude/backups/.claude.json.backup.*` y `~/.claude/cache/`.** Es comportamiento normal de Claude; el README lo agrupaba dentro de «datos de ejecución».

## Estado que quedó después de las pruebas

- **`%APPDATA%\dev-cockpit-probe\`:** cada servidor borró su `endpoint.json` al terminar y la carpeta vacía se ha eliminado.
- **`~/.claude/sessions/`:** queda el registro obsoleto de T3. Claude lo limpiará en el próximo arranque, y no se ha borrado a mano porque escribir en `~/.claude` requiere autorización.
- Los transcripts de las sesiones de prueba (si existen) se quedan en `~/.claude/projects/`, como los de cualquier sesión.
- **`settings.json` y `plugins/known_marketplaces.json`:** sin cambios. Sus fechas de modificación son anteriores a las pruebas.
