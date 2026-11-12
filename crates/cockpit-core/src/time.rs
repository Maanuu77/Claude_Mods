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

/// Horas desde la medianoche local que se esperan antes de agregar un día (§6.4.2).
pub const AGGREGATION_GRACE_HOURS: u32 = 2;

/// Primer día **no** cerrado: se agregan los días `< closed_before(now)`.
///
/// Un día queda cerrado cuando ya es el siguiente y han pasado 2 h desde la
/// medianoche local (margen para eventos tardíos y la ventana de §7.3).
pub fn closed_before(now_ms: i64) -> String {
    closed_before_in(&Local, now_ms)
}

/// Fecha civil local `days` días antes de la de `now_ms`.
pub fn local_day_minus(now_ms: i64, days: u32) -> String {
    day_minus_in(&Local, now_ms, days)
}

fn closed_before_in<Tz: TimeZone>(tz: &Tz, now_ms: i64) -> String
where
    Tz::Offset: std::fmt::Display,
{
    use chrono::Timelike;
    let Some(now) = tz.timestamp_millis_opt(now_ms).single() else {
        return "1970-01-01".to_owned();
    };
    let back = u32::from(now.hour() < AGGREGATION_GRACE_HOURS);
    day_minus_in(tz, now_ms, back)
}

fn day_minus_in<Tz: TimeZone>(tz: &Tz, now_ms: i64, days: u32) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match tz.timestamp_millis_opt(now_ms).single() {
        Some(dt) => (dt.date_naive() - chrono::Days::new(u64::from(days)))
            .format("%Y-%m-%d")
            .to_string(),
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

    #[test]
    fn a_day_closes_two_hours_after_local_midnight() {
        let madrid = FixedOffset::east_opt(3600).unwrap();
        // 00:30 locales del día 11: el 10 aún no está cerrado.
        let early = parse_rfc3339_ms("2026-11-10T23:30:00.000Z").unwrap();
        assert_eq!(closed_before_in(&madrid, early), "2026-11-10");
        // 02:00 locales del día 11: ya se puede agregar el 10.
        let later = parse_rfc3339_ms("2026-11-11T01:00:00.000Z").unwrap();
        assert_eq!(closed_before_in(&madrid, later), "2026-11-11");
        assert_eq!(day_minus_in(&madrid, later, 30), "2026-10-12");
    }
}
