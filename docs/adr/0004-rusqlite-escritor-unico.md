# ADR-0004: rusqlite (bundled) con escritor único en WAL frente a sqlx o `tauri-plugin-sql`

- **Estado:** Aceptada (2026-11-10), SPEC v1.2.2
- **Referencias:** SPEC §6, §6.4, §6.4.2

## Contexto
La base local guarda métricas y metadatos. La escritura es continua pero de volumen moderado: unos pocos eventos por segundo según M2, con ráfagas en el backfill. La lectura es concurrente desde la interfaz. SQLite en WAL permite lectores concurrentes con **un único escritor**. `tauri-plugin-sql` (sobre sqlx) expone SQL al WebView y no activa WAL por defecto.

## Decisión
- `rusqlite` 0.41 con la feature `bundled`.
- `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`.
- **Un solo hilo escritor**, que agrupa en transacciones cada 250 ms o cada 500 eventos.
- Un pool de lectura de 2 conexiones mediante `spawn_blocking`.
- El WebView **nunca** accede a SQL: solo usa comandos tipados.

## Alternativas consideradas
- **`tauri-plugin-sql`:** SQL accesible desde el frontend y permisos más amplios. Descartado.
- **sqlx asíncrono:** sus ventajas no compensan con un solo escritor. Habría que configurar WAL a mano. Descartado.

## Consecuencias
- **A favor:**
  - Sin contención entre escritores.
  - Transacciones atómicas, en las que se apoya la agregación idempotente (ADR-0011).
  - Superficie mínima.
- **En contra:** hay que gestionar los checkpoints del WAL. Por eso se hace `wal_checkpoint(TRUNCATE)` al final de cada compactación.

## Validación
Tests de §6.4.2 (doble ejecución, fallo inyectado, purga) y consultas < 100 ms en F4c.
