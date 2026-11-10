# ADR-0010: El Task Monitor mide el progreso con las tareas declaradas, no con el número de llamadas

- **Estado:** Aceptada (2026-11-10), SPEC v1.2.2
- **Referencias:** SPEC §2, §8, §10 (D1), §13.2 (G6)

## Contexto
El usuario exige no confundir el número de llamadas a herramientas con el progreso real. En el historial de este equipo, las tareas se declaran así:
- `TaskCreate`/`TaskUpdate`: 0 usos;
- `TodoWrite`: 35;
- `mcp__ccd_turn__update_plan` del Desktop: **426**, con un esquema no publicado [INT].

## Decisión
- **El progreso es lo que declara el agente:** tareas completadas / total, la tarea en curso (`in_progress`) y los bloqueos (`blockedBy`).
- Las tareas se obtienen con cuatro adaptadores:
  - **A1:** herramientas Task y `classic.TaskCreated`/`TaskCompleted` [DOC];
  - **A2:** `TodoWrite` [DOC];
  - **A3:** `update_plan` del Desktop [INT]+[EXP], aislado y desactivable;
  - **A4:** subagentes mediante `$.agent.list()` y `AgentStatus` [DOC], consultados por evento.
- El número de herramientas aparece solo como «actividad», en una sparkline aparte.
- Si la sesión no declara tareas, se muestra «sin lista de tareas declarada» y nunca un progreso inventado.
- Los títulos se truncan a 120 caracteres (D1). Con `capture_task_titles = false` se muestra «Tarea n.º k».

## Alternativas consideradas
- **Calcular el progreso con el número de llamadas o con heurísticas de tiempo:** son engañosos, y el usuario los ha prohibido. Descartado.
- **Usar solo A1:** en este equipo no habría ningún dato. Descartado.

## Consecuencias
- **A favor:** el progreso es fiel a lo que dice el agente, y funciona en el CLI y en el Desktop.
- **En contra:** A3 depende de un formato interno y puede romperse (R16).

## Validación
G6 (fixtures de A3 en 2.1.317 y 2.1.321), E2E con `TodoWrite` y `update_plan` (F4b) y G4 (subagentes).
