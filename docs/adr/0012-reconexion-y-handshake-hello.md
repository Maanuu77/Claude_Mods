# ADR-0012: Reconexión automática del mod y handshake `POST /v1/hello`

- **Estado:** Aceptada (2026-11-10), SPEC v1.2.2
- **Referencias:** SPEC §5.3, §5.4, §6.2, §6.2.1, §11.2, §11.3, §13.2 (G10), C1

## Contexto
Claude Code puede llevar horas con una sesión abierta cuando se abre DEV COCKPIT. El usuario exige que el mod se conecte **sin reiniciar Claude**. La app rota el puerto y el token en cada arranque. Un mod en una sesión inactiva no puede recibir un push de la app sin usar canales internos.

## Decisión
- **El estado de conexión del mod** es una máquina con tres estados:
  - `connected`;
  - `disconnected`: el mod hace un `$.fs.stat(endpoint.json)` cada 15 s, solo en este estado, e intenta conectar de inmediato si llega un evento;
  - `resyncing`.
- **`POST /v1/hello`**, protegido con Bearer, sin `Origin`, con `Host` comprobado y con lista blanca de campos:
  - la petición lleva la identidad, el snapshot de uso, los agentes y las herramientas en curso, siempre bajo la política activa (`PRE_HANDSHAKE` la primera vez);
  - la respuesta lleva la política `privacy` autorizada, `ack_seq` y `engine_supported`.
- **Idempotencia:**
  - `hello` es un upsert por `session.id`, con clave de deduplicación `(instance_id, hello_seq)`;
  - una herramienta que no aparece en el snapshot **no se cierra**;
  - `/v1/events` usa la clave `(instance_id, seq)` con marca de agua.
- **Escenarios cubiertos:**
  - S1: la app se abre con Claude ya en marcha → conecta en ≤ 15 s, o ≤ 1 s tras el siguiente evento;
  - S2: la app se reinicia → un `401` o `ECONNREFUSED` provoca la relectura inmediata del endpoint;
  - S3 y S4: la app se cierra o muere → cola acotada y la sesión no se ve afectada.
- Instalar o actualizar el mod durante una sesión sigue necesitando `/reload-plugins`, porque es una instalación y no una reconexión.

## Alternativas consideradas
- **Push de la app al mod por la named pipe interna de `sessions/<pid>.json`:** es un formato interno y podría inyectar mensajes en la sesión. Descartado.
- **Puerto fijo sin descubrimiento:** impide rotar el token y choca con otros procesos. Descartado.
- **Si G10 falla** (el mod no puede leer `%APPDATA%`): el trabajo **se detiene** y se presentan las alternativas A–E de SPEC §13.2. **Ninguna se elige automáticamente.** Las alternativas que escriben en `~/.claude` (D y E) requieren autorización previa.

## Consecuencias
- **A favor:** no hace falta reiniciar Claude, y la reconexión y los reintentos no duplican datos.
- **En contra:** mientras el mod está desconectado hay un `stat` local cada 15 s, que es el único timer del mod en reposo.

## Validación
- G10 en G-pre, antes de F0, en los tres motores.
- B11: escenarios S1–S4, con 10 repeticiones cada uno.
- Criterio C1 de F3.
- E2E de SPEC §16, paso 4.
- **Cada sesión de prueba requiere antes la autorización explícita del usuario (W11).**
