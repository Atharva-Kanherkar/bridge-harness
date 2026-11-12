//! When a workflow fires: a 5-field cron expression read in an IANA timezone.
//!
//! Five fields because Claude Code's native schedules use the same form, so a
//! later hand-off to its scheduler is a copy rather than a translation. The
//! timezone is explicit because "9 in the morning" means a wall clock, and a
//! laptop that travels must not quietly move the brief to the middle of the
//! night.
//!
//! Daylight saving is decided here, once, so the trigger engine never has to:
//! a local time that does not exist (spring forward) fires once at the first
//! valid instant after the gap, and a local time that occurs twice (fall back)
//! fires once, at its first occurrence. One fire per intended occasion, never
//! zero and never two.

use std::collections::BTreeSet;
use std::str::FromStr;

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

/// The shortest allowed gap between two consecutive fire times. The same floor
/// Work briefings and the learning schedule use: a background model run every
/// minute is a subscription drain, not a cadence.
pub const MIN_GAP_MINUTES: u32 = 15;

/// How far ahead `next_after` looks before deciding an expression never fires.
/// Four years and a day covers every leap-day schedule.
const SEARCH_DAYS: i64 = 4 * 366 + 1;

/// The longest daylight-saving gap any zone has, with room to spare.
const MAX_GAP_MINUTES: i64 = 180;

#[derive(Debug, Clone, PartialEq, Eq)]
struct CronSpec {
    minutes: BTreeSet<u32>,
    hours: BTreeSet<u32>,
    days_of_month: BTreeSet<u32>,
    months: BTreeSet<u32>,
    days_of_week: BTreeSet<u32>,
    dom_restricted: bool,
    dow_restricted: bool,
}

/// A parsed, validated schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schedule {
    spec: CronSpec,
    timezone: Tz,
}

impl Schedule {
    /// Parse and validate. Every refusal names what was wrong.
    pub fn parse(cron: &str, timezone: &str) -> Result<Self, String> {
        let timezone = Tz::from_str(timezone.trim())
            .map_err(|_| format!("`{timezone}` is not an IANA timezone, such as Europe/London"))?;
        let spec = parse_cron(cron)?;
        let schedule = Self { spec, timezone };
        let gap = schedule.min_gap_minutes();
        if gap < MIN_GAP_MINUTES {
            return Err(format!(
                "`{}` fires {gap} minutes apart; workflows run at most every {MIN_GAP_MINUTES} minutes",
                cron.trim()
            ));
        }
        if schedule.next_after(Utc::now()).is_none() {
            return Err(format!("`{}` never fires", cron.trim()));
        }
        Ok(schedule)
    }

    pub fn timezone(&self) -> Tz {
        self.timezone
    }

    /// The smallest gap between two consecutive fire times on the wall clock.
    ///
    /// Computed from the times of day alone. Days that do not match only make
    /// gaps longer, so this is a lower bound and never lets a too-frequent
    /// schedule through.
    pub fn min_gap_minutes(&self) -> u32 {
        let times: Vec<u32> = self
            .spec
            .hours
            .iter()
            .flat_map(|hour| self.spec.minutes.iter().map(move |minute| hour * 60 + minute))
            .collect();
        let mut gap = match (times.first(), times.last()) {
            (Some(first), Some(last)) => first + 24 * 60 - last,
            _ => return 0,
        };
        for pair in times.windows(2) {
            gap = gap.min(pair[1] - pair[0]);
        }
        gap
    }

    /// The first fire time strictly after `after`, or `None` when the
    /// expression cannot fire in the next four years.
    pub fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let start = after.with_timezone(&self.timezone).date_naive();
        for offset in 0..SEARCH_DAYS {
            let day = start + Duration::days(offset);
            if !self.day_matches(day) {
                continue;
            }
            for hour in &self.spec.hours {
                for minute in &self.spec.minutes {
                    let Some(naive) = day.and_hms_opt(*hour, *minute, 0) else { continue };
                    let Some(instant) = self.resolve(naive) else { continue };
                    if instant > after {
                        return Some(instant);
                    }
                }
            }
        }
        None
    }

    /// Every fire time in `(after, until]`, oldest first. Bounded by `limit`
    /// so a caller asking about a long sleep cannot allocate without end.
    pub fn fires_between(&self, after: DateTime<Utc>, until: DateTime<Utc>, limit: usize) -> Vec<DateTime<Utc>> {
        let mut fires = Vec::new();
        let mut cursor = after;
        while fires.len() < limit {
            match self.next_after(cursor) {
                Some(next) if next <= until => {
                    fires.push(next);
                    cursor = next;
                }
                _ => break,
            }
        }
        fires
    }

    fn day_matches(&self, day: NaiveDate) -> bool {
        if !self.spec.months.contains(&day.month()) {
            return false;
        }
        let dom = self.spec.days_of_month.contains(&day.day());
        let dow = self.spec.days_of_week.contains(&day.weekday().num_days_from_sunday());
        // Vixie cron: when both day fields are restricted, either may match.
        match (self.spec.dom_restricted, self.spec.dow_restricted) {
            (true, true) => dom || dow,
            (true, false) => dom,
            (false, true) => dow,
            (false, false) => true,
        }
    }

    /// The instant a wall-clock time means here, with the DST rules above.
    fn resolve(&self, naive: NaiveDateTime) -> Option<DateTime<Utc>> {
        match self.timezone.from_local_datetime(&naive) {
            LocalResult::Single(instant) => Some(instant.with_timezone(&Utc)),
            // Fall back: the first occurrence only.
            LocalResult::Ambiguous(first, _) => Some(first.with_timezone(&Utc)),
            // Spring forward: the first valid instant after the gap.
            LocalResult::None => (1..=MAX_GAP_MINUTES).find_map(|minutes| {
                let later = naive + Duration::minutes(minutes);
                match self.timezone.from_local_datetime(&later) {
                    // Gaps start and end on whole minutes, so the first
                    // minute that exists is the gap's end.
                    LocalResult::Single(instant) => Some(instant.with_timezone(&Utc)),
                    LocalResult::Ambiguous(first, _) => Some(first.with_timezone(&Utc)),
                    LocalResult::None => None,
                }
            }),
        }
    }
}

fn parse_cron(expression: &str) -> Result<CronSpec, String> {
    let fields: Vec<&str> = expression.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "`{}` has {} fields; a schedule needs five: minute hour day-of-month month day-of-week",
            expression.trim(),
            fields.len()
        ));
    }
    let minutes = parse_field(fields[0], 0, 59, "minute")?;
    let hours = parse_field(fields[1], 0, 23, "hour")?;
    let days_of_month = parse_field(fields[2], 1, 31, "day of month")?;
    let months = parse_field(fields[3], 1, 12, "month")?;
    // 7 is Sunday too.
    let days_of_week: BTreeSet<u32> = parse_field(fields[4], 0, 7, "day of week")?
        .into_iter()
        .map(|day| if day == 7 { 0 } else { day })
        .collect();
    Ok(CronSpec {
        minutes,
        hours,
        days_of_month,
        months,
        days_of_week,
        dom_restricted: fields[2] != "*",
        dow_restricted: fields[4] != "*",
    })
}

fn parse_field(field: &str, low: u32, high: u32, name: &str) -> Result<BTreeSet<u32>, String> {
    let mut values = BTreeSet::new();
    for item in field.split(',') {
        let (range, step) = match item.split_once('/') {
            Some((range, step)) => {
                let step: u32 = step
                    .parse()
                    .ok()
                    .filter(|step| *step > 0)
                    .ok_or_else(|| format!("`{item}` has an invalid step in the {name} field"))?;
                (range, step)
            }
            None => (item, 1),
        };
        let (start, end) = if range == "*" {
            (low, high)
        } else if let Some((start, end)) = range.split_once('-') {
            (number(start, low, high, name)?, number(end, low, high, name)?)
        } else {
            let start = number(range, low, high, name)?;
            // `5/15` means from 5 to the end, every 15.
            (start, if item.contains('/') { high } else { start })
        };
        if start > end {
            return Err(format!("`{item}` runs backwards in the {name} field"));
        }
        values.extend((start..=end).step_by(step as usize));
    }
    Ok(values)
}

fn number(text: &str, low: u32, high: u32, name: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .ok()
        .filter(|value| (low..=high).contains(value))
        .ok_or_else(|| format!("`{text}` is not a {name} between {low} and {high}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn weekdays_at_nine_in_kolkata() {
        let schedule = Schedule::parse("0 9 * * 1-5", "Asia/Kolkata").unwrap();
        // Friday 2026-10-30 10:00 IST is after that day's slot; next is Monday.
        let next = schedule.next_after(at("2026-10-30T04:30:00Z")).unwrap();
        assert_eq!(next, at("2026-11-02T03:30:00Z"));
    }

    #[test]
    fn lists_ranges_steps_and_sunday_as_seven() {
        let schedule = Schedule::parse("0,30 */6 1-7 * 7", "UTC").unwrap();
        assert_eq!(schedule.spec.hours, BTreeSet::from([0, 6, 12, 18]));
        assert_eq!(schedule.spec.minutes, BTreeSet::from([0, 30]));
        assert_eq!(schedule.spec.days_of_week, BTreeSet::from([0]));
        assert!(schedule.spec.dom_restricted && schedule.spec.dow_restricted);
    }

    #[test]
    fn day_fields_match_either_when_both_are_restricted() {
        // The 13th, or any Friday.
        let schedule = Schedule::parse("0 12 13 * 5", "UTC").unwrap();
        // Thursday 2026-10-29: next is Friday the 30th, not the 13th of November.
        assert_eq!(schedule.next_after(at("2026-10-29T13:00:00Z")).unwrap(), at("2026-10-30T12:00:00Z"));
    }

    #[test]
    fn every_refusal_names_its_reason() {
        let cases = [
            ("* * * *", "UTC", "five"),
            ("61 * * * *", "UTC", "minute"),
            ("0 24 * * *", "UTC", "hour"),
            ("0 9 * 13 *", "UTC", "month"),
            ("0 9 * * 8", "UTC", "day of week"),
            ("*/0 * * * *", "UTC", "step"),
            ("30-10 * * * *", "UTC", "backwards"),
            ("0 9 * * 1-5", "Mars/Olympus", "IANA"),
            ("*/5 * * * *", "UTC", "15 minutes"),
            ("0 0 30 2 *", "UTC", "never fires"),
        ];
        for (cron, timezone, needle) in cases {
            let error = Schedule::parse(cron, timezone).unwrap_err();
            assert!(error.contains(needle), "{cron} {timezone}: {error}");
        }
    }

    #[test]
    fn the_gap_floor_counts_the_wrap_across_midnight() {
        assert_eq!(Schedule::parse("*/15 * * * *", "UTC").unwrap().min_gap_minutes(), 15);
        assert!(Schedule::parse("0,50 23 * * *", "UTC").is_ok());
        // 23:55 then 00:05 the next day is ten minutes apart.
        assert!(Schedule::parse("5,55 0,23 * * *", "UTC").unwrap_err().contains("10 minutes"));
    }

    #[test]
    fn fires_between_is_bounded() {
        let schedule = Schedule::parse("0 * * * *", "UTC").unwrap();
        let fires = schedule.fires_between(at("2026-10-29T00:00:00Z"), at("2026-10-30T00:00:00Z"), 5);
        assert_eq!(fires.len(), 5);
        assert_eq!(fires[0], at("2026-10-29T01:00:00Z"));
    }
}
