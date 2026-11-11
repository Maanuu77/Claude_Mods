//! Ingesta de DEV COCKPIT (SPEC §6, §7).
//!
//! F1: `TranscriptTailer` (descubrimiento, offsets y backfill). `SessionRegistryWatch`,
//! `HttpIngest`, `WorkspaceWatcher` y `GitReader` llegan en F2, F3 y F4a.

pub mod tailer;

/// Rutas HTTP que expondrá `HttpIngest` (SPEC §6.2).
pub const HTTP_ROUTES: [&str; 3] = ["/v1/hello", "/v1/events", "/v1/health"];
