# ADR-0009: Push antes que polling

- **Estado:** Aceptada (2026-11-10), SPEC v1.2.2
- **Referencias:** SPEC §4 (principio 4), §5.3, §5.4, §11.3

## Contexto
El cockpit está siempre abierto, así que cada sondeo tiene un coste fijo de CPU y batería. La v1 tenía tres sondeos que se pueden sustituir:
- `$.session.usage()` cada 5 s;
- flush del mod cada 250 ms;
- lectura de `sessions/*.json` cada 2 s.

La API de mods ofrece `session.measure`, que se dispara al terminar cada turno principal y cada vez que un rate limit se mueve un punto. Es push.

## Decisión
- `session.measure` sustituye al sondeo de uso. Si G1 falla en algún motor, se llama a `$.session.usage()` en `turn.complete`, que también va por evento y no usa timer.
- Flush **bajo demanda** con `$.clock.after(50)`. Con la sesión inactiva y conectada, el mod tiene **0 timers**.
- `sessions/*.json` se vigila con `notify`, y además se comprueba el PID cada 30 s.
- **Sondeos que se conservan, con su justificación:**
  - `stat` cada 2 s, **solo** de los JSONL de sesiones activas, porque `ReadDirectoryChangesW` puede retrasar los avisos de append;
  - reescaneo cada 60 s o cuando llega un `Rescan`;
  - `stat` de `endpoint.json` cada 15 s, **solo** mientras el mod está desconectado (C1, ADR-0012).
- Git y el contexto a mitad de turno nunca se sondean. El contexto se obtiene del transcript, y no se hookea `turn.step` salvo que G2 demuestre que es barato.

## Alternativas consideradas
- **Mantener los sondeos de la v1:** coste fijo innecesario. Descartado.
- **Hookear `turn.step` por defecto:** coste por cada chunk de las respuestas del modelo. Descartado, pendiente de G2.

## Consecuencias
- **A favor:** CPU en reposo < 0,5 % (objetivo) y 0 timers en el mod mientras está conectado e inactivo.
- **En contra:** el contexto % del mod solo se actualiza al final de cada turno. Durante el turno se usa el `usage` del transcript.

## Validación
B2 (timers del mod en los dos estados) y B8 (CPU en reposo).
