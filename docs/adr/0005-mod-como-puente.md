# ADR-0005: Mod de Claude Code como puente de telemetría con indicador mínimo

- **Estado:** Aceptada (2026-11-10), SPEC v1.2.2
- **Referencias:** SPEC §3, §5, §12, §13.2 (G1, G3, G4, G10)

## Contexto
Algunos datos en vivo solo existen dentro de la sesión:
- herramientas en curso, antes de que terminen;
- contexto %, rate limits de 5h/7d y coste «como `/cost`», que llegan por `session.measure`;
- estado de los subagentes, que da `$.agent.list()`.

La API de mods está documentada, pero es *early access* y puede cambiar entre releases. Los transcripts sirven como respaldo, pero su formato es interno.

## Decisión
Mod `dev-cockpit-bridge` de **observación pura**:
- ningún hook bloquea ni reescribe; `tool.call` emite `ToolStarted` antes de `await next(e)` y `ToolFinished` en `finally`;
- envía por `$.http.fetch` a `127.0.0.1` con flush bajo demanda;
- su interfaz se limita a `$.ui.status` y `/cockpit`.

Además:
- **La app funciona sin el mod** (modo degradado, solo transcripts) y **el mod funciona sin la app** (cola acotada, sin afectar a la sesión).
- Versión mínima provisional: 2.1.289. Se fija tras G1.
- **Durante el desarrollo, el mod se carga con `--plugin-dir`.** La instalación permanente modifica `~/.claude` y **requiere autorización previa** (W11).

## Alternativas consideradas
- **Hooks clásicos `command`/`http`:** arrancan un proceso por cada evento. Se quedan como plan B.
- **Status line:** solo hay un slot y pisaría el del usuario. Descartado.
- **El mod como interfaz principal:** limitado a las superficies de Claude y sin histórico multisesión. Descartado.
- **OpenTelemetry:** latencia de 5 a 60 s y no da rate limits. Diferido.

## Consecuencias
- **A favor:** latencia baja (objetivo p95 < 300 ms) y datos que no están en ninguna otra fuente.
- **En contra:** depende de una API inestable. Hay que validarlo contra los tres motores reales en cada release (SPEC §12).

## Validación
G1 y G10, en G-pre antes de F0. G3 y G4, B2, B3 y B11 en F3. Cada sesión de prueba requiere autorización explícita (W11).
