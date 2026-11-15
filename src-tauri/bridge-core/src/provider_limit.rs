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

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Utc};
use regex::Regex;
use std::sync::OnceLock;

/// A provider telling Bridge that the account behind a harness is spent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderLimit {
    /// The matched phrase, for the cooldown reason and the ledger.
    pub signal: String,
    /// When the provider says it clears, when it says so at all.
    pub reset_at: Option<DateTime<Utc>>,
    /// Whether the account ran out or the provider is busy for everyone.
    pub kind: LimitKind,
}

/// The two things a "limit" frame can mean. Both cool a harness down; only a
/// usage limit is something a chat should wait out or switch away from on the
/// user's behalf. A capacity error clears on its own in seconds and is the
/// provider's in-place retry to make, not a reason to resend a turn later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    Usage,
    Capacity,
}

/// Claude and newer Codex builds say "You've hit your session limit" or
/// "your 5-hour limit" or "your weekly limit", none of which contains the
/// shared vocabulary's "usage limit". The phrase is read only from adapter
/// frames, which the provider wrote, never from worker prose: it is the
/// provider's own account statement, so the wider reading is safe here.
fn hit_your_limit(haystack: &str) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| Regex::new(r"hit your [a-z0-9 -]{0,24}limit").expect("static pattern"))
        .is_match(haystack)
}

/// A bare "429" only counts as a whole number. As a substring it matches hex
/// request ids and cf-ray headers in unrelated 401s.
fn whole_429(haystack: &str) -> bool {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| Regex::new(r"(?:^|[^0-9a-z])429(?:[^0-9a-z]|$)").expect("static pattern"))
        .is_match(haystack)
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
    // Curly apostrophes ("You’ve") are as common as straight ones in the
    // recorded frames; folding them keeps one pattern per phrase.
    let haystack = text.to_lowercase().replace('\u{2019}', "'");
    // The same vocabulary the result-prose classifier uses. One list, so a
    // phrase recognised on one path cannot be missed on the other. The frame
    // reading adds only the provider's own "hit your ... limit" wording.
    let signal = match crate::worker_retry::quota_signal_in(&haystack) {
        Some("429") if !whole_429(&haystack) => None,
        found => found,
    }
    .or_else(|| hit_your_limit(&haystack).then_some("usage limit"))?;
    let kind = if matches!(signal, "overloaded" | "at capacity") {
        LimitKind::Capacity
    } else {
        LimitKind::Usage
    };
    Some(ProviderLimit {
        signal: signal.to_owned(),
        reset_at: parse_reset_at(&haystack, now),
        kind,
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

/// Codex says it in words: "Try again at Sep 7th, 11:35 AM." or, in newer
/// builds, "try again at Sep 7th, 2026 11:35 AM." Claude says "resets 7:50pm
/// (Asia/Tokyo)". A bare time is read as the next occurrence of that clock
/// time; a date without a year is read against the current year.
///
/// The clock is the one the frame names, else the user's, never UTC. Reading
/// "11:35 AM" as 11:35 UTC on a machine in PDT puts the reset seven hours
/// early, and routing resumes straight into the account that is still
/// exhausted. Claude names its zone because the account's zone and the
/// machine's need not agree (a laptop that travelled, a VPN), and ignoring it
/// would be wrong by the difference.
fn wall_clock_reset(haystack: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(
            r"(?:try again|resets?|available again|back)\s*(?:at|on|in)?\s*(?:(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?\s+(\d{1,2})(?:st|nd|rd|th)?,?\s*(?:(\d{4}),?\s*)?)?(\d{1,2})(?::(\d{2}))?\s*(am|pm)(?:\s*\(([a-z_]+(?:/[a-z_+-]+){1,2})\))?",
        )
        .expect("static pattern")
    });
    let captures = pattern.captures(haystack)?;
    let hour: u32 = captures.get(4)?.as_str().parse().ok()?;
    if hour == 0 || hour > 12 {
        return None;
    }
    let minute: u32 = match captures.get(5) {
        Some(minute) => minute.as_str().parse().ok().filter(|minute| *minute < 60)?,
        None => 0,
    };
    let hour24 = match (hour, captures.get(6)?.as_str()) {
        (12, "am") => 0,
        (12, "pm") => 12,
        (hour, "pm") => hour + 12,
        (hour, _) => hour,
    };
    let date = match (captures.get(1), captures.get(2)) {
        (Some(month), Some(day)) => Some((
            month_number(month.as_str())?,
            day.as_str().parse::<u32>().ok()?,
            match captures.get(3) {
                Some(year) => Some(year.as_str().parse::<i32>().ok()?),
                None => None,
            },
        )),
        _ => None,
    };
    let clock = WallClock { date, hour: hour24, minute };
    match captures.get(7).and_then(|zone| named_zone(zone.as_str())) {
        Some(zone) => clock.next_after(&zone, now),
        None => clock.next_after(&Local, now),
    }
}

/// An IANA zone from the frame's lowercased text. The table is matched case
/// insensitively against chrono-tz's own names, so "asia/calcutta" finds the
/// `Asia/Calcutta` alias without an extra crate feature.
fn named_zone(lowered: &str) -> Option<chrono_tz::Tz> {
    chrono_tz::TZ_VARIANTS
        .iter()
        .find(|zone| zone.name().eq_ignore_ascii_case(lowered))
        .copied()
}

/// A reset stated on a wall clock: a time, and a date when the frame gave one.
struct WallClock {
    /// Month, day, and the year when stated.
    date: Option<(u32, u32, Option<i32>)>,
    hour: u32,
    minute: u32,
}

impl WallClock {
    /// The instant this clock reading names in `zone`, as the next occurrence
    /// after `now` when the frame left the date or the year out.
    fn next_after<Z: TimeZone>(&self, zone: &Z, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let zoned_now = now.with_timezone(zone);
        match self.date {
            Some((month, day, Some(year))) => zoned_reset(zone, year, month, day, self.hour, self.minute),
            Some((month, day, None)) => {
                let dated = zoned_reset(zone, zoned_now.year(), month, day, self.hour, self.minute)?;
                // No year in the text, so a date that already passed is next year's.
                if dated > now {
                    Some(dated)
                } else {
                    zoned_reset(zone, zoned_now.year() + 1, month, day, self.hour, self.minute)
                }
            }
            None => {
                let today = zoned_now.date_naive();
                let first = zoned_reset_on(zone, today, self.hour, self.minute)?;
                if first > now {
                    Some(first)
                } else {
                    // A day later on the wall clock, which a DST boundary makes
                    // 23 or 25 hours rather than 24, so it is rebuilt from the
                    // next local date, not by adding a fixed span.
                    zoned_reset_on(zone, today.succ_opt()?, self.hour, self.minute)
                }
            }
        }
    }
}

fn zoned_reset_on<Z: TimeZone>(zone: &Z, date: NaiveDate, hour: u32, minute: u32) -> Option<DateTime<Utc>> {
    zoned_reset(zone, date.year(), date.month(), date.day(), hour, minute)
}

/// A wall-clock instant in `zone` as UTC. Ambiguous and skipped times (the two
/// DST edges) resolve to the later reading, which errs toward waiting longer
/// rather than resuming into an exhausted account.
fn zoned_reset<Z: TimeZone>(
    zone: &Z,
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
) -> Option<DateTime<Utc>> {
    let zoned = zone.with_ymd_and_hms(year, month, day, hour, minute, 0).latest()?;
    Some(zoned.with_timezone(&Utc))
}

/// [`zoned_reset`] on the machine's clock.
#[cfg(test)]
fn local_reset(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Option<DateTime<Utc>> {
    zoned_reset(&Local, year, month, day, hour, minute)
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

    /// The exact shape the live database recorded 20,595 times. Codex prints
    /// the reset on the user's wall clock with no zone and no year, so the
    /// expectation is built the same way rather than pinned to UTC — on a
    /// machine in PDT, reading "11:35 AM" as 11:35 UTC would resume routing
    /// seven hours early, straight back into the exhausted account.
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
            Some(local_reset(2026, 9, 7, 11, 35).unwrap()),
            "the provider named a reset roughly 25 hours out, on its own clock"
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
        assert_eq!(limit.reset_at, Some(local_reset(2027, 1, 2, 9, 0).unwrap()));
    }

    /// A bare clock time resolves to its next occurrence on the user's
    /// clock. Which calendar day that is depends on the machine's zone, so
    /// the assertion is the property rather than a fixed instant.
    #[test]
    fn a_bare_time_resolves_to_its_next_local_occurrence() {
        use chrono::Timelike;
        let limit = detect_at("usage limit reached, try again at 9:00 AM", now()).unwrap();
        let reset = limit.reset_at.expect("a bare time is still a hint");
        assert!(reset > now(), "a reset in the past is not a reset");
        assert!(
            reset <= now() + Duration::days(1) + Duration::hours(2),
            "the next 9am is at most a day away, DST included: {reset}"
        );
        let local = reset.with_timezone(&Local);
        assert_eq!((local.hour(), local.minute()), (9, 0), "{local}");
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

    /// The labeled corpus of real provider frames, as an eval with gates:
    /// every limit recognised (recall 1.0), nothing else (no false
    /// positives), the right kind, and the stated reset recovered to the
    /// minute. The expected instant is computed independently of the parser,
    /// by walking calendar days in the frame's zone.
    #[test]
    fn eval_recorded_limit_frames() {
        use chrono::{Datelike, TimeZone};
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../../testing/fixtures/limit-recovery/frames.json"
        ))
        .unwrap();
        let now: DateTime<Utc> = corpus["now"].as_str().unwrap().parse().unwrap();
        fn next_occurrence<Z: TimeZone>(zone: &Z, now: DateTime<Utc>, hour: u32, minute: u32) -> DateTime<Utc> {
            let start = now.with_timezone(zone).date_naive();
            (0..3)
                .filter_map(|offset| {
                    let date = start + Duration::days(offset);
                    zone.with_ymd_and_hms(date.year(), date.month(), date.day(), hour, minute, 0)
                        .latest()
                        .map(|instant| instant.with_timezone(&Utc))
                })
                .find(|instant| *instant > now)
                .unwrap()
        }
        let (mut positives, mut recalled, mut false_positives, mut resets, mut resets_right) = (0, 0, 0, 0, 0);
        let mut failures = Vec::new();
        for case in corpus["cases"].as_array().unwrap() {
            let text = case["text"].as_str().unwrap();
            let got = detect_at(text, now);
            if !case["limit"].as_bool().unwrap() {
                if got.is_some() {
                    false_positives += 1;
                    failures.push(format!("false positive: {text}"));
                }
                continue;
            }
            positives += 1;
            let Some(limit) = got else {
                failures.push(format!("missed: {text}"));
                continue;
            };
            recalled += 1;
            let kind = if limit.kind == LimitKind::Usage { "usage" } else { "capacity" };
            if kind != case["kind"].as_str().unwrap() {
                failures.push(format!("kind {kind}: {text}"));
            }
            let reset = &case["reset"];
            let expected = if reset.is_null() {
                None
            } else if let Some(epoch) = reset["epoch"].as_i64() {
                Some(Utc.timestamp_opt(epoch, 0).unwrap())
            } else if let Some(after) = reset["afterSeconds"].as_i64() {
                Some(now + Duration::seconds(after))
            } else {
                let (hour, minute) = (reset["hour"].as_u64().unwrap() as u32, reset["minute"].as_u64().unwrap() as u32);
                let zone = reset["zone"].as_str().unwrap();
                Some(match (reset["year"].as_i64(), zone) {
                    (Some(year), "local") => Local
                        .with_ymd_and_hms(year as i32, reset["month"].as_u64().unwrap() as u32, reset["day"].as_u64().unwrap() as u32, hour, minute, 0)
                        .latest()
                        .unwrap()
                        .with_timezone(&Utc),
                    (None, "local") => next_occurrence(&Local, now, hour, minute),
                    (None, named) => next_occurrence(&named.parse::<chrono_tz::Tz>().unwrap(), now, hour, minute),
                    (Some(_), named) => panic!("dated case in a named zone is not in the corpus: {named}"),
                })
            };
            if expected.is_some() {
                resets += 1;
            }
            if limit.reset_at == expected {
                if expected.is_some() {
                    resets_right += 1;
                }
            } else {
                failures.push(format!("reset {:?} != {:?}: {text}", limit.reset_at, expected));
            }
        }
        eprintln!(
            "limit-frame eval: recall {recalled}/{positives}, false positives {false_positives}, resets {resets_right}/{resets}"
        );
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!(recalled, positives);
        assert_eq!(false_positives, 0);
        assert_eq!(resets_right, resets);
    }

    mod fuzz {
        use super::super::*;
        use proptest::prelude::*;

        /// The case count, unless `PROPTEST_CASES` asks for another (mutation
        /// runs lower it; a soak raises it).
        fn fuzz_cases(default: u32) -> u32 {
            std::env::var("PROPTEST_CASES").ok().and_then(|value| value.parse().ok()).unwrap_or(default)
        }

        fn any_now() -> impl Strategy<Value = DateTime<Utc>> {
            // 2000-01-01 .. 2090-01-01
            (946_684_800i64..3_786_912_000i64).prop_map(|seconds| Utc.timestamp_opt(seconds, 0).unwrap())
        }

        /// Frame-shaped noise: the vocabulary, digits, clock words, zones.
        fn frame_text() -> impl Strategy<Value = String> {
            let pieces = prop::collection::vec(
                prop_oneof![
                    Just("You've hit your ".to_owned()),
                    Just("session limit".to_owned()),
                    Just(" · resets ".to_owned()),
                    Just("try again at ".to_owned()),
                    Just("retry-after: ".to_owned()),
                    Just("usage limit".to_owned()),
                    Just("429".to_owned()),
                    Just("overloaded".to_owned()),
                    Just("|".to_owned()),
                    Just(" (Asia/Tokyo)".to_owned()),
                    Just(" (Not/AZone)".to_owned()),
                    Just("Feb 30th, ".to_owned()),
                    Just("Sep 7th, 2026 ".to_owned()),
                    Just("am".to_owned()),
                    Just("pm".to_owned()),
                    Just(":".to_owned()),
                    "[0-9]{1,11}",
                    "\\PC{0,12}",
                ],
                0..12,
            );
            pieces.prop_map(|pieces| pieces.concat())
        }

        proptest! {
            #![proptest_config(ProptestConfig { cases: fuzz_cases(4096), ..ProptestConfig::default() })]

            /// Whatever a provider writes: no panic, and any reset Bridge
            /// believes is strictly ahead of now and inside the plausible
            /// window. A reset outside it would bench a provider or resume a
            /// chat at a nonsense time.
            #[test]
            fn any_frame_yields_only_a_plausible_reset(text in frame_text(), now in any_now()) {
                if let Some(limit) = detect_at(&text, now) {
                    if let Some(reset) = limit.reset_at {
                        prop_assert!(reset > now, "{text:?} -> {reset} at {now}");
                        prop_assert!(reset <= now + Duration::days(MAX_PLAUSIBLE_RESET_DAYS), "{text:?} -> {reset}");
                    }
                }
            }

            /// Arbitrary Unicode never panics the detector.
            #[test]
            fn arbitrary_text_never_panics(text in "\\PC{0,400}", now in any_now()) {
                let _ = detect_at(&text, now);
            }

            /// A stated clock time in a named zone lands on that wall clock,
            /// within a day of now, whatever the date.
            #[test]
            fn a_named_zone_reset_reads_back_on_that_zone_clock(hour in 1u32..=12, minute in 0u32..60, pm in any::<bool>(), now in any_now()) {
                use chrono::Timelike;
                let suffix = if pm { "pm" } else { "am" };
                let text = format!("You've hit your session limit · resets {hour}:{minute:02}{suffix} (Asia/Tokyo)");
                let limit = detect_at(&text, now).expect("a limit");
                let reset = limit.reset_at.expect("a reset");
                let tokyo = reset.with_timezone(&chrono_tz::Asia::Tokyo);
                let hour24 = match (hour, pm) { (12, false) => 0, (12, true) => 12, (hour, true) => hour + 12, (hour, false) => hour };
                prop_assert_eq!((tokyo.hour(), tokyo.minute()), (hour24, minute));
                prop_assert!(reset > now && reset <= now + Duration::hours(24));
            }
        }
    }
}
