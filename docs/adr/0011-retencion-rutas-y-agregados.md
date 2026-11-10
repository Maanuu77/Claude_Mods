# ADR-0011: Retención de 30 días del detalle de rutas y agregados indefinidos con compactación transaccional

- **Estado:** Aceptada (2026-11-10), SPEC v1.2.2 con la revisión final R-2
- **Referencias:** SPEC §6.4, §6.4.1, §6.4.2, §7.4, §10, D3

## Contexto
El usuario fijó dos condiciones:
- el detalle de rutas de fichero se conserva **30 días**, de forma coherente en todas las tablas;
- las estadísticas agregadas se conservan indefinidamente, **sin rutas individuales**.

Además, la compactación no puede duplicar ni perder datos si se interrumpe, y los datos tardíos (backfill, atribución, reasignación de proyecto, cambios de `is_copy`) tienen que reflejarse en los agregados.

## Decisión
- **Retención:**
  - `file_events` se borra a los 30 días;
  - en `tool_calls`, `rel_path` y `host` se anulan a los 30 días, pero la fila se conserva;
  - `ingest_errors` se borra a los 30 días.
  
  El plazo lo fija `retention.path_detail_days = 30`.
- **Agregados sin rutas:**
  - `file_change_daily`, con PK `(root_id, day, actor, change, ext)`;
  - `tool_usage_daily`, con PK `(root_id, day, tool)`, que excluye las filas `is_copy`.
- **Registro `aggregation_ledger`** con PK `(target, root_id, day)` y `state` `done` o `dirty`.
- **Normalización:** el centinela `0` significa «sin proyecto». Se aplica `COALESCE(root_id,0)` en las agrupaciones, el registro, la purga, la reconciliación y los triggers.
- **Unidad de trabajo:** una transacción `BEGIN IMMEDIATE` por cada `(target, rk, day)`, con estos pasos:
  1. borrar el agregado de ese día;
  2. recalcularlo desde la tabla cruda;
  3. insertar el resultado;
  4. marcar el día como `done` en el registro;
  5. `COMMIT`.
  
  Si se interrumpe, se deshace entera.
- **Purga:** en otra transacción posterior, y solo sobre días en `done`. No dispara triggers:
  - el trigger de `tool_calls` no observa `rel_path` ni `host`;
  - `file_events` no tiene trigger `AFTER DELETE`.
- **Triggers `AFTER INSERT/UPDATE/DELETE`:** marcan `dirty` el target `OLD` **y** el `NEW`, normalizados. Así, una reasignación no deja datos obsoletos en el proyecto anterior.
- **Invariante:** el escritor rechaza cualquier inserción o actualización en un día ya purgado de `file_events`.

## Alternativas consideradas
- **Agregación incremental (sumar deltas):** no es idempotente si se reintenta. Descartado.
- **Conservar `top_dir` en los agregados:** es un fragmento de ruta y contradice la condición de «sin rutas individuales». Descartado (v1.2.1).

## Consecuencias
- **A favor:** sin duplicados ni pérdidas en caso de interrupción, y ninguna ruta guardada más allá del plazo.
- **En contra:** los días antiguos de `file_events` ya no se pueden recalcular. Es aceptable porque esa tabla solo recibe datos en vivo.

## Validación
Tests obligatorios de §6.4.2 en F4a: doble ejecución, fallo inyectado entre los pasos 3 y 5, dato tardío, reasignación `NULL → 7`, cambio de `day`, que la purga no deje días `dirty` y los casos R-4. Test de purga de F4a: no queda ninguna ruta con más de 30 días.
