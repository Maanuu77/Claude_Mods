//! Almacenamiento de DEV COCKPIT: SQLite con `rusqlite`, migraciones, `DbWriter`
//! (hilo escritor único) y consultas (SPEC §6.4).
//!
//! F0: solo el andamiaje. La implementación empieza en F1.

/// Nombre del fichero de la base de datos dentro de `%APPDATA%\dev-cockpit\`.
pub const DB_FILE_NAME: &str = "cockpit.db";

/// Versión del protocolo con la que se ha compilado el almacenamiento.
pub const fn protocol_schema() -> u32 {
    cockpit_core::PROTOCOL_SCHEMA
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_against_core() {
        assert_eq!(protocol_schema(), 1);
    }
}
