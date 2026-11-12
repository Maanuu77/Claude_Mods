//! Ingesta de DEV COCKPIT (SPEC §6, §7).
//!
//! - F1: `TranscriptTailer` (descubrimiento, offsets y backfill).
//! - F2: `SessionRegistryWatch` y la canalización en vivo (`live`).
//! - F3: `HttpIngest` y el contrato del mod (`wire`).
//! - F4a: `WorkspaceWatcher` (`workspace`, `watcher`) y `GitReader` (`git`).

pub mod git;
pub mod http;
pub mod live;
pub mod registry;
pub mod tailer;
pub mod watcher;
pub mod wire;
pub mod workspace;

/// Rutas HTTP que expondrá `HttpIngest` (SPEC §6.2).
pub const HTTP_ROUTES: [&str; 3] = ["/v1/hello", "/v1/events", "/v1/health"];
