//! Dominio de DEV COCKPIT: modelo canónico, parser de transcripts y
//! `PrivacyFilter` (SPEC §6, §10).
//!
//! Pendiente de otras fases: precios y coste estimado y detectores de manifiestos
//! (F4c). La agregación diaria vive en `cockpit-store`, junto a su SQL (F4a).

pub mod attribution;
pub mod model;
pub mod privacy;
pub mod project_map;
pub mod time;
pub mod transcript;
pub mod workspace;

/// Versión del esquema del protocolo mod ↔ app (`schema` en SPEC §6.2.1).
pub const PROTOCOL_SCHEMA: u32 = 1;

/// Motor de Claude Code más antiguo soportado (SPEC §12, confirmado en G-pre).
pub const MIN_ENGINE: &str = "2.1.289";

/// `retention.path_detail_days`: días de detalle de rutas (D3, SPEC §6.4.1).
pub const PATH_DETAIL_DAYS: u32 = 30;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_engine_is_a_three_part_version() {
        let parts: Vec<u32> = MIN_ENGINE.split('.').map(|p| p.parse().unwrap()).collect();
        assert_eq!(parts, [2, 1, 289]);
    }
}
