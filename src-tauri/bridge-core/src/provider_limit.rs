//! Recognising "this provider is out of quota" from the provider's own words.
//!
//! Bridge already knew what to do about an exhausted harness — cool it down,
//! route the next delegation elsewhere. What it could not do was notice. The
//! only detector read the worker's typed result (`worker_retry::classify`),
//! and a rate-limited worker does not produce one: Codex fails the turn, emits
//! an `error` frame, and never writes an assistant message. So the cooldown
//! table sat empty through 12,155 rate-limited turns.
//!
//! This module reads the frame instead. Three providers say it three ways —
//! Codex "You've hit your usage limit. Try again at Sep 7th, 11:35 AM.",
//! Claude `rate_limit_error` with an epoch reset, OpenCode a bare 429 — and
//! all three carry the same two facts: that the account is out, and sometimes
//! when it comes back.
//!
//! The reset hint matters more than it looks. A limit that clears in 40 hours
//! and a limit that clears in 40 seconds are the same event to a detector that
//! only sees "429", and the old fixed 15-minute cooldown treated them alike —
//! which meant re-hammering an exhausted account 160 times before it cleared.

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use regex::Regex;
use std::sync::OnceLock;

/// A provider telling Bridge that the account behind a harness is spent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderLimit {
    /// The matched phrase, for the cooldown reason and the ledger.
    pub signal: String,
    /// When the provider says it clears, when it says so at all.
    pub reset_at: Option<DateTime<Utc>>,
}

/// The longest reset hint that is believable. A parsed timestamp beyond this
/// is a misparse — an unrelated number that happened to look like an epoch,
/// or a date whose year we guessed wrong — and a misparse here would keep a
/// working harness out of routing for months.
const MAX_PLAUSIBLE_RESET_DAYS: i64 = 30;

/// Read a provider limit out of an adapter error frame's text.
///
/// Returns `None` for every other kind of error, including the transient ones
/// (`ECONNRESET`, `socket hang up`) that a retry can genuinely clear. Being
/// wrong in that direction costs a retry; being wrong in the other direction
/// benches a healthy provider.
pub fn detect(text: &str) -> Option<ProviderLimit> {
    detect_at(text, Utc::now())
}

/// [`detect`] with an explicit "now", so reset parsing is testable without
/// waiting for the calendar.
pub fn detect_at(text: &str, now: DateTime<Utc>) -> Option<ProviderLimit> {
    let haystack = text.to_ascii_lowercase();
    // The same vocabulary the result-prose classifier uses. One list, so a
    // phrase recognised on one path cannot be missed on the other.
    let signal = crate::worker_retry::quota_signal_in(&haystack)?;
    Some(ProviderLimit {
        signal: signal.to_owned(),
        reset_at: parse_reset_at(&haystack, now),
    })
}

/// The provider's own reset hint, in the three shapes the adapters produce.
///
/// Ordered most explicit first: an epoch and a retry-after are unambiguous,
/// a wall-clock phrase has to be reconstructed against today's date and is
/// only trusted when the reconstruction lands in a plausible window.
fn parse_reset_at(haystack: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    epoch_reset(haystack, now)
        .or_else(|| retry_after_reset(haystack, now))
        .or_else(|| wall_clock_reset(haystack, now))
        .filter(|reset| plausible(*reset, now))
}

/// Claude's `rate_limit_error` carries the reset as unix seconds, either in a
/// `resets at <epoch>` phrase or in the `|<epoch>` suffix of its limit banner.
fn epoch_reset(haystack: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r"(?:reset[s]?[_ ]?at|reset[s]?|retry[_ ]?at)\D{0,12}(\d{10})\b|\|(\d{10})\b")
            .expect("static pattern")
    });
    let captures = pattern.captures(haystack)?;
    let seconds: i64 = captures
        .get(1)
        .or_else(|| captures.get(2))?
        .as_str()
        .parse()
        .ok()?;
    let reset = Utc.timestamp_opt(seconds, 0).single()?;
    (reset > now).then_some(reset)
}

/// OpenCode and raw HTTP 429s carry a delay rather than an instant.
fn retry_after_reset(haystack: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(
            r"retry[- ]?after\D{0,4}(\d{1,7})\s*(second|sec|s\b|minute|min|m\b|hour|hr|h\b)?",
        )
        .expect("static pattern")
    });
    let captures = pattern.captures(haystack)?;
    let amount: i64 = captures.get(1)?.as_str().parse().ok()?;
    let unit = captures
        .get(2)
        .map(|unit| unit.as_str())
        .unwrap_or("second");
    let delay = match unit.trim() {
        "minute" | "min" | "m" => Duration::minutes(amount),
        "hour" | "hr" | "h" => Duration::hours(amount),
        _ => Duration::seconds(amount),
    };
    Some(now + delay)
}

/// Codex says it in words: "Try again at Sep 7th, 11:35 AM." The date is
/// optional and the year never appears, so a bare time is read as the next
/// occurrence of that clock time and a date is read against the current year.
fn wall_clock_reset(haystack: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(
            r"(?:try again|resets?|available again|back)\s*(?:at|on|in)?\s*(?:(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?\s+(\d{1,2})(?:st|nd|rd|th)?,?\s*)?(\d{1,2})(?::(\d{2}))?\s*(am|pm)",
        )
        .expect("static pattern")
    });
    let captures = pattern.captures(haystack)?;
    let hour: u32 = captures.get(3)?.as_str().parse().ok()?;
    if hour == 0 || hour > 12 {
        return None;
    }
    let minute: u32 = captures
        .get(4)
        .and_then(|minute| minute.as_str().parse().ok())
        .unwrap_or(0);
    let hour24 = match (hour, captures.get(5)?.as_str()) {
        (12, "am") => 0,
        (12, "pm") => 12,
        (hour, "pm") => hour + 12,
        (hour, _) => hour,
    };
    let candidate = match (captures.get(1), captures.get(2)) {
        (Some(month), Some(day)) => {
            let month = month_number(month.as_str())?;
            let day: u32 = day.as_str().parse().ok()?;
            let dated = Utc
                .with_ymd_and_hms(now.year(), month, day, hour24, minute, 0)
                .single()?;
            // No year in the text, so a date that already passed is next year's.
            if dated > now {
                dated
            } else {
                Utc.with_ymd_and_hms(now.year() + 1, month, day, hour24, minute, 0)
                    .single()?
            }
        }
        _ => {
            let today = now.date_naive().and_hms_opt(hour24, minute, 0)?.and_utc();
            if today > now {
                today
            } else {
                today + Duration::days(1)
            }
        }
    };
    Some(candidate)
}

fn month_number(month: &str) -> Option<u32> {
    Some(match month {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => return None,
    })
}

/// A hint is only usable if it is ahead of now and inside the window any real
/// provider reset falls in. Everything else falls back to the caller's floor.
fn plausible(reset: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    reset > now && reset <= now + Duration::days(MAX_PLAUSIBLE_RESET_DAYS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 6, 10, 0, 0).unwrap()
    }

    /// The exact string the live database recorded 20,595 times.
    #[test]
    fn the_codex_usage_limit_frame_is_a_limit_with_its_stated_reset() {
        let limit = detect_at(
            "You've hit your usage limit. Try again at Sep 7th, 11:35 AM.",
            now(),
        )
        .expect("the frame that ran the loop must be recognised");
        assert_eq!(limit.signal, "usage limit");
        assert_eq!(
            limit.reset_at,
            Some(Utc.with_ymd_and_hms(2026, 9, 7, 11, 35, 0).unwrap()),
            "the provider named a reset roughly 25 hours out; a floor would re-hammer it"
        );
    }

    /// The compaction variant of the same failure, wrapped in the adapter's
    /// own prefix. 3,703 of these were recorded.
    #[test]
    fn the_compaction_wrapped_limit_is_recognised_too() {
        let limit = detect_at(
            "Error running remote compact task: You've hit your usage limit. Try again at Sep 7th, 11:35 AM.",
            now(),
        )
        .expect("a limit is a limit whatever task hit it");
        assert_eq!(limit.signal, "usage limit");
    }

    #[test]
    fn claude_states_its_reset_as_an_epoch() {
        let reset = Utc.with_ymd_and_hms(2026, 9, 6, 15, 0, 0).unwrap();
        let limit = detect_at(
            &format!(
                "{{\"type\":\"rate_limit_error\",\"message\":\"usage limit reached\",\"resets_at\":{}}}",
                reset.timestamp()
            ),
            now(),
        )
        .expect("claude's typed rate limit must be recognised");
        assert_eq!(limit.reset_at, Some(reset));
    }

    #[test]
    fn claude_limit_banners_carry_the_epoch_after_a_pipe() {
        let reset = Utc.with_ymd_and_hms(2026, 9, 6, 18, 30, 0).unwrap();
        let limit = detect_at(
            &format!("Claude usage limit reached|{}", reset.timestamp()),
            now(),
        )
        .unwrap();
        assert_eq!(limit.reset_at, Some(reset));
    }

    #[test]
    fn opencode_states_a_delay_rather_than_an_instant() {
        let limit = detect_at(
            "Request failed: 429 Too Many Requests (retry-after: 900)",
            now(),
        )
        .expect("a bare 429 is still a limit");
        assert_eq!(limit.signal, "429");
        assert_eq!(limit.reset_at, Some(now() + Duration::seconds(900)));
    }

    /// A hint Bridge cannot read is not a reason to invent one. The caller's
    /// floor applies instead, which is the conservative direction.
    #[test]
    fn an_unparseable_or_absent_reset_leaves_the_hint_empty() {
        for text in [
            "You've hit your usage limit.",
            "429 rate limit exceeded",
            "quota exceeded, contact your administrator",
        ] {
            let limit = detect_at(text, now()).expect("still a limit: {text}");
            assert_eq!(limit.reset_at, None, "{text}");
        }
    }

    /// Codex names a date without a year. Read against today, "Jan 2nd" in
    /// December is next year, not eleven months ago.
    #[test]
    fn a_dateless_year_rolls_forward_rather_than_landing_in_the_past() {
        let december = Utc.with_ymd_and_hms(2026, 12, 28, 9, 0, 0).unwrap();
        let limit = detect_at(
            "You've hit your usage limit. Try again at Jan 2nd, 9:00 AM.",
            december,
        )
        .unwrap();
        assert_eq!(
            limit.reset_at,
            Some(Utc.with_ymd_and_hms(2027, 1, 2, 9, 0, 0).unwrap())
        );
    }

    /// A bare clock time that has already passed today means tomorrow.
    #[test]
    fn a_bare_time_already_past_today_means_tomorrow() {
        let limit = detect_at("usage limit reached, try again at 9:00 AM", now()).unwrap();
        assert_eq!(
            limit.reset_at,
            Some(Utc.with_ymd_and_hms(2026, 9, 7, 9, 0, 0).unwrap())
        );
    }

    /// A reset months out is a misparse, and believing it would bench a
    /// healthy provider for a month.
    #[test]
    fn an_implausibly_distant_reset_is_discarded() {
        let limit = detect_at("usage limit reached; retry-after: 9000000 seconds", now()).unwrap();
        assert_eq!(limit.reset_at, None);
    }

    /// The whole point of a narrow vocabulary: a network fault is retryable
    /// in place and must not cool a provider down.
    #[test]
    fn transient_network_faults_are_not_provider_limits() {
        for text in [
            "socket hang up",
            "ECONNRESET while streaming",
            "Provider process exited with code 1",
            "Refusing to create helper binaries under temporary dir",
        ] {
            assert_eq!(detect_at(text, now()), None, "{text}");
        }
    }
}
