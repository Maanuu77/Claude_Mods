//! Dominio de DEV COCKPIT: modelo canónico, parsers de transcripts, normalizador,
//! `PrivacyFilter`, precios, agregación y detectores de manifiestos (SPEC §6).
//!
//! F0: solo el andamiaje. La implementación empieza en F1.

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
