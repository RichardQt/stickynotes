//! .NET ticks <-> Unix time conversion.
//!
//! 1 tick = 100 ns. .NET ticks count from 0001-01-01 00:00:00 UTC.
/// .NET ticks at the Unix epoch (1970-01-01 00:00:00 UTC).
pub const EPOCH_TICKS: i64 = 621_355_968_000_000_000;

/// Convert .NET ticks to seconds + subsecond nanos since the Unix epoch.
pub fn ticks_to_unix(ticks: i64) -> (i64, u32) {
    let unix_ticks = ticks - EPOCH_TICKS;
    let secs = unix_ticks.div_euclid(10_000_000);
    let nanos = unix_ticks.rem_euclid(10_000_000) as u32 * 100;
    (secs, nanos)
}

/// Convert Unix seconds + nanos to .NET ticks.
pub fn unix_to_ticks(secs: i64, nanos: u32) -> i64 {
    EPOCH_TICKS + secs * 10_000_000 + (nanos / 100) as i64
}

/// Current time as .NET ticks (UTC).
pub fn now_ticks() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time before epoch");
    unix_to_ticks(now.as_secs() as i64, now.subsec_nanos())
}

/// Format ticks for UI display (local time, e.g. "2026-11-16 17:32").
pub fn format_ticks(ticks: i64) -> String {
    let (secs, _) = ticks_to_unix(ticks);
    match time::OffsetDateTime::from_unix_timestamp(secs) {
        Ok(dt) => {
            // Local-time conversion via the local-offset feature of the time crate.
            let offset = time::UtcOffset::local_offset_at(dt).unwrap_or(time::UtcOffset::UTC);
            let local = dt.to_offset(offset);
            let fmt = time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]");
            local.format(&fmt).unwrap_or_default()
        }
        Err(_) => String::new(),
    }
}

/// Format ticks for cards / compact UI display (e.g. "17:32", "昨天 17:32", "11月16日", or "2025/3/6").
pub fn format_friendly_ticks(ticks: i64) -> String {
    let (secs, _) = ticks_to_unix(ticks);
    let Ok(dt) = time::OffsetDateTime::from_unix_timestamp(secs) else {
        return String::new();
    };
    let offset = time::UtcOffset::local_offset_at(dt).unwrap_or(time::UtcOffset::UTC);
    let local = dt.to_offset(offset);
    let now = time::OffsetDateTime::now_utc().to_offset(offset);

    if local.date() == now.date() {
        let fmt = time::macros::format_description!("[hour]:[minute]");
        local.format(&fmt).unwrap_or_default()
    } else if let Some(prev) = now.date().previous_day() {
        if local.date() == prev {
            let fmt = time::macros::format_description!("昨天 [hour]:[minute]");
            local.format(&fmt).unwrap_or_default()
        } else if local.year() == now.year() {
            format!("{}月{}日", local.month() as u8, local.day())
        } else {
            format!("{}/{}/{}", local.year(), local.month() as u8, local.day())
        }
    } else if local.year() == now.year() {
        format!("{}月{}日", local.month() as u8, local.day())
    } else {
        format!("{}/{}/{}", local.year(), local.month() as u8, local.day())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let t = 638_768_289_310_000_000i64;
        let (secs, nanos) = ticks_to_unix(t);
        assert_eq!(unix_to_ticks(secs, nanos), t);
    }

    #[test]
    fn sample_date() {
        // 638768289310000000 ticks = 2025-03-06 (verified against a real plum row)
        let (secs, _) = ticks_to_unix(638_768_289_310_000_000);
        let dt = time::OffsetDateTime::from_unix_timestamp(secs).unwrap();
        assert_eq!(dt.year(), 2025);
        assert_eq!(dt.month(), time::Month::March);
        assert_eq!(dt.day(), 6);
    }

    #[test]
    fn epoch() {
        assert_eq!(ticks_to_unix(EPOCH_TICKS), (0, 0));
        assert_eq!(unix_to_ticks(0, 0), EPOCH_TICKS);
    }
}
