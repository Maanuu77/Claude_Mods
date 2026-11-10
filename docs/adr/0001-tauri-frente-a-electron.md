# ADR-0001: Tauri 2 + React (WebView2) frente a Electron

- **Estado:** Aceptada (2026-11-10), SPEC v1.2.2
- **Referencias:** SPEC §1, §4, §6, §11, §14

## Contexto
DEV COCKPIT es una app de escritorio de observabilidad para uso personal en Windows 11. Tiene que funcionar todo el día con poco consumo y ofrecer una interfaz rica (dashboards, timelines, grafos). Su núcleo de ingesta exige rendimiento: 471,7 MB de transcripts (M1), un pico de 107 herramientas por minuto (M2) y vigilancia de ficheros con decenas de miles de entradas en disco (M6).

## Decisión
- **Tauri 2.12.x**, con núcleo en Rust y frontend React 19 + Vite + TypeScript sobre **WebView2 Evergreen**.
- WebView2 en modo `downloadBootstrapper`, que añade 0 MB al instalador.
- Instalador NSIS sin firmar en v1.
- No se usa Tauri 3 (está en alfa).

## Alternativas consideradas
- **Electron:** el instalador pesa entre 80 y 200 MB y Node va dentro del proceso. En Windows no hay ventaja de motor, porque WebView2 también es Chromium. Descartado.
- **Sidecar de Node para la ingesta:** añade un runtime y decenas de MB. Rust con tokio cubre la ingesta. Descartado.

## Consecuencias
- **A favor:** instalador pequeño (objetivo < 15 MB, B10) y ingesta nativa en Rust. Modelo de seguridad basado en capabilities.
- **En contra:**
  - WebView2 es multiproceso y la RAM total no es mínima (objetivo < 250 MB, aislada según C2).
  - `backgroundThrottling` no está soportado en Windows, y eso obliga a ADR-0002.
  - Hace falta toolchain MSVC + Rust ≥ 1.90 (F0).

## Validación
B8, B9 y B10 (SPEC §11.4) y G9 (aislamiento de procesos WebView2).
