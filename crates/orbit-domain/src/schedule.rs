//! Cron schedule evaluation shared by validation, the store and the scheduler.
//!
//! This is the single place that turns `(cron, timezone)` into the next UTC instant.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;

/// Why a schedule could not be evaluated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScheduleError {
    #[error("invalid cron expression (standard 5-field cron)")]
    InvalidCron,
    #[error("unknown timezone '{0}' (use an IANA name such as 'Europe/Amsterdam' or 'UTC')")]
    UnknownTimezone(String),
}

/// Parse an IANA timezone name. `None` or a blank value means UTC.
pub fn parse_timezone(timezone: Option<&str>) -> Result<Tz, ScheduleError> {
    match timezone.map(str::trim).filter(|tz| !tz.is_empty()) {
        None => Ok(Tz::UTC),
        Some(name) => name
            .parse::<Tz>()
            .map_err(|_| ScheduleError::UnknownTimezone(name.to_string())),
    }
}

/// Validate a cron expression and timezone without computing an occurrence.
pub fn validate_schedule(cron: &str, timezone: Option<&str>) -> Result<(), ScheduleError> {
    cron.parse::<croner::Cron>()
        .map_err(|_| ScheduleError::InvalidCron)?;
    parse_timezone(timezone)?;
    Ok(())
}

/// Next occurrence of `cron` strictly after `after`, evaluated in `timezone` (default UTC)
/// and returned as a UTC instant. `Ok(None)` means the expression never fires again.
pub fn next_occurrence(
    cron: &str,
    timezone: Option<&str>,
    after: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, ScheduleError> {
    let parsed = cron
        .parse::<croner::Cron>()
        .map_err(|_| ScheduleError::InvalidCron)?;
    let tz = parse_timezone(timezone)?;
    Ok(parsed
        .find_next_occurrence(&after.with_timezone(&tz), false)
        .ok()
        .map(|next| next.with_timezone(&Utc)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn amsterdam_nine_am_follows_dst() {
        let winter = next_occurrence("0 9 * * *", Some("Europe/Amsterdam"), utc(2026, 1, 15, 0));
        assert_eq!(winter, Ok(Some(utc(2026, 1, 15, 8))));
        let summer = next_occurrence("0 9 * * *", Some("Europe/Amsterdam"), utc(2026, 7, 15, 0));
        assert_eq!(summer, Ok(Some(utc(2026, 7, 15, 7))));
    }

    #[test]
    fn default_and_utc_are_equivalent() {
        let a = next_occurrence("0 9 * * *", None, utc(2026, 7, 15, 0));
        let b = next_occurrence("0 9 * * *", Some("UTC"), utc(2026, 7, 15, 0));
        assert_eq!(a, Ok(Some(utc(2026, 7, 15, 9))));
        assert_eq!(a, b);
    }

    #[test]
    fn rejects_unknown_timezone_and_bad_cron() {
        assert_eq!(
            validate_schedule("0 9 * * *", Some("Mars/Olympus")),
            Err(ScheduleError::UnknownTimezone("Mars/Olympus".into()))
        );
        assert_eq!(
            validate_schedule("not cron", None),
            Err(ScheduleError::InvalidCron)
        );
        assert!(validate_schedule("0 9 * * *", Some("Europe/Amsterdam")).is_ok());
    }
}
