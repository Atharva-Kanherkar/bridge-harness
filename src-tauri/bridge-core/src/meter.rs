//! Menu-bar usage meter, ported from CodexBar (MIT licence,
//! steipete/CodexBar).
//!
//! Direct ports (logic copied, translated Swift → Rust):
//! - `UsagePace.weekly` (`Sources/CodexBarCore/UsagePace.swift`): even-rate
//!   pace, deficit/reserve delta, run-out ETA, last-until-reset, and the
//!   speed multiplier needed to land exactly on the reset.
//! - `AdaptiveRefreshPolicy` table (`Sources/CodexBar/AdaptiveRefreshPolicy.swift`,
//!   documented in `docs/refresh-loop.md`): constrained 30m, recent-interaction
//!   2m, warm/coding-activity 5m, idle 15m, long-idle 30m.
//! - Pace visibility rule (`docs/ui.md`): hidden until 3% of the window has
//!   elapsed; the weekly menu-bar token is the one exception at 1%.
//!
//! Bridge differences (deliberate, not drift):
//! - Windows arrive from Bridge's own `usage.updated` rate-limit frames, not
//!   from CodexBar's fetch pipeline (OAuth/cookies/PTY). The math is identical;
//!   only the source differs.
//! - Only Codex and Claude are wired in v1. Every other CodexBar provider id is
//!   present in the registry as `planned` so follow-ups extend coverage without
//!   changing the contract.

use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};

/// A CodexBar provider id. `codex` and `claude` are live in v1; the rest of
/// CodexBar's matrix is registered as planned (see [`PROVIDER_REGISTRY`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MeterProvider {
    Codex,
    Claude,
}

impl MeterProvider {
    pub fn id(self) -> &'static str {
        match self {
            MeterProvider::Codex => "codex",
            MeterProvider::Claude => "claude",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MeterProvider::Codex => "Codex",
            MeterProvider::Claude => "Claude",
        }
    }
}

/// One provider in the meter registry: live ones report windows, planned ones
/// name the CodexBar source they will reuse when wired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterProviderEntry {
    pub id: String,
    pub label: String,
    pub supported: bool,
    /// Why a planned provider is not live yet (absent for live providers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_source: Option<String>,
}

/// A slice of CodexBar's 69-provider matrix (`docs/*.md` upstream). v1 wires
/// the two Bridge already reads (Codex rate-limit frames, Claude `/usage`
/// probe); 41 further providers are registered below as planned follow-ups.
const PLANNED_PROVIDERS: &[(&str, &str, &str)] = &[
    ("openai", "OpenAI", "Admin API key usage/cost graphs"),
    ("azure-openai", "Azure OpenAI", "API key, endpoint and deployment probe"),
    ("cursor", "Cursor", "browser session cookies"),
    ("opencode", "OpenCode", "browser cookies / local SQLite"),
    ("gemini", "Gemini", "Gemini CLI OAuth quota API"),
    ("antigravity", "Antigravity", "local language-server probe"),
    ("copilot", "Copilot", "GitHub device flow + usage API"),
    ("devin", "Devin", "browser localStorage session or bearer token"),
    ("zai", "z.ai", "API token quota"),
    ("manus", "Manus", "browser session_id credits"),
    ("minimax", "MiniMax", "API token / cookie coding-plan usage"),
    ("t3chat", "T3 Chat", "browser cookies base + overage buckets"),
    ("kimi", "Kimi", "auth token weekly + 5h windows"),
    ("kilo", "Kilo", "API token with CLI-auth fallback"),
    ("kiro", "Kiro", "CLI monthly + bonus credits"),
    ("vertexai", "Vertex AI", "gcloud OAuth + local cost tracking"),
    ("augment", "Augment", "Augment CLI or browser cookies"),
    ("amp", "Amp", "browser cookie Amp Free usage"),
    ("ollama", "Ollama", "API key + browser cookies"),
    ("synthetic", "Synthetic", "API key rolling windows"),
    ("jetbrains", "JetBrains AI", "local XML quota"),
    ("warp", "Warp", "API token GraphQL limits"),
    ("elevenlabs", "ElevenLabs", "API key character credits"),
    ("openrouter", "OpenRouter", "API token credit tracking"),
    ("windsurf", "Windsurf", "browser session / local SQLite"),
    ("zed", "Zed", "editor Keychain session"),
    ("perplexity", "Perplexity", "account usage credits"),
    ("mistral", "Mistral", "browser cookies spend + credits"),
    ("deepseek", "DeepSeek", "API key balance"),
    ("fireworks", "Fireworks", "API key 30-day spend"),
    ("deepinfra", "DeepInfra", "API key balance + spend"),
    ("moonshot", "Moonshot", "API key balance"),
    ("venice", "Venice", "API key balance"),
    ("codebuff", "Codebuff", "API token credits + weekly limit"),
    ("bedrock", "AWS Bedrock", "AWS keys Cost Explorer spend"),
    ("grok", "Grok", "Grok CLI billing RPC"),
    ("groqcloud", "GroqCloud", "API key Prometheus metrics"),
    ("litellm", "LiteLLM", "virtual key budget/spend"),
    ("deepgram", "Deepgram", "API key usage summaries"),
    ("poe", "Poe", "API key point balance"),
    ("xai", "xAI", "management API key + team spend"),
];

/// The full registry: live providers first, then planned ones in CodexBar order.
pub fn provider_registry() -> Vec<MeterProviderEntry> {
    let mut entries = vec![
        MeterProviderEntry { id: MeterProvider::Codex.id().into(), label: MeterProvider::Codex.label().into(), supported: true, planned_source: None },
        MeterProviderEntry { id: MeterProvider::Claude.id().into(), label: MeterProvider::Claude.label().into(), supported: true, planned_source: None },
    ];
    entries.extend(PLANNED_PROVIDERS.iter().map(|(id, label, source)| MeterProviderEntry {
        id: (*id).into(),
        label: (*label).into(),
        supported: false,
        planned_source: Some((*source).into()),
    }));
    entries
}

/// One quota window, mirroring CodexBar's `RateWindow`: percentage used plus
/// the reset the countdown reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterWindow {
    pub id: String,
    pub label: String,
    pub used_percent: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<i64>,
    /// RFC 3339 reset instant, when the provider names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_in_seconds: Option<f64>,
}

/// Pace detail for one window: a direct port of `UsagePace.weekly`'s even-rate
/// path (CodexBar's historical-curve path needs its SQLite history store and
/// is a follow-up; the even-rate model is what every non-Codex provider uses).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterPace {
    pub stage: PaceStage,
    /// Signed delta vs the sustainable rate: `+11` is 11% ahead of pace
    /// (deficit), `-8` is 8% behind (reserve). CodexBar's compact token form.
    pub delta_percent: f64,
    pub expected_used_percent: f64,
    pub actual_used_percent: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<f64>,
    pub will_last_to_reset: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_multiplier_to_reset: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaceStage {
    OnTrack,
    SlightlyAhead,
    Ahead,
    FarAhead,
    SlightlyBehind,
    Behind,
    FarBehind,
}

fn pace_stage(delta: f64) -> PaceStage {
    let absolute = delta.abs();
    if absolute <= 2.0 {
        PaceStage::OnTrack
    } else if absolute <= 6.0 {
        if delta >= 0.0 { PaceStage::SlightlyAhead } else { PaceStage::SlightlyBehind }
    } else if absolute <= 12.0 {
        if delta >= 0.0 { PaceStage::Ahead } else { PaceStage::Behind }
    } else if delta >= 0.0 {
        PaceStage::FarAhead
    } else {
        PaceStage::FarBehind
    }
}

fn clamp(value: f64, lower: f64, upper: f64) -> f64 {
    value.max(lower).min(upper)
}

/// The window's reset instant: the provider's absolute timestamp when present,
/// otherwise the live countdown Bridge normalizes absolute resets into (see
/// `src/usage.ts`, which stores only `resetsInSeconds`). Both halves must
/// accept both shapes — a countdown-only window is the common case on live
/// data, and requiring one shape while tests set both hid that asymmetry.
fn reset_instant(window: &MeterWindow, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if let Some(text) = window.resets_at.as_deref() {
        if let Ok(instant) = text.parse() {
            return Some(instant);
        }
    }
    window.resets_in_seconds.and_then(|secs| {
        (secs > 0.0 && secs.is_finite())
            .then(|| now + chrono::Duration::milliseconds((secs * 1000.0) as i64))
    })
}

/// Even-rate pace for one window at `now`.
///
/// Ports `UsagePace.weekly(window:now:defaultWindowMinutes:workDays:calendar:)`
/// with `work_days = None` (the explicit-schedule path needs a calendar and is
/// covered by [`pace_weekly_with_workdays`]). Returns `None` when the reset is
/// missing, outside the window, or the sample is contradictory (usage recorded
/// with zero elapsed time).
pub fn pace_weekly(window: &MeterWindow, now: DateTime<Utc>) -> Option<MeterPace> {
    pace_weekly_inner(window, now, None)
}

pub fn pace_weekly_with_workdays(
    window: &MeterWindow,
    now: DateTime<Utc>,
    work_days: u32,
    utc_offset_seconds: i64,
) -> Option<MeterPace> {
    pace_weekly_inner(window, now, Some((work_days, utc_offset_seconds)))
}

fn pace_weekly_inner(
    window: &MeterWindow,
    now: DateTime<Utc>,
    work_schedule: Option<(u32, i64)>,
) -> Option<MeterPace> {
    let resets_at = reset_instant(window, now)?;
    let minutes = window.window_minutes.unwrap_or(10_080);
    if minutes <= 0 {
        return None;
    }
    let duration = minutes as f64 * 60.0;
    let time_until_reset = (resets_at - now).num_milliseconds() as f64 / 1000.0;
    if time_until_reset <= 0.0 || time_until_reset > duration {
        return None;
    }
    let elapsed = clamp(duration - time_until_reset, 0.0, duration);
    let (expected, pace_elapsed, effective_remaining) = match work_schedule {
        Some((days, offset)) if (2..7).contains(&days) && minutes == 10_080 => {
            let progress = workday_progress(now, duration, resets_at, days, offset)?;
            (progress.expected_used_percent(), progress.elapsed_seconds, progress.remaining_seconds)
        }
        _ => (clamp(elapsed / duration * 100.0, 0.0, 100.0), elapsed, time_until_reset),
    };
    let actual = clamp(window.used_percent, 0.0, 100.0);
    // Guard on the clock the expectation runs on, not wall time: a window that
    // opens on a non-workday has zero pace-elapsed while wall time advances,
    // and recording usage against a zero expectation is contradictory data.
    if pace_elapsed == 0.0 && actual > 0.0 {
        return None;
    }
    let delta = actual - expected;
    let projected_remaining = if pace_elapsed > 0.0 {
        actual * effective_remaining / pace_elapsed
    } else {
        0.0
    };
    let speed_multiplier_to_reset = if (100.0 - actual) > 0.0 && projected_remaining > 0.0 {
        let multiplier = (100.0 - actual) / projected_remaining;
        multiplier.is_finite().then_some(multiplier)
    } else {
        None
    };
    let (eta_seconds, will_last_to_reset) = if actual >= 100.0 {
        (Some(0.0), false)
    } else if pace_elapsed > 0.0 && actual > 0.0 {
        let rate = actual / pace_elapsed;
        if rate > 0.0 {
            let candidate = (100.0 - actual) / rate;
            if candidate >= effective_remaining {
                (None, true)
            } else {
                (Some(candidate), false)
            }
        } else {
            (None, false)
        }
    } else if pace_elapsed > 0.0 {
        (None, true)
    } else {
        (None, false)
    };
    Some(MeterPace {
        stage: pace_stage(delta),
        delta_percent: delta,
        expected_used_percent: expected,
        actual_used_percent: actual,
        eta_seconds,
        will_last_to_reset,
        speed_multiplier_to_reset,
    })
}

struct WorkdayProgress {
    total_seconds: f64,
    elapsed_seconds: f64,
    remaining_seconds: f64,
}

impl WorkdayProgress {
    fn expected_used_percent(&self) -> f64 {
        if self.total_seconds <= 0.0 {
            return 0.0;
        }
        clamp(self.elapsed_seconds / self.total_seconds * 100.0, 0.0, 100.0)
    }
}

/// Weekly work-day split at the viewer's local day boundaries (CodexBar
/// `workdayProgress`). `utc_offset_seconds` is the viewer's zone offset (east
/// positive); day slices and weekday classification both run in local time so
/// weekend detection is correct around local midnight.
fn workday_progress(
    now: DateTime<Utc>,
    duration: f64,
    resets_at: DateTime<Utc>,
    work_days: u32,
    utc_offset_seconds: i64,
) -> Option<WorkdayProgress> {
    let window_start = resets_at - chrono::Duration::milliseconds((duration * 1000.0) as i64);
    let mut total = 0.0;
    let mut elapsed = 0.0;
    let mut remaining = 0.0;
    let mut cursor = window_start;
    while cursor < resets_at {
        let boundary = next_local_day_boundary(cursor, utc_offset_seconds)?;
        let slice_end = boundary.min(resets_at);
        if is_workday(cursor, work_days, utc_offset_seconds) {
            let slice = (slice_end - cursor).num_milliseconds() as f64 / 1000.0;
            total += slice;
            if now > cursor {
                elapsed += (now.min(slice_end) - cursor).num_milliseconds() as f64 / 1000.0;
            }
            if now < slice_end {
                remaining += (slice_end - now.max(cursor)).num_milliseconds() as f64 / 1000.0;
            }
        }
        cursor = slice_end;
    }
    (total > 0.0).then_some(WorkdayProgress { total_seconds: total, elapsed_seconds: elapsed, remaining_seconds: remaining })
}

fn next_local_day_boundary(after: DateTime<Utc>, utc_offset_seconds: i64) -> Option<DateTime<Utc>> {
    let local = after + chrono::Duration::seconds(utc_offset_seconds);
    let next_local_midnight = local.date_naive().succ_opt()?.and_hms_opt(0, 0, 0)?;
    let boundary = DateTime::<Utc>::from_naive_utc_and_offset(next_local_midnight, Utc)
        - chrono::Duration::seconds(utc_offset_seconds);
    (boundary > after).then_some(boundary)
}

/// Monday = 1 .. Sunday = 7 in the viewer's local time; `work_days` counts
/// from Monday.
fn is_workday(instant: DateTime<Utc>, work_days: u32, utc_offset_seconds: i64) -> bool {
    let local = instant + chrono::Duration::seconds(utc_offset_seconds);
    local.weekday().number_from_monday() <= work_days
}

/// Whether pace is shown for a window (`docs/ui.md`): hidden until 3% of the
/// window has elapsed; the weekly menu-bar token appears after 1%.
pub fn pace_visible(window: &MeterWindow, now: DateTime<Utc>, weekly_menu_token: bool) -> bool {
    let Some(resets_at) = reset_instant(window, now) else {
        return false;
    };
    let minutes = window.window_minutes.unwrap_or(10_080);
    if minutes <= 0 {
        return false;
    }
    let duration = minutes as f64 * 60.0;
    let elapsed = duration - (resets_at - now).num_milliseconds() as f64 / 1000.0;
    if elapsed <= 0.0 {
        return false;
    }
    let threshold = if weekly_menu_token && minutes == 10_080 { 0.01 } else { 0.03 };
    elapsed / duration >= threshold
}

/// Human pace line: "3% in deficit · runs out in 2h" / "5% in reserve · lasts
/// until reset" / "on pace". CodexBar menu-card copy, compacted to one line.
pub fn pace_label(pace: &MeterPace) -> String {
    let delta = pace.delta_percent.round() as i64;
    let head = match pace.stage {
        PaceStage::OnTrack => "on pace".to_owned(),
        PaceStage::SlightlyAhead | PaceStage::Ahead | PaceStage::FarAhead => {
            format!("{delta}% in deficit")
        }
        PaceStage::SlightlyBehind | PaceStage::Behind | PaceStage::FarBehind => {
            format!("{}% in reserve", delta.abs())
        }
    };
    let tail = if pace.actual_used_percent >= 100.0 {
        "limit reached".to_owned()
    } else if pace.will_last_to_reset {
        "lasts until reset".to_owned()
    } else if let Some(eta) = pace.eta_seconds {
        format!("runs out in {}", compact_duration(eta))
    } else {
        "reset timing only".to_owned()
    };
    format!("{head} · {tail}")
}

/// Signed compact delta for menu-bar tokens (`+11%` ahead, `-8%` behind).
pub fn pace_token_delta(pace: &MeterPace) -> String {
    format!("{:+.0}%", pace.delta_percent)
}

fn compact_duration(seconds: f64) -> String {
    let total = seconds.round() as i64;
    let days = total / 86_400;
    let hours = (total % 86_400) / 3_600;
    let minutes = (total % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        "soon".to_owned()
    }
}

/// Adaptive refresh input: all signals CodexBar's pure policy reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdaptiveInput {
    pub now_ms: i64,
    pub last_menu_open_ms: Option<i64>,
    /// Latest local Codex/Claude transcript activity (agent-aware mode only).
    pub last_coding_activity_ms: Option<i64>,
    pub low_power_mode: bool,
    pub thermally_constrained: bool,
    pub agent_aware: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AdaptiveReason {
    Constrained,
    RecentInteraction,
    Warm,
    CodingActivity,
    Idle,
    LongIdle,
}

impl AdaptiveReason {
    pub fn as_str(self) -> &'static str {
        match self {
            AdaptiveReason::Constrained => "constrained",
            AdaptiveReason::RecentInteraction => "recentInteraction",
            AdaptiveReason::Warm => "warm",
            AdaptiveReason::CodingActivity => "codingActivity",
            AdaptiveReason::Idle => "idle",
            AdaptiveReason::LongIdle => "longIdle",
        }
    }
}

/// Next automatic refresh delay, ported from CodexBar's `AdaptiveRefreshPolicy`
/// table (first match wins; every decision lands in 2–30 minutes).
pub fn adaptive_delay(input: AdaptiveInput) -> (std::time::Duration, AdaptiveReason) {
    use std::time::Duration;
    const MINUTE: i64 = 60_000;
    if input.low_power_mode || input.thermally_constrained {
        return (Duration::from_secs(1_800), AdaptiveReason::Constrained);
    }
    if let Some(opened) = input.last_menu_open_ms {
        let age = input.now_ms.saturating_sub(opened);
        if age <= 5 * MINUTE {
            return (Duration::from_secs(120), AdaptiveReason::RecentInteraction);
        }
        if age <= 60 * MINUTE {
            return (Duration::from_secs(300), AdaptiveReason::Warm);
        }
        if age <= 4 * 60 * MINUTE {
            // Agent-aware coding activity can only bring the tick forward to
            // `warm`, never postpone it (CodexBar policy note).
            if input.agent_aware {
                if let Some(activity) = input.last_coding_activity_ms {
                    if input.now_ms.saturating_sub(activity) < 5 * MINUTE {
                        return (Duration::from_secs(300), AdaptiveReason::CodingActivity);
                    }
                }
            }
            return (Duration::from_secs(900), AdaptiveReason::Idle);
        }
        return (Duration::from_secs(1_800), AdaptiveReason::LongIdle);
    }
    // No recorded menu open: coding activity alone can still warm one tick.
    if input.agent_aware {
        if let Some(activity) = input.last_coding_activity_ms {
            if input.now_ms.saturating_sub(activity) < 5 * MINUTE {
                return (Duration::from_secs(300), AdaptiveReason::CodingActivity);
            }
        }
    }
    (Duration::from_secs(1_800), AdaptiveReason::LongIdle)
}

/// Representative cadence for interval-derived heuristics (CodexBar
/// `nominalIntervalForHeuristics`): the steady-state active delay.
pub const NOMINAL_INTERVAL_SECONDS: u64 = 300;

/// Static registry payload for `meter/get_meter_snapshot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeterRegistry {
    pub providers: Vec<MeterProviderEntry>,
    pub adaptive_default_seconds: u64,
    pub nominal_interval_seconds: u64,
    pub attribution: String,
}

pub fn registry_snapshot() -> MeterRegistry {
    MeterRegistry {
        providers: provider_registry(),
        adaptive_default_seconds: NOMINAL_INTERVAL_SECONDS,
        nominal_interval_seconds: NOMINAL_INTERVAL_SECONDS,
        attribution: "Meter math ported from steipete/CodexBar (MIT)".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn window(used: f64, minutes: i64, resets_in: f64, now: DateTime<Utc>) -> MeterWindow {
        MeterWindow {
            id: "weekly".into(),
            label: "Weekly".into(),
            used_percent: used,
            window_minutes: Some(minutes),
            resets_at: Some((now + chrono::Duration::milliseconds((resets_in * 1000.0) as i64)).to_rfc3339()),
            resets_in_seconds: Some(resets_in),
        }
    }

    fn monday_noon() -> DateTime<Utc> {
        // 2026-09-07 is a Monday.
        Utc.with_ymd_and_hms(2026, 9, 7, 12, 0, 0).single().unwrap()
    }

    #[test]
    fn even_pace_reports_deficit_with_an_eta() {
        let now = monday_noon();
        // Half the week gone, 75% used: 25 points ahead, burns out early.
        let pace = pace_weekly(&window(75.0, 10_080, 5_040.0 * 60.0, now), now).unwrap();
        assert!((pace.expected_used_percent - 50.0).abs() < 1e-6);
        assert!((pace.delta_percent - 25.0).abs() < 1e-6);
        assert_eq!(pace.stage, PaceStage::FarAhead);
        assert!(!pace.will_last_to_reset);
        assert!(pace.eta_seconds.unwrap() > 0.0);
        // 25 points of headroom against 75 projected: exactly one third.
        assert!((pace.speed_multiplier_to_reset.unwrap() - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(pace_token_delta(&pace), "+25%");
        assert!(pace_label(&pace).contains("in deficit"));
    }

    #[test]
    fn under_pace_lasts_until_reset() {
        let now = monday_noon();
        let pace = pace_weekly(&window(20.0, 10_080, 5_040.0 * 60.0, now), now).unwrap();
        assert_eq!(pace.stage, PaceStage::FarBehind);
        assert!(pace.will_last_to_reset);
        // 80 points of headroom against 20 projected: four times the pace.
        assert!((pace.speed_multiplier_to_reset.unwrap() - 4.0).abs() < 1e-9);
        assert!(pace_label(&pace).contains("in reserve"));
        assert!(pace_label(&pace).contains("lasts until reset"));
    }

    #[test]
    fn contradictory_and_missing_resets_yield_nothing() {
        let now = monday_noon();
        assert!(pace_weekly(&MeterWindow { id: "w".into(), label: "W".into(), used_percent: 10.0, window_minutes: Some(10_080), resets_at: None, resets_in_seconds: None }, now).is_none());
        // Reset outside the window.
        assert!(pace_weekly(&window(10.0, 10_080, 20_000.0 * 60.0, now), now).is_none());
    }

    #[test]
    fn countdown_only_windows_take_the_same_path_as_absolute_resets() {
        // Live Bridge data carries countdowns (see usage.ts normalization),
        // not timestamps — a countdown-only window must produce pace.
        let now = monday_noon();
        let countdown = MeterWindow {
            id: "weekly".into(),
            label: "Weekly".into(),
            used_percent: 75.0,
            window_minutes: Some(10_080),
            resets_at: None,
            resets_in_seconds: Some(5_040.0 * 60.0),
        };
        let pace = pace_weekly(&countdown, now).expect("countdown-only windows produce pace");
        assert!((pace.delta_percent - 25.0).abs() < 1e-6);
        assert!(!pace.will_last_to_reset);
    }

    #[test]
    fn workdays_reshape_the_expected_rate() {
        let now = monday_noon();
        let plain = pace_weekly(&window(10.0, 10_080, 6.5 * 24.0 * 3_600.0, now), now).unwrap();
        let workdays = pace_weekly_with_workdays(&window(10.0, 10_080, 6.5 * 24.0 * 3_600.0, now), now, 5, 0).unwrap();
        assert!(workdays.expected_used_percent > plain.expected_used_percent);
    }

    #[test]
    fn workday_boundaries_follow_the_viewer_offset_not_utc() {
        // Monday 00:30 UTC is still Sunday evening in UTC-5: with a 5-day
        // week those minutes are non-work time.
        let monday_0030_utc = Utc.with_ymd_and_hms(2026, 9, 7, 0, 30, 0).single().unwrap();
        assert!(!is_workday(monday_0030_utc, 5, -5 * 3_600));
        assert!(is_workday(monday_0030_utc, 5, 0));
        assert!(is_workday(monday_0030_utc, 7, -5 * 3_600));
    }

    #[test]
    fn shared_fixtures_agree_with_the_typescript_port_case_for_case() {
        #[derive(serde::Deserialize)]
        struct Fixture {
            now: String,
            cases: Vec<FixtureCase>,
        }
        #[derive(serde::Deserialize)]
        struct FixtureCase {
            name: String,
            used_percent: f64,
            window_minutes: i64,
            resets_in_seconds: Option<f64>,
            resets_at: Option<String>,
            expected: FixtureExpected,
        }
        #[derive(serde::Deserialize)]
        struct FixtureExpected {
            delta: f64,
            stage: PaceStage,
            will_last: bool,
            eta_some: bool,
            multiplier: f64,
        }
        let fixture: Fixture = serde_json::from_str(include_str!(
            "../../../testing/fixtures/meter-pace-cases.json"
        ))
        .expect("pace fixtures parse");
        let now: DateTime<Utc> = fixture.now.parse().expect("fixture now parses");
        for case in &fixture.cases {
            let window = MeterWindow {
                id: "weekly".into(),
                label: "Weekly".into(),
                used_percent: case.used_percent,
                window_minutes: Some(case.window_minutes),
                resets_at: case.resets_at.clone(),
                resets_in_seconds: case.resets_in_seconds,
            };
            let pace = pace_weekly(&window, now).unwrap_or_else(|| panic!("{}: pace", case.name));
            assert!((pace.delta_percent - case.expected.delta).abs() < 1e-6, "{}: delta", case.name);
            assert_eq!(pace.stage, case.expected.stage, "{}: stage", case.name);
            assert_eq!(pace.will_last_to_reset, case.expected.will_last, "{}: will_last", case.name);
            assert_eq!(pace.eta_seconds.is_some(), case.expected.eta_some, "{}: eta", case.name);
            assert!(
                (pace.speed_multiplier_to_reset.unwrap() - case.expected.multiplier).abs() < 1e-4,
                "{}: multiplier",
                case.name
            );
        }
    }

    #[test]
    fn pace_visibility_follows_the_3_percent_rule_with_a_weekly_exception() {
        let now = monday_noon();
        let duration = 10_080.0 * 60.0;
        let early = window(1.0, 10_080, duration - 0.02 * duration, now);
        assert!(!pace_visible(&early, now, false));
        assert!(pace_visible(&early, now, true));
        let later = window(10.0, 10_080, duration - 0.05 * duration, now);
        assert!(pace_visible(&later, now, false));
    }

    #[test]
    fn adaptive_table_matches_codexbar_reasons() {
        let now = 1_000_000_000_000;
        let base = AdaptiveInput { now_ms: now, last_menu_open_ms: None, last_coding_activity_ms: None, low_power_mode: false, thermally_constrained: false, agent_aware: false };
        assert_eq!(adaptive_delay(base).1, AdaptiveReason::LongIdle);
        assert_eq!(adaptive_delay(AdaptiveInput { low_power_mode: true, ..base }).1, AdaptiveReason::Constrained);
        assert_eq!(adaptive_delay(AdaptiveInput { last_menu_open_ms: Some(now - 60_000), ..base }).1, AdaptiveReason::RecentInteraction);
        assert_eq!(adaptive_delay(AdaptiveInput { last_menu_open_ms: Some(now - 30 * 60_000), ..base }).1, AdaptiveReason::Warm);
        assert_eq!(adaptive_delay(AdaptiveInput { last_menu_open_ms: Some(now - 2 * 60 * 60_000), ..base }).1, AdaptiveReason::Idle);
        let activity = AdaptiveInput { last_menu_open_ms: Some(now - 2 * 60 * 60_000), last_coding_activity_ms: Some(now - 60_000), agent_aware: true, ..base };
        assert_eq!(adaptive_delay(activity).1, AdaptiveReason::CodingActivity);
        // Non-agent-aware activity never warms the tick.
        let plain = AdaptiveInput { agent_aware: false, ..activity };
        assert_eq!(adaptive_delay(plain).1, AdaptiveReason::Idle);
    }

    #[test]
    fn registry_names_codex_and_claude_live_with_planned_follow_ups() {
        let registry = provider_registry();
        let live: Vec<&str> = registry.iter().filter(|entry| entry.supported).map(|entry| entry.id.as_str()).collect();
        assert_eq!(live, vec!["codex", "claude"]);
        assert!(registry.iter().any(|entry| entry.id == "openrouter" && !entry.supported));
        assert_eq!(registry.len(), 2 + PLANNED_PROVIDERS.len());
        assert!(registry.len() > 30, "the CodexBar matrix must be visible, not just the live two");
    }
}
