# ADR-0007: Workspace Watcher con `notify` y Git CLI estrictamente de solo lectura

- **Estado:** Aceptada (2026-11-10), SPEC v1.2.2
- **Referencias:** SPEC §7, §11.1 (M3–M6), §13.2 (G5, G7)

## Contexto
El cockpit tiene que mostrar en directo los cambios de código de los proyectos activos y sus diffs **sin escribir nunca** en los repositorios. Hay tres hechos medidos que condicionan el diseño:
- `git status` puede reescribir el índice, salvo con `--no-optional-locks`. Con esa opción, M5 confirmó que el índice queda intacto.
- Los repos tienen hasta 70.502 ficheros en disco frente a 139 versionados (M6).
- `status` tarda entre 72 y 117 ms y `diff --numstat` entre 128 y 687 ms (M3, M4).

## Decisión
- Un watcher `notify` recursivo por raíz activa (una por worktree), con `notify-debouncer-full` (200 ms, unión de renombrados).
- Filtro con el crate `ignore` (`.gitignore`) más exclusiones fijas: `node_modules`, `target`, `dist`, `.venv`, `Library/`…
- Modo «cambio masivo» cuando hay más de 500 eventos por segundo, y resincronización con `git status` cuando llega un `Rescan`.
- Se usa el **binario `git` del sistema**, con estas opciones en cada invocación: `--no-optional-locks -c core.fsmonitor=false -c gc.auto=0 -c maintenance.auto=false`, `GIT_OPTIONAL_LOCKS=0` y `GIT_TERMINAL_PROMPT=0`.
- **Lista blanca de subcomandos:** `status`, `diff`, `rev-parse`, `ls-files`, `log -n`, `show --stat`, `worktree list`, `cat-file -s`. Los diffs llevan `--no-ext-diff --no-textconv`.
- Una consulta en vuelo por repositorio, con coalescencia de 500 ms. **Git nunca se sondea.**
- Los diffs y las vistas previas viajan solo en la respuesta: no se guarda contenido en ningún sitio.
- La atribución de cada cambio a una herramienta se hace por intervalo completo (SPEC §7.3).

## Alternativas consideradas
- **`git2` o `gix`:** no garantizan la misma configuración que tiene el usuario (autocrlf, filtros, LFS, `safe.directory`) y existe riesgo con el índice. Se reconsiderarán si B7 muestra un cuello de botella.
- **`PollWatcher` general:** demasiado caro con decenas de miles de ficheros. Descartado.

## Consecuencias
- **A favor:** misma configuración que el usuario, ninguna escritura (verificable con el test M5) y un coste acotado.
- **En contra:** lanzar un proceso por consulta tiene un coste. Las unidades de red quedan fuera de v1.

## Validación
B6 y B7, el test M5 automatizado, G5 (precisión de la atribución) y G7 (renombrados en NTFS).
