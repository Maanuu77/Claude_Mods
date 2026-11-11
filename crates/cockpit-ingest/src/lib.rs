//! Ingesta de DEV COCKPIT: `TranscriptTailer`, `SessionRegistryWatch`,
//! `WorkspaceWatcher`, `GitReader` y `HttpIngest` (SPEC §6, §7).
//!
//! F0: solo el andamiaje. La implementación empieza en F1.

/// Rutas HTTP que expone `HttpIngest` (SPEC §6.2).
pub const HTTP_ROUTES: [&str; 3] = ["/v1/hello", "/v1/events", "/v1/health"];

/// Fichero de la base de datos sobre el que escribe la ingesta.
pub const fn db_file_name() -> &'static str {
    cockpit_store::DB_FILE_NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_against_store_and_core() {
        assert_eq!(db_file_name(), "cockpit.db");
        assert_eq!(cockpit_core::PROTOCOL_SCHEMA, 1);
        assert_eq!(HTTP_ROUTES.len(), 3);
    }
}
