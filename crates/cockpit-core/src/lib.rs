//! Dominio de DEV COCKPIT: modelo canónico, parser de transcripts y
//! `PrivacyFilter` (SPEC §6, §10).
//!
//! Pendiente de otras fases: precios y coste estimado, agregación diaria (F4a) y
//! detectores de manifiestos (F4c).

pub mod model;
pub mod privacy;
pub mod time;
pub mod transcript;

/// Versión del esquema del protocolo mod ↔ app (`schema` en SPEC §6.2.1).
pub const PROTOCOL_SCHEMA: u32 = 1;

/// Motor de Claude Code más antiguo soportado (SPEC §12, confirmado en G-pre).
pub const MIN_ENGINE: &str = "2.1.289";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_engine_is_a_three_part_version() {
        let parts: Vec<u32> = MIN_ENGINE.split('.').map(|p| p.parse().unwrap()).collect();
        assert_eq!(parts, [2, 1, 289]);
    }
}
