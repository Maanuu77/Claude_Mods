# ADR-0003: `tauri::ipc::Channel` para el stream vivo frente a eventos `emit`

- **Estado:** Aceptada (2026-11-10), SPEC v1.2.2
- **Referencias:** SPEC §4, §6.5, §7 (frontend)

## Contexto
La interfaz recibe deltas frecuentes: herramientas en curso, tareas, uso y cambios de fichero. Según la documentación de Tauri 2, el sistema de eventos «not designed for low latency or high throughput»: entrega JSON evaluando JavaScript y sus listeners asíncronos pueden desordenarse. Los Channels, en cambio, entregan los mensajes **ordenados** y son el mecanismo que Tauri recomienda para streaming.

## Decisión
- Un único `Channel<LiveDelta>`, abierto con `subscribe_live`.
- En Rust, los deltas se coalescen cada 100 ms y nunca se envía un evento por cada evento crudo.
- Las consultas históricas usan comandos `invoke` asíncronos con TanStack Query.

## Alternativas consideradas
- **`emit`/`listen`:** rendimiento y orden no garantizados. Descartado.
- **WebSocket o HTTP local hacia el WebView:** amplía la CSP y la superficie de ataque. Descartado.

## Consecuencias
- **A favor:** orden garantizado, poca sobrecarga y CSP estricta (`connect-src ipc: http://ipc.localhost`).
- **En contra:** `Channel<Vec<u8>>` se serializa como un array JSON de números. Si hacen falta binarios, se usa `InvokeResponseBody` o `Response`.

## Validación
B3 y B4 (latencias p95 < 300 ms y < 2,5 s).
