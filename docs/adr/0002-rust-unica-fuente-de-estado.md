# ADR-0002: Rust como única fuente de estado; el WebView es una vista desechable

- **Estado:** Aceptada (2026-11-10), SPEC v1.2.2
- **Referencias:** SPEC §4 (principio 1), §6, §6.5, §11.3

## Contexto
En Windows, la opción `backgroundThrottling` de Tauri no tiene efecto y los timers de JavaScript se ralentizan con la ventana oculta o minimizada. DEV COCKPIT sigue funcionando en la bandeja mientras ingiere datos. Además, WebView2 puede acumular memoria en páginas que llevan mucho tiempo abiertas.

## Decisión
- Toda la ingesta, los temporizadores, la agregación y el estado vivo (`LiveState`) residen en el proceso Rust.
- El WebView solo pinta.
- Con la ventana oculta no se publica nada. Al volver a mostrarla, se envía un `Snapshot` completo.
- Las series se guardan en ring buffers de tamaño fijo.

## Alternativas consideradas
- **Estado en el frontend (Zustand como fuente de verdad):** se ve afectado por el throttling y se pierde al recargar. Descartado.

## Consecuencias
- **A favor:**
  - La ingesta no depende de la visibilidad de la ventana.
  - Recargar el WebView no pierde datos.
  - La lógica se puede probar sin interfaz (crate `cockpit-core`).
- **En contra:** hace falta un protocolo de snapshot y deltas (ADR-0003).

## Validación
B8 y B9 (crecimiento de memoria < 10 % en 24 h). Prueba de ocultar y mostrar en F2.
