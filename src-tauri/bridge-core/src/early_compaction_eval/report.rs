//! The figures of the live eval, the pass bar and the report.
//!
//! Everything here is pure: the driver reads the database and the files, and
//! this module turns the raw rows into sums, a verdict and report text.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};

use crate::analytics::TokenUsage;
use crate::early_compaction::{IDLE_DELAY, SETTLE_LIMIT, THRESHOLD_TOKENS, WARM_LIMIT};
use crate::usage_pricing::{cost_microusd, ModelRate};

use super::workload::TurnScore;

/// The column label of the Codex figure that the eval adds for each
/// compaction. The harness does not report the tokens of that request.
pub(super) const BOUND_LABEL: &str = "compaction bound (not observed)";
/// The failure of a harness that does not show the cost of a compaction.
pub(super) const UNOBSERVED: &str = "compaction_usage_unobserved";
/// The statement about noise that each report carries.
pub(super) const NOISE: &str = "Each arm runs one time, so each figure is one sample and is noise-prone. \
A small change in the path of the model changes the number of requests, and so the sums. \
On Codex, the harness can send its skills list again (about 10,000 tokens) on any request. \
That resend adds noise to the Codex sums.";
/// The lowest score of arm `off` that the bar accepts.
const OFF_FLOOR: u32 = 6;
/// Arm `on` can score this number of points below arm `off`.
const SCORE_SLACK: u32 = 1;
const COMPACTED: &str = "compacted";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Arm {
    On,
    Off,
}

impl Arm {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
        }
    }
}

/// How the wait after a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WaitEnd {
    /// The due mark had a final decision, and the chat was idle.
    Decided,
    /// The mark was still there at `WARM_LIMIT`.
    WarmLimit,
    /// An attempt was still open at `SETTLE_LIMIT`.
    SettleLimit,
}

impl WaitEnd {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Decided => "decided",
            Self::WarmLimit => "warm_limit",
            Self::SettleLimit => "settle_limit",
        }
    }
}

/// The token sums of one chat.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Sums {
    pub uncached_input: i64,
    pub cache_write: i64,
    pub cache_read: i64,
    pub output: i64,
}

impl Sums {
    /// Uncached input, cache write and cache read: the figure of the bar.
    pub fn input(&self) -> i64 {
        self.uncached_input + self.cache_write + self.cache_read
    }
}

/// One `usage_ledger` row of the chat. A null column reads as 0.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LedgerRow {
    pub created_at: Option<DateTime<Utc>>,
    pub uncached_input: i64,
    pub cache_write: i64,
    pub cache_read: i64,
    pub output: i64,
}

impl LedgerRow {
    fn tokens(&self) -> i64 {
        self.uncached_input + self.cache_write + self.cache_read + self.output
    }
}

/// One `early_compaction_attempts` row of the chat.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AttemptRow {
    pub outcome: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub dispatched_at: Option<DateTime<Utc>>,
    pub settled_at: Option<DateTime<Utc>>,
    pub trigger_reading_id: Option<i64>,
    pub trigger_used_tokens: Option<i64>,
    pub boundary_pre_tokens: Option<i64>,
    pub boundary_post_tokens: Option<i64>,
    pub next_reading_id: Option<i64>,
    pub next_used_tokens: Option<i64>,
    pub error: Option<String>,
}

impl AttemptRow {
    fn compacted(&self) -> bool {
        self.outcome.as_deref() == Some(COMPACTED)
    }
}

/// The raw result of one arm.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ArmRun {
    pub arm: Arm,
    pub turns_total: usize,
    pub turns_finished: usize,
    pub max_points: u32,
    /// Why the arm stopped before the last turn.
    pub failure: Option<String>,
    /// The data directory of the core. It keeps the chat files.
    pub data_dir: Option<String>,
    pub scores: Vec<TurnScore>,
    /// The time of each scripted send.
    pub submissions: Vec<DateTime<Utc>>,
    /// The latest window reading after the wait of each turn.
    pub readings: Vec<Option<i64>>,
    pub waits: Vec<WaitEnd>,
    pub rows: Vec<LedgerRow>,
    pub attempts: Vec<AttemptRow>,
    /// The price of the model, or `None` when `usage_pricing` has no rate.
    pub rate: Option<ModelRate>,
}

impl ArmRun {
    pub fn new(arm: Arm, turns_total: usize, max_points: u32) -> Self {
        Self {
            arm,
            turns_total,
            turns_finished: 0,
            max_points,
            failure: None,
            data_dir: None,
            scores: Vec::new(),
            submissions: Vec::new(),
            readings: Vec::new(),
            waits: Vec::new(),
            rows: Vec::new(),
            attempts: Vec::new(),
            rate: None,
        }
    }

    pub fn finished(&self) -> bool {
        self.failure.is_none() && self.turns_finished == self.turns_total
    }

    pub fn score(&self) -> u32 {
        self.scores.iter().map(TurnScore::points).sum()
    }
}

fn is_codex(harness: &str) -> bool {
    harness == "codex"
}

/// The sums of the ledger rows, as they are.
pub(super) fn sum_rows(rows: &[LedgerRow]) -> Sums {
    rows.iter().fold(Sums::default(), |sums, row| Sums {
        uncached_input: sums.uncached_input + row.uncached_input,
        cache_write: sums.cache_write + row.cache_write,
        cache_read: sums.cache_read + row.cache_read,
        output: sums.output + row.output,
    })
}

/// The Codex figure for the compactions of one arm.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Bound {
    /// `trigger_used_tokens` of each compacted attempt, one time each.
    pub tokens: i64,
    /// The compacted attempts with no trigger reading.
    pub unobserved: usize,
}

pub(super) fn codex_bound(attempts: &[AttemptRow]) -> Bound {
    attempts.iter().filter(|attempt| attempt.compacted()).fold(Bound::default(), |bound, attempt| {
        match (attempt.trigger_reading_id, attempt.trigger_used_tokens) {
            (Some(_), Some(tokens)) => Bound { tokens: bound.tokens + tokens, ..bound },
            _ => Bound { unobserved: bound.unobserved + 1, ..bound },
        }
    })
}

/// The Claude compaction requests with no ledger row of tokens above zero
/// between the dispatch and the next scripted send. Each dispatched attempt
/// counts, with any outcome, because each one sent a request. A compacted
/// attempt counts also with no dispatch time. The wait after a turn lets the
/// compaction turn end before the next send.
pub(super) fn claude_unobserved(attempts: &[AttemptRow], submissions: &[DateTime<Utc>], rows: &[LedgerRow]) -> usize {
    attempts
        .iter()
        .filter(|attempt| attempt.dispatched_at.is_some() || attempt.compacted())
        .filter(|attempt| {
            let Some(start) = attempt.dispatched_at.or(attempt.created_at) else {
                return true;
            };
            let end = submissions.iter().filter(|sent| **sent > start).min();
            !rows.iter().any(|row| {
                row.tokens() > 0
                    && row.created_at.is_some_and(|at| at >= start && end.is_none_or(|end| at < *end))
            })
        })
        .count()
}

/// A labelled side figure. It is not part of the bar.
pub(super) fn price_text(rate: Option<&ModelRate>, sums: &Sums, bound: i64) -> String {
    let Some(rate) = rate else {
        return "no rate".into();
    };
    let usage = TokenUsage {
        uncached_input_tokens: Some(sums.uncached_input + bound),
        cache_write_tokens: Some(sums.cache_write),
        cache_read_tokens: Some(sums.cache_read),
        output_tokens: Some(sums.output),
        ..TokenUsage::default()
    };
    let micro = cost_microusd(rate, &usage);
    format!("${}.{:04}", micro / 1_000_000, (micro % 1_000_000) / 100)
}

/// The figures of one arm, as the bar and the report use them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ArmFigures {
    pub sums: Sums,
    /// The Codex bound. `None` on Claude, which reports the real share.
    pub bound: Option<i64>,
    /// `sums.input()`, plus the bound on Codex.
    pub input_total: i64,
    pub score: u32,
    pub compacted: usize,
    /// The compaction requests with no observed cost.
    pub unobserved: usize,
    pub price: String,
}

pub(super) fn figures(harness: &str, run: &ArmRun) -> ArmFigures {
    let sums = sum_rows(&run.rows);
    let (bound, unobserved) = if is_codex(harness) {
        let bound = codex_bound(&run.attempts);
        (Some(bound.tokens), bound.unobserved)
    } else {
        (None, claude_unobserved(&run.attempts, &run.submissions, &run.rows))
    };
    let extra = bound.unwrap_or(0);
    ArmFigures {
        sums,
        bound,
        input_total: sums.input() + extra,
        score: run.score(),
        compacted: run.attempts.iter().filter(|attempt| attempt.compacted()).count(),
        unobserved,
        price: price_text(run.rate.as_ref(), &sums, extra),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Verdict {
    /// False when one arm did not run, so the bar cannot compare.
    pub judged: bool,
    pub failures: Vec<String>,
}

impl Verdict {
    pub fn passed(&self) -> bool {
        self.judged && self.failures.is_empty()
    }

    fn label(&self) -> &'static str {
        match (self.judged, self.failures.is_empty()) {
            (true, true) => "PASS",
            (_, false) => "FAIL",
            (false, true) => "NOT JUDGED",
        }
    }
}

/// The pass bar of one harness. An arm that did not finish is a failure
/// also when the other arm did not run.
pub(super) fn verdict(harness: &str, runs: &[ArmRun]) -> Verdict {
    let mut failures: Vec<String> = runs
        .iter()
        .filter(|run| !run.finished())
        .map(|run| {
            let why = run.failure.as_deref().map(|failure| format!(": {failure}")).unwrap_or_default();
            format!("arm {} finished {} of {} turns{why}", run.arm.as_str(), run.turns_finished, run.turns_total)
        })
        .collect();
    let on = runs.iter().find(|run| run.arm == Arm::On);
    let off = runs.iter().find(|run| run.arm == Arm::Off);
    let (Some(on), Some(off)) = (on, off) else {
        return Verdict { judged: false, failures };
    };
    let (on_figures, off_figures) = (figures(harness, on), figures(harness, off));
    failures.extend(compaction_failures(harness, on, &on_figures));
    failures.extend(comparison_failures(&on_figures, &off_figures));
    Verdict { judged: true, failures }
}

fn compaction_failures(harness: &str, on: &ArmRun, figures: &ArmFigures) -> Vec<String> {
    let mut failures = Vec::new();
    let compacted: Vec<&AttemptRow> = on.attempts.iter().filter(|attempt| attempt.compacted()).collect();
    if compacted.is_empty() {
        failures.push("arm on has no compacted attempt".to_owned());
    } else if !is_codex(harness) && !compacted.iter().any(|attempt| attempt.boundary_pre_tokens.is_some()) {
        failures.push("no compacted attempt of arm on has boundary_pre_tokens".to_owned());
    }
    if figures.unobserved > 0 {
        let what = if is_codex(harness) {
            "compacted attempts of arm on have no trigger reading for the bound"
        } else {
            "dispatched compaction requests of arm on, with any outcome, have no ledger row with tokens above zero before the next send"
        };
        failures.push(format!("{UNOBSERVED}: {} {what}", figures.unobserved));
    }
    failures
}

fn comparison_failures(on: &ArmFigures, off: &ArmFigures) -> Vec<String> {
    let mut failures = Vec::new();
    if on.score + SCORE_SLACK < off.score {
        failures.push(format!("arm on scored {}, more than {SCORE_SLACK} below arm off ({})", on.score, off.score));
    }
    if off.score < OFF_FLOOR {
        failures.push(format!("arm off scored {}, below {OFF_FLOOR}", off.score));
    }
    if on.input_total >= off.input_total {
        failures.push(format!(
            "the input total of arm on ({}) is not below the input total of arm off ({})",
            on.input_total, off.input_total
        ));
    }
    failures
}

/// The runs and the verdict of one harness.
pub(super) struct HarnessReport {
    pub harness: String,
    pub model: String,
    pub runs: Vec<ArmRun>,
    pub verdict: Verdict,
}

fn optional(value: Option<i64>) -> String {
    value.map_or_else(|| "none".to_owned(), |value| value.to_string())
}

fn reading(id: Option<i64>, tokens: Option<i64>) -> String {
    match (id, tokens) {
        (Some(id), Some(tokens)) => format!("#{id}: {tokens}"),
        (None, None) => "none".to_owned(),
        (id, tokens) => format!("#{}: {}", optional(id), optional(tokens)),
    }
}

/// The report as Markdown.
pub(super) fn markdown(started: DateTime<Utc>, reports: &[HarnessReport]) -> String {
    let mut text = format!(
        "# Early compaction eval\n\nStarted at {}. THRESHOLD_TOKENS {THRESHOLD_TOKENS}, IDLE_DELAY {} s, WARM_LIMIT {} s, SETTLE_LIMIT {} s.\n\n",
        started.to_rfc3339(),
        IDLE_DELAY.as_secs(),
        WARM_LIMIT.as_secs(),
        SETTLE_LIMIT.as_secs(),
    );
    text.push_str(&format!("Noise: {NOISE}\n\n"));
    text.push_str(
        "The input total is uncached input plus cache write plus cache read. On Codex, the input total \
includes the compaction bound. The price estimate comes from usage_pricing. It is a side figure and not part of the pass bar.\n",
    );
    for report in reports {
        text.push_str(&harness_markdown(report));
    }
    text
}

fn harness_markdown(report: &HarnessReport) -> String {
    let mut text = format!("\n## {}, model {}: {}\n\n", report.harness, report.model, report.verdict.label());
    for failure in &report.verdict.failures {
        text.push_str(&format!("- {failure}\n"));
    }
    if !report.verdict.failures.is_empty() {
        text.push('\n');
    }
    text.push_str(&figures_table(report));
    text.push_str(&rubric_table(report));
    text.push_str(&attempts_table(report));
    text.push_str(&windows_table(report));
    text
}

fn figures_table(report: &HarnessReport) -> String {
    let mut text = format!(
        "| arm | turns | score | uncached input | cache write | cache read | output | {BOUND_LABEL} | input total | price estimate |\n\
         | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n"
    );
    for run in &report.runs {
        let figures = figures(&report.harness, run);
        text.push_str(&format!(
            "| {} | {} of {} | {} of {} | {} | {} | {} | {} | {} | {} | {} |\n",
            run.arm.as_str(),
            run.turns_finished,
            run.turns_total,
            figures.score,
            run.max_points,
            figures.sums.uncached_input,
            figures.sums.cache_write,
            figures.sums.cache_read,
            figures.sums.output,
            figures.bound.map_or_else(|| "n/a".to_owned(), |bound| bound.to_string()),
            figures.input_total,
            figures.price,
        ));
    }
    for run in &report.runs {
        if let Some(dir) = &run.data_dir {
            text.push_str(&format!("\nThe data directory of arm {} is {dir}.\n", run.arm.as_str()));
        }
    }
    text
}

fn rubric_table(report: &HarnessReport) -> String {
    let mut text = "\n### Rubric of turns 11 to 14\n\n| arm | turn | R1 check | R2 decisions | points | check output |\n| --- | --- | --- | --- | --- | --- |\n".to_owned();
    for run in &report.runs {
        for score in &run.scores {
            let kept: Vec<String> = score
                .kept
                .iter()
                .map(|(name, kept)| format!("{name} {}", if *kept { "kept" } else { "lost" }))
                .collect();
            text.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                run.arm.as_str(),
                score.turn,
                if score.check_passed { "pass" } else { "fail" },
                kept.join(", "),
                score.points(),
                score.check_output.replace('|', "/").replace('\n', " "),
            ));
        }
    }
    text
}

fn attempts_table(report: &HarnessReport) -> String {
    let mut text = "\n### Attempts\n\n".to_owned();
    if report.runs.iter().all(|run| run.attempts.is_empty()) {
        text.push_str("No attempt rows.\n");
        return text;
    }
    text.push_str(
        "| arm | outcome | trigger reading | boundary_pre_tokens | boundary_post_tokens | next reading | error |\n\
         | --- | --- | --- | --- | --- | --- | --- |\n",
    );
    for run in &report.runs {
        for attempt in &run.attempts {
            text.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                run.arm.as_str(),
                attempt.outcome.as_deref().unwrap_or("open"),
                reading(attempt.trigger_reading_id, attempt.trigger_used_tokens),
                optional(attempt.boundary_pre_tokens),
                optional(attempt.boundary_post_tokens),
                reading(attempt.next_reading_id, attempt.next_used_tokens),
                attempt.error.as_deref().unwrap_or(""),
            ));
        }
    }
    text
}

fn windows_table(report: &HarnessReport) -> String {
    let mut text = "\n### Window after each turn\n\n| turn |".to_owned();
    for run in &report.runs {
        text.push_str(&format!(" window {arm} | wait {arm} |", arm = run.arm.as_str()));
    }
    text.push_str(&format!("\n| --- |{}\n", " --- | --- |".repeat(report.runs.len())));
    let turns = report.runs.iter().map(|run| run.readings.len().max(run.waits.len())).max().unwrap_or(0);
    for turn in 0..turns {
        text.push_str(&format!("| {} |", turn + 1));
        for run in &report.runs {
            let window = run.readings.get(turn).copied().flatten();
            let wait = run.waits.get(turn).map_or("", |wait| wait.as_str());
            text.push_str(&format!(" {} | {wait} |", optional(window)));
        }
        text.push('\n');
    }
    text
}

/// The report as JSON: the raw figures of each arm, the rubric points, the
/// attempt rows and the verdict.
pub(super) fn json_report(started: DateTime<Utc>, reports: &[HarnessReport]) -> Value {
    let harnesses: Vec<Value> = reports
        .iter()
        .map(|report| {
            let arms: Vec<Value> = report
                .runs
                .iter()
                .map(|run| json!({"figures": figures(&report.harness, run), "run": run}))
                .collect();
            json!({
                "harness": report.harness,
                "model": report.model,
                "verdict": {"label": report.verdict.label(), "judged": report.verdict.judged, "failures": report.verdict.failures},
                "arms": arms,
            })
        })
        .collect();
    json!({
        "startedAt": started.to_rfc3339(),
        "constants": {
            "thresholdTokens": THRESHOLD_TOKENS,
            "idleDelaySeconds": IDLE_DELAY.as_secs(),
            "warmLimitSeconds": WARM_LIMIT.as_secs(),
            "settleLimitSeconds": SETTLE_LIMIT.as_secs(),
        },
        "noise": NOISE,
        "boundLabel": BOUND_LABEL,
        "harnesses": harnesses,
    })
}

/// Write the Markdown file and the JSON file into `dir`. Returns their paths.
pub(super) fn write_report(dir: &Path, started: DateTime<Utc>, reports: &[HarnessReport]) -> Result<(PathBuf, PathBuf), String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let stem = format!("early-compaction-{}", started.format("%Y%m%dT%H%M%SZ"));
    let markdown_path = dir.join(format!("{stem}.md"));
    let json_path = dir.join(format!("{stem}.json"));
    let json_text = serde_json::to_string_pretty(&json_report(started, reports)).map_err(|error| error.to_string())?;
    std::fs::write(&markdown_path, markdown(started, reports)).map_err(|error| error.to_string())?;
    std::fs::write(&json_path, json_text).map_err(|error| error.to_string())?;
    Ok((markdown_path, json_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> Option<DateTime<Utc>> {
        DateTime::from_timestamp(1_800_000_000 + seconds, 0)
    }

    fn row(seconds: i64, uncached_input: i64, cache_write: i64, cache_read: i64, output: i64) -> LedgerRow {
        LedgerRow { created_at: at(seconds), uncached_input, cache_write, cache_read, output }
    }

    fn compacted(dispatched: i64, trigger: Option<i64>, pre: Option<i64>) -> AttemptRow {
        AttemptRow {
            outcome: Some(COMPACTED.into()),
            created_at: at(dispatched - 1),
            dispatched_at: at(dispatched),
            settled_at: at(dispatched + 30),
            trigger_reading_id: trigger.map(|_| 7),
            trigger_used_tokens: trigger,
            boundary_pre_tokens: pre,
            boundary_post_tokens: pre.map(|pre| pre / 10),
            next_reading_id: Some(9),
            next_used_tokens: Some(45_000),
            error: None,
        }
    }

    fn turn_score(turn: usize, check_passed: bool, kept: bool) -> TurnScore {
        let kept = ["header", "rounding", "errors"].map(|name| (name.to_owned(), kept)).to_vec();
        TurnScore { turn, check_passed, kept, check_output: String::new() }
    }

    /// An arm that finished each turn and scored `points` (0 to 8).
    fn arm(arm: Arm, points: u32, rows: Vec<LedgerRow>, attempts: Vec<AttemptRow>) -> ArmRun {
        let mut run = ArmRun::new(arm, 14, 8);
        run.turns_finished = 14;
        run.scores = (0..4u32)
            .map(|index| turn_score(11 + index as usize, points > index * 2, points > index * 2 + 1))
            .collect();
        run.submissions = (0..14).filter_map(|turn| at(turn * 100)).collect();
        run.rows = rows;
        run.attempts = attempts;
        run
    }

    /// A Claude pair that passes: one compaction at 820 s, with its own
    /// ledger row, and fewer input tokens in arm on.
    fn claude_pair() -> Vec<ArmRun> {
        let on = arm(
            Arm::On,
            8,
            vec![row(810, 10, 1_000, 50_000, 300), row(840, 5, 40_000, 100_000, 4_000), row(910, 10, 900, 45_000, 200)],
            vec![compacted(820, Some(112_000), Some(111_500))],
        );
        let off = arm(Arm::Off, 7, vec![row(810, 10, 1_000, 50_000, 300), row(910, 10, 1_200, 260_000, 200)], Vec::new());
        vec![on, off]
    }

    #[test]
    fn sums_add_each_ledger_row_as_it_is() {
        let sums = sum_rows(&[row(1, 1, 2, 3, 4), row(2, 10, 20, 30, 40)]);
        assert_eq!(sums, Sums { uncached_input: 11, cache_write: 22, cache_read: 33, output: 44 });
        assert_eq!(sums.input(), 66);
        assert_eq!(sum_rows(&[]), Sums::default());
    }

    #[test]
    fn the_codex_bound_adds_each_compacted_trigger_one_time() {
        let mut ended = compacted(500, Some(90_000), None);
        ended.outcome = Some("ended_without_boundary".into());
        let bound = codex_bound(&[compacted(100, Some(110_000), None), ended, compacted(900, Some(105_000), None)]);
        assert_eq!(bound, Bound { tokens: 215_000, unobserved: 0 });
        let blind = codex_bound(&[compacted(100, None, None)]);
        assert_eq!(blind, Bound { tokens: 0, unobserved: 1 });
    }

    #[test]
    fn a_claude_compaction_needs_a_ledger_row_with_tokens_before_the_next_send() {
        let attempts = [compacted(820, Some(112_000), Some(111_500))];
        let sends: Vec<DateTime<Utc>> = [0, 900].iter().filter_map(|seconds| at(*seconds)).collect();
        assert_eq!(claude_unobserved(&attempts, &sends, &[row(830, 0, 40_000, 100_000, 4_000)]), 0);
        // A row before the dispatch, a row after the next send and an empty
        // row do not show the cost of the compaction.
        let elsewhere = [row(810, 1, 1, 1, 1), row(905, 1, 1, 1, 1), row(830, 0, 0, 0, 0)];
        assert_eq!(claude_unobserved(&attempts, &sends, &elsewhere), 1);
        // With no later send, the window has no end.
        assert_eq!(claude_unobserved(&attempts, &sends[..1], &[row(5_000, 0, 1, 0, 0)]), 0);
    }

    #[test]
    fn each_dispatched_claude_attempt_needs_a_ledger_row_whatever_its_outcome() {
        let mut runs = claude_pair();
        if let [on, _] = runs.as_mut_slice() {
            // The compaction at 820 s has its row at 840 s. This request ends
            // with no boundary and has no row before the send at 1100 s.
            let mut ended = compacted(1_020, Some(60_000), None);
            ended.outcome = Some("ended_without_boundary".into());
            on.attempts.push(ended);
        }
        let figures_on = figures("claude", &runs[0]);
        assert_eq!((figures_on.compacted, figures_on.unobserved), (1, 1));
        let failures = verdict("claude", &runs).failures.join("\n");
        assert!(failures.contains(&format!("{UNOBSERVED}: 1 ")), "{failures}");
        // An attempt that the host never dispatched sent no request.
        let mut undispatched = claude_pair();
        if let [on, _] = undispatched.as_mut_slice() {
            let mut skipped = compacted(1_020, Some(60_000), None);
            skipped.outcome = Some("unsupported".into());
            skipped.dispatched_at = None;
            on.attempts.push(skipped);
        }
        assert!(verdict("claude", &undispatched).passed());
    }

    #[test]
    fn the_bar_passes_a_claude_pair_that_compacts_and_saves() {
        let verdict = verdict("claude", &claude_pair());
        assert!(verdict.passed(), "{verdict:?}");
    }

    #[test]
    fn the_bar_names_each_failure() {
        let mut runs = claude_pair();
        if let [on, off] = runs.as_mut_slice() {
            on.turns_finished = 12;
            on.failure = Some("turn 13 did not end".into());
            on.attempts = vec![compacted(820, Some(112_000), None)];
            on.rows = vec![row(810, 10, 1_000, 500_000, 300)];
            on.scores.iter_mut().for_each(|score| score.check_passed = false);
            on.scores.iter_mut().take(1).for_each(|score| score.kept.iter_mut().for_each(|kept| kept.1 = false));
            off.scores.iter_mut().take(2).for_each(|score| score.check_passed = false);
        }
        let verdict = verdict("claude", &runs);
        let text = verdict.failures.join("\n");
        assert!(!verdict.passed());
        for part in [
            "arm on finished 12 of 14 turns: turn 13 did not end",
            "no compacted attempt of arm on has boundary_pre_tokens",
            UNOBSERVED,
            "arm on scored 3, more than 1 below arm off (5)",
            "arm off scored 5, below 6",
            "is not below the input total of arm off",
        ] {
            assert!(text.contains(part), "{part} is not in {text}");
        }
    }

    #[test]
    fn the_codex_bar_counts_the_bound_and_needs_a_trigger_reading() {
        let on = arm(Arm::On, 8, vec![row(810, 100_000, 0, 300_000, 900)], vec![compacted(820, Some(150_000), None)]);
        let off = arm(Arm::Off, 8, vec![row(810, 100_000, 0, 500_000, 900)], Vec::new());
        let figures_on = figures("codex", &on);
        assert_eq!(figures_on.bound, Some(150_000));
        assert_eq!(figures_on.input_total, 550_000);
        assert!(verdict("codex", &[on.clone(), off.clone()]).passed());
        // The bound can tip the comparison.
        let mut heavy = on.clone();
        heavy.attempts = vec![compacted(820, Some(250_000), None)];
        assert!(!verdict("codex", &[heavy, off.clone()]).passed());
        let mut blind = on;
        blind.attempts = vec![compacted(820, None, None)];
        let failures = verdict("codex", &[blind, off]).failures.join("\n");
        assert!(failures.contains(UNOBSERVED), "{failures}");
    }

    #[test]
    fn one_arm_is_not_judged_but_an_unfinished_arm_still_fails() {
        let runs = claude_pair();
        let alone = verdict("claude", &runs[..1]);
        assert!(!alone.judged && alone.failures.is_empty() && !alone.passed());
        let mut broken = runs[..1].to_vec();
        broken[0].failure = Some("the chat did not start".into());
        assert_eq!(verdict("claude", &broken).failures.len(), 1);
    }

    #[test]
    fn the_price_is_a_side_figure_or_no_rate() {
        let sums = Sums { uncached_input: 1_000_000, cache_write: 0, cache_read: 2_000_000, output: 100_000 };
        assert_eq!(price_text(None, &sums, 0), "no rate");
        let rate = ModelRate { input: 100_000, output: 500_000, cache_read: 10_000, cache_write: 125_000 };
        // 0.10 + 0.02 + 0.05 dollars.
        assert_eq!(price_text(Some(&rate), &sums, 0), "$0.1700");
        assert_eq!(price_text(Some(&rate), &sums, 1_000_000), "$0.2700");
    }

    #[test]
    fn the_report_carries_the_figures_the_noise_and_the_bound_column() -> Result<(), String> {
        let runs = claude_pair();
        let verdict = verdict("claude", &runs);
        let reports = [HarnessReport { harness: "claude".into(), model: "claude-haiku-5-5".into(), runs, verdict }];
        let started = at(0).ok_or("no time")?;
        let text = markdown(started, &reports);
        for part in [NOISE, BOUND_LABEL, "about 10,000 tokens", "## claude, model claude-haiku-5-5: PASS", "| on | 14 of 14 | 8 of 8 |", "| on | compacted | #7: 112000 | 111500 | 11150 | #9: 45000 |", "no rate", "| 11 |"] {
            if !text.contains(part) {
                return Err(format!("{part:?} is not in the report:\n{text}"));
            }
        }
        let json = json_report(started, &reports);
        let arm = &json["harnesses"][0]["arms"][0];
        if json["boundLabel"] != BOUND_LABEL || json["noise"] != NOISE || json["harnesses"][0]["verdict"]["label"] != "PASS" {
            return Err(format!("the JSON report lacks a field: {json}"));
        }
        if arm["figures"]["sums"]["cacheRead"] != 195_000 || arm["run"]["attempts"][0]["boundaryPreTokens"] != 111_500 || arm["run"]["scores"][0]["checkPassed"] != true {
            return Err(format!("the JSON report lacks a raw figure: {arm}"));
        }
        let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
        let (markdown_path, json_path) = write_report(dir.path(), started, &reports)?;
        if !markdown_path.is_file() || !json_path.is_file() {
            return Err("the report files are missing".into());
        }
        Ok(())
    }
}
