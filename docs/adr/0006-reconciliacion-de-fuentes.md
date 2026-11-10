# ADR-0006: Reglas de reconciliación entre fuentes y claves idempotentes

- **Estado:** Aceptada (2026-11-10), SPEC v1.2.2 con la revisión final R-4
- **Referencias:** SPEC §2, §3, §6.3, §6.4, §11.1 (M7, M8)

## Contexto
Un mismo hecho puede llegar por varias fuentes: el mod, el transcript, el registro de sesiones, el sistema de ficheros y Git. Hay dos hechos medidos que condicionan el diseño:
- **M7:** las filas `assistant` repiten el mismo `usage` unas 2,5 veces por `message.id`.
- **M8:** 399 `tool_use_id` y 242 `message.id` aparecen en **varias sesiones**, como copias de historial con el mismo `timestamp` y el mismo `uuid`.

## Decisión
- **Tokens:** manda el transcript, deduplicado por la PK `message_id`. La sesión que se asigna es la **canónica**, la de `started_at` más antiguo.
- **Contexto %, rate limits y coste en vivo:** solo los aporta el mod. Sin mod se muestra «n/d», o una estimación marcada como tal.
- **Herramientas:** la clave es **`(session_id, tool_use_id)`**. Las copias de historial se marcan con `is_copy = 1` y no se cuentan en los agregados.
- **Tareas:** la clave es `(session_id, source, task_id)`.
- **Transporte:** la clave es `(instance_id, seq)`, con marca de agua `ack_seq`.
- **Coste:** se trata siempre como estimación del cliente. Prioridad: mod > `cost-state` > uso × tabla de precios.
- Todas las escrituras son *upserts* idempotentes.

## Alternativas consideradas
- **`tool_use_id` como clave global:** M8 lo refuta, porque las copias sobrescribirían la fila o se contarían dos veces. Descartado.
- **Sumar el `usage` por fila:** infla los tokens unas 2,5 veces. Descartado.

## Consecuencias
- **A favor:** sin dobles conteos y con una atribución estable aunque el backfill procese primero lo más reciente.
- **En contra:** hay que mantener las marcas `is_copy`. Si cambia la sesión canónica, sus targets se marcan como sucios (ADR-0011).

## Validación
- Test bloqueante de deduplicación (M7).
- Tests R-4 de §6.4.2: una herramienta y un mensaje presentes en dos sesiones se cuentan una sola vez.
- Comparación de los totales con `cost-state` y `/cost` (F1, E2E).
