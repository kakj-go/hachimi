use std::collections::BTreeSet;

use chrono::{LocalResult, NaiveDateTime, TimeZone as ChronoTimeZone, Utc};
use chrono_tz::Tz;
use hachimi_protocol::{SchedulePreview, ScheduleSpec};
use thiserror::Error;

const MIN_EVERY_INTERVAL_MS: u64 = 60_000;
#[cfg(test)]
static RELEASE_SOAK_SHORT_INTERVALS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn minimum_every_interval_ms() -> u64 {
    #[cfg(test)]
    if RELEASE_SOAK_SHORT_INTERVALS.load(std::sync::atomic::Ordering::SeqCst) {
        return 250;
    }
    MIN_EVERY_INTERVAL_MS
}

#[cfg(test)]
pub(crate) struct ReleaseSoakIntervalGuard;

#[cfg(test)]
impl Drop for ReleaseSoakIntervalGuard {
    fn drop(&mut self) {
        RELEASE_SOAK_SHORT_INTERVALS.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
pub(crate) fn enable_release_soak_short_intervals() -> ReleaseSoakIntervalGuard {
    RELEASE_SOAK_SHORT_INTERVALS.store(true, std::sync::atomic::Ordering::SeqCst);
    ReleaseSoakIntervalGuard
}
use time::{Duration, PrimitiveDateTime, Time, Weekday};

const MAX_CRON_SEARCH_STEPS: usize = 6 * 366 * 24 * 60 * 60;

#[derive(Debug, Error)]
pub enum CalendarError {
    #[error("schedule timestamp must be in the future")]
    PastTimestamp,
    #[error("schedule interval must be at least 60000 milliseconds")]
    IntervalTooShort,
    #[error("invalid Cron expression: {0}")]
    InvalidCron(String),
    #[error("invalid IANA timezone: {0}")]
    InvalidTimeZone(String),
    #[error("timezone runtime is unavailable: {0}")]
    TimeZoneUnavailable(String),
    #[error("no Cron occurrence was found within the search horizon")]
    SearchHorizonExceeded,
    #[error("timestamp is outside the supported range")]
    TimestampRange,
}

pub trait TimeZoneResolver: Send + Sync {
    fn utc_to_local(
        &self,
        timezone: &str,
        unix_ms: i64,
    ) -> Result<PrimitiveDateTime, CalendarError>;

    fn local_to_utc(
        &self,
        timezone: &str,
        local: PrimitiveDateTime,
    ) -> Result<Vec<i64>, CalendarError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BundledIanaTimeZoneResolver;

impl TimeZoneResolver for BundledIanaTimeZoneResolver {
    fn utc_to_local(
        &self,
        timezone: &str,
        unix_ms: i64,
    ) -> Result<PrimitiveDateTime, CalendarError> {
        let timezone = parse_timezone(timezone)?;
        let utc = chrono::DateTime::<Utc>::from_timestamp_millis(unix_ms)
            .ok_or(CalendarError::TimestampRange)?;
        parse_local(
            &utc.with_timezone(&timezone)
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string(),
        )
    }

    fn local_to_utc(
        &self,
        timezone: &str,
        local: PrimitiveDateTime,
    ) -> Result<Vec<i64>, CalendarError> {
        let timezone = parse_timezone(timezone)?;
        let local = NaiveDateTime::parse_from_str(&format_local(local), "%Y-%m-%dT%H:%M:%S")
            .map_err(|_| CalendarError::TimestampRange)?;
        let mut values = match timezone.from_local_datetime(&local) {
            LocalResult::None => Vec::new(),
            LocalResult::Single(value) => vec![value.timestamp_millis()],
            LocalResult::Ambiguous(first, second) => {
                vec![first.timestamp_millis(), second.timestamp_millis()]
            }
        };
        values.sort_unstable();
        values.dedup();
        Ok(values)
    }
}

fn parse_timezone(timezone: &str) -> Result<Tz, CalendarError> {
    validate_timezone_name(timezone)?;
    timezone
        .parse::<Tz>()
        .map_err(|_| CalendarError::InvalidTimeZone(timezone.into()))
}

pub fn preview_schedule(
    resolver: &dyn TimeZoneResolver,
    schedule: &ScheduleSpec,
    after_ms: i64,
    count: usize,
) -> SchedulePreview {
    match occurrences_after(resolver, schedule, after_ms, count.clamp(1, 20)) {
        Ok(next_occurrences_ms) => SchedulePreview {
            valid: true,
            error_code: None,
            next_occurrences_ms,
        },
        Err(error) => SchedulePreview {
            valid: false,
            error_code: Some(error_code(&error).into()),
            next_occurrences_ms: Vec::new(),
        },
    }
}

pub fn occurrences_after(
    resolver: &dyn TimeZoneResolver,
    schedule: &ScheduleSpec,
    after_ms: i64,
    count: usize,
) -> Result<Vec<i64>, CalendarError> {
    match schedule {
        ScheduleSpec::At { timestamp_ms } => {
            if *timestamp_ms <= after_ms {
                Ok(Vec::new())
            } else {
                Ok(vec![*timestamp_ms])
            }
        }
        ScheduleSpec::Every {
            interval_ms,
            anchor_ms,
        } => {
            if *interval_ms < minimum_every_interval_ms() {
                return Err(CalendarError::IntervalTooShort);
            }
            let interval =
                i64::try_from(*interval_ms).map_err(|_| CalendarError::TimestampRange)?;
            let elapsed = after_ms.saturating_sub(*anchor_ms);
            let steps = if elapsed < 0 {
                0
            } else {
                elapsed.div_euclid(interval).saturating_add(1)
            };
            let mut next = anchor_ms.saturating_add(steps.saturating_mul(interval));
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                values.push(next);
                next = next
                    .checked_add(interval)
                    .ok_or(CalendarError::TimestampRange)?;
            }
            Ok(values)
        }
        ScheduleSpec::Cron {
            expression,
            timezone,
        } => cron_occurrences(resolver, expression, timezone, after_ms, count),
        ScheduleSpec::Event { .. } => Ok(Vec::new()),
    }
}

#[must_use]
pub fn error_code(error: &CalendarError) -> &'static str {
    match error {
        CalendarError::PastTimestamp => "schedule_time_in_past",
        CalendarError::IntervalTooShort => "schedule_interval_too_short",
        CalendarError::InvalidCron(_) => "schedule_cron_invalid",
        CalendarError::InvalidTimeZone(_) => "schedule_timezone_invalid",
        CalendarError::TimeZoneUnavailable(_) => "schedule_timezone_runtime_unavailable",
        CalendarError::SearchHorizonExceeded => "schedule_search_horizon_exceeded",
        CalendarError::TimestampRange => "schedule_timestamp_out_of_range",
    }
}

fn cron_occurrences(
    resolver: &dyn TimeZoneResolver,
    expression: &str,
    timezone: &str,
    after_ms: i64,
    count: usize,
) -> Result<Vec<i64>, CalendarError> {
    validate_timezone_name(timezone)?;
    let cron = CronExpression::parse(expression)?;
    let mut local = resolver.utc_to_local(timezone, after_ms)?;
    local = local
        .replace_nanosecond(0)
        .map_err(|_| CalendarError::TimestampRange)?
        .checked_add(if cron.has_seconds {
            Duration::SECOND
        } else {
            Duration::MINUTE
        })
        .ok_or(CalendarError::TimestampRange)?;
    if !cron.has_seconds {
        local = PrimitiveDateTime::new(
            local.date(),
            Time::from_hms(local.hour(), local.minute(), 0)
                .map_err(|_| CalendarError::TimestampRange)?,
        );
    }
    let step = if cron.has_seconds {
        Duration::SECOND
    } else {
        Duration::MINUTE
    };
    let mut values = Vec::with_capacity(count);
    for _ in 0..MAX_CRON_SEARCH_STEPS {
        if cron.matches(local) {
            let mut candidates = resolver.local_to_utc(timezone, local)?;
            candidates.sort_unstable();
            for candidate in candidates {
                if candidate > after_ms
                    && values.last().is_none_or(|previous| *previous != candidate)
                {
                    values.push(candidate);
                    if values.len() == count {
                        return Ok(values);
                    }
                }
            }
        }
        local = local
            .checked_add(step)
            .ok_or(CalendarError::TimestampRange)?;
    }
    Err(CalendarError::SearchHorizonExceeded)
}

#[derive(Debug, Clone)]
struct CronExpression {
    seconds: BTreeSet<u8>,
    minutes: BTreeSet<u8>,
    hours: BTreeSet<u8>,
    days: BTreeSet<u8>,
    months: BTreeSet<u8>,
    weekdays: BTreeSet<u8>,
    day_wildcard: bool,
    weekday_wildcard: bool,
    has_seconds: bool,
}

impl CronExpression {
    fn parse(expression: &str) -> Result<Self, CalendarError> {
        let fields = expression.split_whitespace().collect::<Vec<_>>();
        let (has_seconds, fields) = match fields.len() {
            5 => (false, fields),
            6 => (true, fields),
            _ => {
                return Err(CalendarError::InvalidCron("expected 5 or 6 fields".into()));
            }
        };
        let offset = usize::from(!has_seconds);
        let seconds = if has_seconds {
            parse_field(fields[0], 0, 59, None)?
        } else {
            BTreeSet::from([0])
        };
        let minute_index = 1_usize.saturating_sub(offset);
        let day_index = 3_usize.saturating_sub(offset);
        let weekday_index = 5_usize.saturating_sub(offset);
        Ok(Self {
            seconds,
            minutes: parse_field(fields[minute_index], 0, 59, None)?,
            hours: parse_field(fields[minute_index + 1], 0, 23, None)?,
            days: parse_field(fields[day_index], 1, 31, None)?,
            months: parse_field(fields[day_index + 1], 1, 12, Some(&MONTH_NAMES))?,
            weekdays: parse_field(fields[weekday_index], 0, 7, Some(&WEEKDAY_NAMES))?
                .into_iter()
                .map(|value| if value == 7 { 0 } else { value })
                .collect(),
            day_wildcard: fields[day_index] == "*",
            weekday_wildcard: fields[weekday_index] == "*",
            has_seconds,
        })
    }

    fn matches(&self, local: PrimitiveDateTime) -> bool {
        let weekday = match local.weekday() {
            Weekday::Sunday => 0,
            Weekday::Monday => 1,
            Weekday::Tuesday => 2,
            Weekday::Wednesday => 3,
            Weekday::Thursday => 4,
            Weekday::Friday => 5,
            Weekday::Saturday => 6,
        };
        let day_match = self.days.contains(&local.day());
        let weekday_match = self.weekdays.contains(&weekday);
        let calendar_day_match = match (self.day_wildcard, self.weekday_wildcard) {
            (true, true) => true,
            (true, false) => weekday_match,
            (false, true) => day_match,
            (false, false) => day_match || weekday_match,
        };
        self.seconds.contains(&local.second())
            && self.minutes.contains(&local.minute())
            && self.hours.contains(&local.hour())
            && self.months.contains(&(local.month() as u8))
            && calendar_day_match
    }
}

const MONTH_NAMES: [(&str, u8); 12] = [
    ("JAN", 1),
    ("FEB", 2),
    ("MAR", 3),
    ("APR", 4),
    ("MAY", 5),
    ("JUN", 6),
    ("JUL", 7),
    ("AUG", 8),
    ("SEP", 9),
    ("OCT", 10),
    ("NOV", 11),
    ("DEC", 12),
];
const WEEKDAY_NAMES: [(&str, u8); 7] = [
    ("SUN", 0),
    ("MON", 1),
    ("TUE", 2),
    ("WED", 3),
    ("THU", 4),
    ("FRI", 5),
    ("SAT", 6),
];

fn parse_field(
    field: &str,
    min: u8,
    max: u8,
    names: Option<&[(&str, u8)]>,
) -> Result<BTreeSet<u8>, CalendarError> {
    let mut values = BTreeSet::new();
    for segment in field.split(',') {
        let (base, step) = segment
            .split_once('/')
            .map_or((segment, 1), |(base, step)| {
                (base, step.parse::<u8>().unwrap_or(0))
            });
        if step == 0 {
            return Err(CalendarError::InvalidCron("step must be positive".into()));
        }
        let (start, end) = if base == "*" {
            (min, max)
        } else if let Some((start, end)) = base.split_once('-') {
            (
                parse_value(start, min, max, names)?,
                parse_value(end, min, max, names)?,
            )
        } else {
            let value = parse_value(base, min, max, names)?;
            (value, value)
        };
        if start > end {
            return Err(CalendarError::InvalidCron("range start exceeds end".into()));
        }
        let mut value = start;
        while value <= end {
            values.insert(value);
            let Some(next) = value.checked_add(step) else {
                break;
            };
            value = next;
        }
    }
    if values.is_empty() {
        return Err(CalendarError::InvalidCron("field is empty".into()));
    }
    Ok(values)
}

fn parse_value(
    value: &str,
    min: u8,
    max: u8,
    names: Option<&[(&str, u8)]>,
) -> Result<u8, CalendarError> {
    let upper = value.to_ascii_uppercase();
    let parsed = names
        .and_then(|names| {
            names
                .iter()
                .find_map(|(name, number)| (*name == upper).then_some(*number))
        })
        .or_else(|| upper.parse::<u8>().ok())
        .ok_or_else(|| CalendarError::InvalidCron(format!("invalid value {value}")))?;
    if !(min..=max).contains(&parsed) {
        return Err(CalendarError::InvalidCron(format!(
            "value {parsed} is outside {min}-{max}"
        )));
    }
    Ok(parsed)
}

fn validate_timezone_name(timezone: &str) -> Result<(), CalendarError> {
    if timezone.is_empty()
        || timezone.len() > 128
        || !timezone
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'+'))
    {
        return Err(CalendarError::InvalidTimeZone(timezone.into()));
    }
    Ok(())
}

fn format_local(local: PrimitiveDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        local.year(),
        local.month() as u8,
        local.day(),
        local.hour(),
        local.minute(),
        local.second()
    )
}

fn parse_local(value: &str) -> Result<PrimitiveDateTime, CalendarError> {
    if value.len() != 19 {
        return Err(CalendarError::TimeZoneUnavailable(
            "timezone runtime returned an invalid local timestamp".into(),
        ));
    }
    let parse = |range: std::ops::Range<usize>| {
        value[range]
            .parse::<u8>()
            .map_err(|error| CalendarError::TimeZoneUnavailable(error.to_string()))
    };
    let year = value[0..4]
        .parse::<i32>()
        .map_err(|error| CalendarError::TimeZoneUnavailable(error.to_string()))?;
    let month = time::Month::try_from(parse(5..7)?)
        .map_err(|error| CalendarError::TimeZoneUnavailable(error.to_string()))?;
    let date = time::Date::from_calendar_date(year, month, parse(8..10)?)
        .map_err(|error| CalendarError::TimeZoneUnavailable(error.to_string()))?;
    let time = Time::from_hms(parse(11..13)?, parse(14..16)?, parse(17..19)?)
        .map_err(|error| CalendarError::TimeZoneUnavailable(error.to_string()))?;
    Ok(PrimitiveDateTime::new(date, time))
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::{Date, Month};

    #[test]
    fn every_uses_a_fixed_anchor_without_drift() {
        let values = occurrences_after(
            &BundledIanaTimeZoneResolver,
            &ScheduleSpec::Every {
                interval_ms: 86_400_000,
                anchor_ms: 1_000,
            },
            86_401_500,
            2,
        )
        .expect("occurrences");
        assert_eq!(values, vec![172_801_000, 259_201_000]);
    }

    #[test]
    fn five_field_cron_defaults_seconds_to_zero() {
        let cron = CronExpression::parse("30 9 * * MON-FRI").expect("Cron");
        let local = PrimitiveDateTime::new(
            Date::from_calendar_date(2026, Month::July, 27).expect("date"),
            Time::from_hms(9, 30, 0).expect("time"),
        );
        assert!(cron.matches(local));
        assert!(!cron.matches(local + Duration::SECOND));
    }

    #[test]
    fn iana_dst_gap_and_fold_are_fail_closed_and_deterministic() {
        let resolver = BundledIanaTimeZoneResolver;
        let gap = PrimitiveDateTime::new(
            Date::from_calendar_date(2026, Month::March, 8).expect("date"),
            Time::from_hms(2, 30, 0).expect("time"),
        );
        assert!(
            resolver
                .local_to_utc("America/New_York", gap)
                .expect("gap")
                .is_empty()
        );
        let fold = PrimitiveDateTime::new(
            Date::from_calendar_date(2026, Month::November, 1).expect("date"),
            Time::from_hms(1, 30, 0).expect("time"),
        );
        assert_eq!(
            resolver
                .local_to_utc("America/New_York", fold)
                .expect("fold")
                .len(),
            2
        );
    }
}
