//! Conversión de marcas de tiempo.

use chrono::{DateTime, Local, TimeZone};

/// `2026-11-10T12:34:56.789Z` → milisegundos desde la época Unix.
pub fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Fecha civil **local** `YYYY-MM-DD` de un instante.
///
/// Se calcula una sola vez, al insertar, para que la agregación diaria sea
/// determinista aunque después cambie la zona horaria (SPEC §6.4).
pub fn local_day(ts_ms: i64) -> String {
    day_in(&Local, ts_ms)
}

fn day_in<Tz: TimeZone>(tz: &Tz, ts_ms: i64) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match tz.timestamp_millis_opt(ts_ms).single() {
        Some(dt) => dt.format("%Y-%m-%d").to_string(),
        None => "1970-01-01".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn parses_transcript_timestamps() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:01.500Z"), Some(1500));
        assert_eq!(parse_rfc3339_ms("no es una fecha"), None);
    }

    #[test]
    fn day_uses_the_given_offset() {
        // 2026-11-10T23:30:00Z ya es día 11 en UTC+1.
        let ts = parse_rfc3339_ms("2026-11-10T23:30:00.000Z").unwrap();
        let madrid = FixedOffset::east_opt(3600).unwrap();
        assert_eq!(day_in(&madrid, ts), "2026-11-11");
        assert_eq!(day_in(&FixedOffset::east_opt(0).unwrap(), ts), "2026-11-10");
    }
}
