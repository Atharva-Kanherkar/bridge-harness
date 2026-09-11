//! Usage insights: one headless harness turn that reads what Bridge already
//! knows about the user's coding — token usage by harness and day, when they
//! prompt, what they prompt about, and the state of their pull requests — and
//! writes the narrative for the Insights tab.
//!
//! The division of labour is strict. Every number the tab charts is computed
//! here from Bridge's own ledgers and handed to the model as context; the model
//! contributes prose only (headline, summary, highlights, themes,
//! recommendations), returned as one fenced JSON object and validated before
//! it is stored. A model that hallucinates a figure cannot put it on a chart.
//!
//! The run is a hidden `briefing`-kind session on the user's harness, started
//! under a scoped briefing policy with no servers in scope — so it has no
//! tools at all, and prompt text never leaves the machine except to the
//! provider the user already sends prompts to. Prompts are truncated and
//! sampled; the report stores none of them.

use std::io::BufRead;
use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bridge_protocol::messages as wire;
use chrono::{Datelike, Local, TimeZone, Timelike, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::adapters::{ShutdownReason, StartRequest};
use crate::briefing_policy::BriefingRuntimePolicy;
use crate::usage_summary::{self, UsageResolution, UsageSummaryRequest};
use crate::model::AdapterDescriptor;
use crate::work_briefing_config::{resolve_briefing, BriefingSelection, BriefingUnavailable, BRIEFING_SESSION_KIND};
use crate::{work, BridgeCore, BridgeError};

/// Longest a run may take before it is abandoned as failed.
const MAX_WALL_SECONDS: i64 = 180;
/// Prompts shown to the model, newest first. Enough for themes, few enough to
/// keep the turn small.
const PROMPT_SAMPLE: usize = 60;
/// Characters of each prompt the model sees.
const PROMPT_CHARS: usize = 280;
/// Workspaces asked about on GitHub. Each is one `gh` call.
const GITHUB_WORKSPACES: usize = 5;
const FENCE: &str = "```";

/// A prompt row from the session forest, reduced to what the analysis needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSample {
    pub harness: String,
    pub text: String,
    pub created_at: String,
}

/// Everything Bridge computes before the model is asked.
#[derive(Debug, Clone)]
pub struct InsightInput {
    pub window_days: i64,
    pub since_day: String,
    pub until_day: String,
    pub harnesses: Vec<wire::UsageInsightHarness>,
    pub hours: Vec<wire::UsageInsightHour>,
    pub days: Vec<wire::UsageInsightDay>,
    pub github: Option<wire::UsageInsightGithub>,
    pub prompts: Vec<PromptSample>,
}

/// The prose the model owns. Deserialized from its fenced JSON, then bounded.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProse {
    pub headline: String,
    pub summary: String,
    #[serde(default)]
    pub highlights: Vec<ProseHighlight>,
    #[serde(default)]
    pub themes: Vec<ProseTheme>,
    #[serde(default)]
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProseHighlight {
    pub title: String,
    pub detail: String,
    #[serde(default)]
    pub tone: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProseTheme {
    pub label: String,
    pub share: f64,
    #[serde(default)]
    pub example: Option<String>,
}

pub fn clamp_window(days: i64) -> i64 {
    days.clamp(1, 90)
}

/// The `usage_insights` table: one row, the latest report.
pub fn install_store(db: &Connection) -> Result<(), BridgeError> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage_insights (
            id INTEGER PRIMARY KEY CHECK(id = 1),
            window_days INTEGER NOT NULL,
            generated_at TEXT NOT NULL,
            harness TEXT NOT NULL,
            model TEXT NOT NULL,
            report TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// The stored report, when one exists.
pub fn latest(db: &Connection) -> Result<Option<wire::UsageInsightsResult>, BridgeError> {
    let row = db
        .query_row(
            "SELECT window_days, generated_at, harness, model, report FROM usage_insights WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    Ok(row.and_then(|(window_days, generated_at, harness, model, report)| {
        let report: wire::UsageInsightsReport = serde_json::from_str(&report).ok()?;
        Some(wire::UsageInsightsResult {
            status: wire::UsageInsightsStatus::Ready,
            window_days,
            generated_at: Some(generated_at),
            harness: Some(harness),
            model: Some(model),
            report: Some(report),
            detail: None,
        })
    }))
}

fn store(db: &Connection, result: &wire::UsageInsightsResult) -> Result<(), BridgeError> {
    let Some(report) = &result.report else {
        return Ok(());
    };
    db.execute(
        "INSERT INTO usage_insights(id, window_days, generated_at, harness, model, report)
         VALUES(1, ?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET window_days=excluded.window_days, generated_at=excluded.generated_at,
             harness=excluded.harness, model=excluded.model, report=excluded.report",
        params![
            result.window_days,
            result.generated_at.as_deref().unwrap_or_default(),
            result.harness.as_deref().unwrap_or_default(),
            result.model.as_deref().unwrap_or_default(),
            serde_json::to_string(report).map_err(|error| BridgeError::Invalid(error.to_string()))?,
        ],
    )?;
    Ok(())
}

/// The entry point behind `usage/insights`. Without `refresh` this only reads:
/// the stored report, or `empty`. With it, the harness is run.
pub fn insights(core: &Arc<BridgeCore>, params: &wire::InsightsParams) -> Result<wire::UsageInsightsResult, BridgeError> {
    let window_days = clamp_window(params.window_days);
    if !params.refresh {
        // Opening the tab must never start a harness turn: that sends sampled
        // prompts to a provider and can cost money. Without a stored report
        // the answer is `empty`, and the explicit Analyse action asks again
        // with `refresh`.
        return Ok(latest(&core.db.lock().unwrap())?.unwrap_or(empty(window_days)));
    }
    let input = gather(core, window_days)?;
    let result = match run(core, &input) {
        Ok(result) => result,
        Err(failure) => failure,
    };
    if result.report.is_some() {
        store(&core.db.lock().unwrap(), &result)?;
    }
    Ok(result)
}

/// Compute every figure the report carries. Pure reads; no model involved.
pub fn gather(core: &Arc<BridgeCore>, window_days: i64) -> Result<InsightInput, BridgeError> {
    let today = Local::now().date_naive();
    let since = today - chrono::Duration::days(window_days - 1);
    let since_day = since.format("%Y-%m-%d").to_string();
    let until_day = today.format("%Y-%m-%d").to_string();
    let time_zone = iana_zone();
    let (summary, prompts) = {
        let db = core.db.lock().unwrap();
        let summary = usage_summary::summarize(
            &db,
            &UsageSummaryRequest {
                since_day: since_day.clone(),
                until_day: until_day.clone(),
                resolution: UsageResolution::Day,
                time_zone: time_zone.clone(),
                workspace_id: None,
                include_imported: true,
                since_time: None,
                until_time: None,
            },
        )?;
        let since_utc = Local
            .from_local_datetime(&since.and_hms_opt(0, 0, 0).expect("midnight"))
            .single()
            .map(|at| at.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);
        let prompts = prompt_rows(&db, &since_utc.to_rfc3339())?;
        (summary, prompts)
    };
    let github = github_overview(core);
    Ok(build_input(window_days, since_day, until_day, &summary.buckets, prompts, github))
}

/// Fold buckets and prompt rows into the report's chart data. Separate from
/// `gather` so the arithmetic is testable without a database.
pub fn build_input(
    window_days: i64,
    since_day: String,
    until_day: String,
    buckets: &[usage_summary::UsageBucket],
    prompts: Vec<PromptSample>,
    github: Option<wire::UsageInsightGithub>,
) -> InsightInput {
    use std::collections::BTreeMap;
    let mut by_harness: BTreeMap<String, wire::UsageInsightHarness> = BTreeMap::new();
    // Every date in the window starts at zero, so a quiet stretch is drawn as a
    // quiet stretch and read by the model as one, not interpolated away.
    let mut by_day: BTreeMap<String, wire::UsageInsightDay> = BTreeMap::new();
    if let (Ok(first), Ok(last)) = (
        chrono::NaiveDate::parse_from_str(&since_day, "%Y-%m-%d"),
        chrono::NaiveDate::parse_from_str(&until_day, "%Y-%m-%d"),
    ) {
        let mut cursor = first;
        while cursor <= last {
            let day = cursor.format("%Y-%m-%d").to_string();
            by_day.insert(day.clone(), wire::UsageInsightDay { day, processed_tokens: 0, prompts: 0 });
            let Some(next) = cursor.succ_opt() else { break };
            cursor = next;
        }
    }
    for bucket in buckets {
        let tokens = bucket.totals.uncached_input_tokens
            + bucket.totals.cache_read_tokens
            + bucket.totals.cache_write_tokens
            + bucket.totals.output_tokens;
        let entry = by_harness.entry(bucket.harness.clone()).or_insert_with(|| wire::UsageInsightHarness {
            harness: bucket.harness.clone(),
            processed_tokens: 0,
            cost_microusd: 0,
            records: 0,
            sessions: 0,
            prompts: 0,
        });
        entry.processed_tokens += tokens;
        entry.cost_microusd += bucket.cost_microusd;
        entry.records += bucket.records;
        entry.sessions += bucket.sessions;
        let day = by_day.entry(bucket.day.clone()).or_insert_with(|| wire::UsageInsightDay {
            day: bucket.day.clone(),
            processed_tokens: 0,
            prompts: 0,
        });
        day.processed_tokens += tokens;
    }
    let mut hours: Vec<wire::UsageInsightHour> = (0..24).map(|hour| wire::UsageInsightHour { hour, prompts: 0 }).collect();
    for prompt in &prompts {
        if let Ok(at) = chrono::DateTime::parse_from_rfc3339(&prompt.created_at) {
            let local = at.with_timezone(&Local);
            hours[local.hour() as usize].prompts += 1;
            let day = format!("{:04}-{:02}-{:02}", local.year(), local.month(), local.day());
            by_day
                .entry(day.clone())
                .or_insert_with(|| wire::UsageInsightDay { day, processed_tokens: 0, prompts: 0 })
                .prompts += 1;
        }
        by_harness
            .entry(prompt.harness.clone())
            .or_insert_with(|| wire::UsageInsightHarness {
                harness: prompt.harness.clone(),
                processed_tokens: 0,
                cost_microusd: 0,
                records: 0,
                sessions: 0,
                prompts: 0,
            })
            .prompts += 1;
    }
    let mut harnesses: Vec<_> = by_harness.into_values().collect();
    harnesses.sort_by(|a, b| b.processed_tokens.cmp(&a.processed_tokens).then_with(|| a.harness.cmp(&b.harness)));
    InsightInput {
        window_days,
        since_day,
        until_day,
        harnesses,
        hours,
        days: by_day.into_values().collect(),
        github,
        prompts,
    }
}

fn iana_zone() -> Option<String> {
    std::env::var("TZ").ok().filter(|zone| !zone.is_empty()).or_else(|| {
        // macOS keeps the zone as a symlink; its target names the IANA id.
        std::fs::read_link("/etc/localtime").ok().and_then(|target| {
            let text = target.to_string_lossy();
            text.split("zoneinfo/").nth(1).map(str::to_owned)
        })
    })
}

/// Visible user prompts since `since_rfc3339`, newest first, hidden sessions
/// excluded so a briefing's own task text never counts as the user's.
fn prompt_rows(db: &Connection, since_rfc3339: &str) -> Result<Vec<PromptSample>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT s.harness, e.payload, e.created_at
         FROM session_entries e JOIN sessions s ON s.id = e.session_id
         WHERE e.kind = 'user.message' AND e.created_at >= ?1
           AND (s.kind IS NULL OR s.kind IN ('direct', 'orchestrator'))
         ORDER BY e.created_at DESC
         LIMIT 2000",
    )?;
    let rows = statement.query_map([since_rfc3339], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    })?;
    let mut prompts = Vec::new();
    for row in rows {
        let (harness, payload, created_at) = row?;
        let text = serde_json::from_str::<Value>(&payload)
            .ok()
            .and_then(|value| value.get("text").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_default();
        let text = text.trim().to_owned();
        if text.is_empty() {
            continue;
        }
        prompts.push(PromptSample { harness, text, created_at });
    }
    Ok(prompts)
}

fn github_overview(core: &Arc<BridgeCore>) -> Option<wire::UsageInsightGithub> {
    use crate::github_surface::{GithubAvailability, PullRequestState, ReviewDecision};
    if !matches!(core.github_surface.availability(), GithubAvailability::Available) {
        return None;
    }
    let paths: Vec<String> = {
        let db = core.db.lock().unwrap();
        let mut statement = db
            .prepare("SELECT DISTINCT path FROM workspaces ORDER BY created_at DESC LIMIT ?1")
            .ok()?;
        let rows: Vec<String> = statement
            .query_map([GITHUB_WORKSPACES as i64], |row| row.get::<_, String>(0))
            .ok()?
            .flatten()
            .collect();
        rows
    };
    let mut overview = wire::UsageInsightGithub { repositories: 0, open_prs: 0, draft_prs: 0, failing_checks: 0, awaiting_review: 0 };
    let mut seen = std::collections::BTreeSet::new();
    for path in paths {
        let Ok(repository) = core.github_surface.resolve_repository(Path::new(&path)) else {
            continue;
        };
        if !seen.insert(repository.selector()) {
            continue;
        }
        let Ok(prs) = core.github_surface.list_prs(Path::new(&path)) else {
            continue;
        };
        overview.repositories += 1;
        for pr in prs.iter().filter(|pr| pr.state == PullRequestState::Open) {
            overview.open_prs += 1;
            if pr.is_draft {
                overview.draft_prs += 1;
            }
            if pr.checks.failed > 0 {
                overview.failing_checks += 1;
            }
            if matches!(pr.review_decision, ReviewDecision::ReviewRequired) {
                overview.awaiting_review += 1;
            }
        }
    }
    (overview.repositories > 0).then_some(overview)
}

/// The instructions the hidden session starts with.
fn instructions() -> String {
    "You are writing the Insights panel of Bridge, a desktop app that runs coding agents. \
You will be given a digest of one person's coding-agent usage: tokens by harness and day, \
when they send prompts, a sample of their recent prompts, and the state of their pull requests. \
Write for that person, in the second person, plainly and specifically. No emoji, no exclamation \
marks, no marketing tone, no praise for its own sake. Every claim must be grounded in the digest. \
Do not invent numbers: quote only figures that appear in the digest. \
Respond with exactly one fenced ```json block and nothing else."
        .to_owned()
}

/// The single turn: the digest plus the shape of the answer.
pub fn task_prompt(input: &InsightInput) -> String {
    let mut digest = String::new();
    digest.push_str(&format!(
        "Window: {} to {} ({} days).\n\n",
        input.since_day, input.until_day, input.window_days
    ));
    digest.push_str("Harnesses (processed tokens, estimated cost in USD, requests, sessions, prompts):\n");
    for entry in &input.harnesses {
        digest.push_str(&format!(
            "- {}: {} tokens, ${:.2}, {} requests, {} sessions, {} prompts\n",
            entry.harness,
            entry.processed_tokens,
            entry.cost_microusd as f64 / 1_000_000.0,
            entry.records,
            entry.sessions,
            entry.prompts
        ));
    }
    digest.push_str("\nProcessed tokens and prompts by day:\n");
    for day in &input.days {
        digest.push_str(&format!("- {}: {} tokens, {} prompts\n", day.day, day.processed_tokens, day.prompts));
    }
    digest.push_str("\nPrompts by local hour of day (hour: prompts):\n");
    let busy: Vec<String> = input
        .hours
        .iter()
        .filter(|hour| hour.prompts > 0)
        .map(|hour| format!("{:02}: {}", hour.hour, hour.prompts))
        .collect();
    digest.push_str(&if busy.is_empty() { "- none\n".to_owned() } else { format!("- {}\n", busy.join(", ")) });
    if let Some(github) = &input.github {
        digest.push_str(&format!(
            "\nGitHub across {} repositories: {} open PRs ({} drafts), {} with failing checks, {} awaiting review.\n",
            github.repositories, github.open_prs, github.draft_prs, github.failing_checks, github.awaiting_review
        ));
    } else {
        digest.push_str("\nGitHub: not available (gh missing or signed out).\n");
    }
    digest.push_str(&format!("\nRecent prompts, newest first (sample of {} of {}):\n", input.prompts.len().min(PROMPT_SAMPLE), input.prompts.len()));
    for prompt in input.prompts.iter().take(PROMPT_SAMPLE) {
        let mut text: String = prompt.text.chars().take(PROMPT_CHARS).collect();
        if prompt.text.chars().count() > PROMPT_CHARS {
            text.push('…');
        }
        digest.push_str(&format!("- [{}] {}\n", prompt.harness, text.replace('\n', " ")));
    }
    format!(
        "{digest}\n\
Return one fenced ```json object with exactly these keys:\n\
{{\n  \"headline\": string (at most 60 characters, the one thing worth knowing),\n  \
\"summary\": string (two or three sentences),\n  \
\"highlights\": [{{ \"title\": string (at most 40 chars), \"detail\": string (one sentence), \"tone\": \"neutral\" | \"good\" | \"watch\" }}] (3 to 5 items),\n  \
\"themes\": [{{ \"label\": string (2-4 words), \"share\": number between 0 and 1, \"example\": string (a short paraphrase, never a verbatim prompt) }}] (3 to 6 items, shares summing to about 1),\n  \
\"recommendations\": [string] (2 to 4 concrete, specific suggestions)\n}}\n"
    )
}

/// Which harness and model write the prose: the configured briefing profile,
/// else Claude at its default model when nothing is configured and Claude is
/// installed. Fail-closed like the briefing itself: a profile that exists but
/// does not resolve (uncertified version, malformed model, unsupported
/// harness) is refused, never quietly swapped for a provider the user did not
/// choose.
fn selection(core: &Arc<BridgeCore>) -> Result<(String, String, Option<String>), wire::UsageInsightsResult> {
    let descriptors = core.adapter_registry.descriptors();
    let versions = |harness: &str| {
        descriptors
            .iter()
            .find(|descriptor| descriptor.id == harness)
            .and_then(|descriptor| descriptor.version.clone())
    };
    let settings = work::read_settings(&core.db.lock().unwrap())
        .map(|snapshot| snapshot.settings)
        .map_err(|error| unavailable(format!("Work settings could not be read: {error}")))?;
    let resolved = resolve_briefing(&settings, &versions);
    let claude = descriptors.iter().find(|descriptor| descriptor.id == "claude");
    choose(resolved, claude)
}

/// The selection rule, separated from the core so it can be tested without an
/// adapter registry.
fn choose(
    resolved: Result<BriefingSelection, BriefingUnavailable>,
    claude: Option<&AdapterDescriptor>,
) -> Result<(String, String, Option<String>), wire::UsageInsightsResult> {
    match resolved {
        Ok(selection) => Ok((selection.harness, selection.model, selection.effort)),
        Err(BriefingUnavailable::NotConfigured) => match claude {
            Some(descriptor) if descriptor.available => Ok(("claude".into(), crate::claude_adapter::DEFAULT_MODEL.into(), None)),
            Some(descriptor) => Err(unavailable(descriptor.unavailable_reason.clone().unwrap_or_else(|| "Claude is not available".into()))),
            None => Err(unavailable("Insights need Claude Code installed: it is the harness that can run a read-only analysis without tools.".into())),
        },
        Err(other) => Err(unavailable(format!("The configured briefing profile cannot run Insights: {}", other.reason()))),
    }
}

fn empty(window_days: i64) -> wire::UsageInsightsResult {
    wire::UsageInsightsResult {
        status: wire::UsageInsightsStatus::Empty,
        window_days,
        generated_at: None,
        harness: None,
        model: None,
        report: None,
        detail: None,
    }
}

fn unavailable(detail: String) -> wire::UsageInsightsResult {
    wire::UsageInsightsResult {
        status: wire::UsageInsightsStatus::Unavailable,
        window_days: 0,
        generated_at: None,
        harness: None,
        model: None,
        report: None,
        detail: Some(detail),
    }
}

fn failed(window_days: i64, harness: &str, model: &str, detail: String) -> wire::UsageInsightsResult {
    wire::UsageInsightsResult {
        status: wire::UsageInsightsStatus::Failed,
        window_days,
        generated_at: None,
        harness: Some(harness.to_owned()),
        model: Some(model.to_owned()),
        report: None,
        detail: Some(detail),
    }
}

/// Run the hidden turn and assemble the report. `Err` carries the typed
/// failure so the caller returns it as a result rather than an RPC error: an
/// analysis that could not run is a state of the tab, not a broken call.
fn run(core: &Arc<BridgeCore>, input: &InsightInput) -> Result<wire::UsageInsightsResult, wire::UsageInsightsResult> {
    let (harness, model, effort) = selection(core).map_err(|mut result| {
        result.window_days = input.window_days;
        result
    })?;
    let limits = wire::WorkBriefLimits {
        max_wall_seconds: MAX_WALL_SECONDS,
        max_turns: 1,
        max_tool_calls: 1,
        max_output_tokens: None,
        cost_ceiling_microusd: None,
    };
    // No servers in scope: the compiled policy admits no tool at all.
    let policy = BriefingRuntimePolicy::compile_scoped(Vec::new(), limits)
        .map_err(|error| failed(input.window_days, &harness, &model, error.reason()))?;

    let session_id = Uuid::new_v4().to_string();
    let scratch = core.chat_scratch_dir(&session_id);
    std::fs::create_dir_all(&scratch).map_err(|error| failed(input.window_days, &harness, &model, error.to_string()))?;
    let cwd = scratch.to_string_lossy().to_string();
    {
        let db = core.db.lock().unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind,title,cwd,depth)
             VALUES(?1,NULL,?2,'Insights','working','reported',?3,?4,'Usage insights',?5,0)",
            params![session_id, harness, model, BRIEFING_SESSION_KIND, cwd],
        )
        .map_err(|error| failed(input.window_days, &harness, &model, error.to_string()))?;
    }
    let instructions = instructions();
    let started = core.adapter_registry.start(
        &harness,
        StartRequest {
            cwd: &cwd,
            model: Some(&model),
            effort: effort.as_deref(),
            instructions: Some(&instructions),
            write_mode: None,
            read_only_sandbox: None,
            briefing: Some(&policy),
            on_progress: None,
        },
    );
    let started = match started {
        Ok(started) => started,
        Err(error) => {
            settle(core, &session_id, "failed");
            return Err(failed(input.window_days, &harness, &model, error.to_string()));
        }
    };
    let mut runtime = started.runtime;
    {
        let db = core.db.lock().unwrap();
        let _ = db.execute(
            "UPDATE sessions SET provider_session_id=?2,started_at=?3 WHERE id=?1",
            params![session_id, runtime.provider_session_id(), Utc::now().to_rfc3339()],
        );
    }
    let (lines, receiver) = mpsc::channel::<String>();
    let mut reader = started.reader;
    std::thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if lines.send(line.trim_end().to_owned()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    if let Err(error) = runtime.send_turn(&task_prompt(input)) {
        runtime.stop(ShutdownReason::Failed);
        settle(core, &session_id, "failed");
        return Err(failed(input.window_days, &harness, &model, error.to_string()));
    }
    let deadline = Instant::now() + Duration::from_secs(MAX_WALL_SECONDS as u64);
    let mut text = String::new();
    let mut ended: Option<String> = None;
    loop {
        if Instant::now() >= deadline {
            ended = Some(format!("the analysis did not finish within {MAX_WALL_SECONDS}s"));
            break;
        }
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(line) => {
                if observe(&line, &mut text) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                ended = Some("the harness ended before answering".into());
                break;
            }
        }
    }
    if let Some(detail) = ended {
        runtime.stop(ShutdownReason::Failed);
        settle(core, &session_id, "failed");
        return Err(failed(input.window_days, &harness, &model, detail));
    }
    runtime.stop(ShutdownReason::Completed);
    settle(core, &session_id, "idle");
    let prose = parse_prose(&text).map_err(|detail| failed(input.window_days, &harness, &model, detail))?;
    Ok(wire::UsageInsightsResult {
        status: wire::UsageInsightsStatus::Ready,
        window_days: input.window_days,
        generated_at: Some(Utc::now().to_rfc3339()),
        harness: Some(harness),
        model: Some(model),
        report: Some(assemble(input, prose)),
        detail: None,
    })
}

/// Fold one sidecar line into the answer. Returns true when the turn is done.
fn observe(line: &str, text: &mut String) -> bool {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    match message.get("type").and_then(Value::as_str) {
        Some("assistant") => {
            let blocks = message
                .get("message")
                .and_then(|inner| inner.get("content"))
                .or_else(|| message.get("content"))
                .and_then(Value::as_array);
            if let Some(blocks) = blocks {
                for block in blocks {
                    if block.get("type").and_then(Value::as_str) == Some("text") {
                        if let Some(part) = block.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(part);
                        }
                    }
                }
            }
            false
        }
        Some("result") => {
            if text.trim().is_empty() {
                if let Some(result) = message.get("result").and_then(Value::as_str) {
                    text.push_str(result);
                }
            }
            true
        }
        _ => false,
    }
}

fn settle(core: &Arc<BridgeCore>, session_id: &str, status: &str) {
    let db = core.db.lock().unwrap();
    let _ = db.execute(
        "UPDATE sessions SET status=?2,ended_at=?3 WHERE id=?1",
        params![session_id, status, Utc::now().to_rfc3339()],
    );
}

/// The model's JSON, out of its fence (or bare), bounded to what the tab shows.
pub fn parse_prose(answer: &str) -> Result<ModelProse, String> {
    let body = fenced_json(answer).unwrap_or_else(|| answer.trim().to_owned());
    let start = body.find('{').ok_or_else(|| "the answer held no JSON object".to_owned())?;
    let end = body.rfind('}').ok_or_else(|| "the answer held no JSON object".to_owned())?;
    if end < start {
        return Err("the answer held no JSON object".into());
    }
    let mut prose: ModelProse = serde_json::from_str(&body[start..=end]).map_err(|error| format!("the answer was not the expected JSON: {error}"))?;
    prose.headline = bounded(&prose.headline, 80);
    prose.summary = bounded(&prose.summary, 600);
    if prose.headline.is_empty() || prose.summary.is_empty() {
        return Err("the answer had no headline or summary".into());
    }
    prose.highlights.truncate(5);
    prose.themes.retain(|theme| !theme.label.trim().is_empty());
    prose.themes.truncate(6);
    prose.recommendations.retain(|item| !item.trim().is_empty());
    prose.recommendations.truncate(4);
    Ok(prose)
}

fn fenced_json(answer: &str) -> Option<String> {
    let open = answer.find(FENCE)?;
    let after = &answer[open + FENCE.len()..];
    let body_start = after.find('\n')? + 1;
    let body = &after[body_start..];
    let close = body.find(FENCE)?;
    Some(body[..close].trim().to_owned())
}

fn bounded(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_owned();
    }
    let mut cut: String = trimmed.chars().take(limit.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

fn tone(raw: Option<&str>) -> wire::UsageInsightTone {
    match raw {
        Some("good") => wire::UsageInsightTone::Good,
        Some("watch") => wire::UsageInsightTone::Watch,
        _ => wire::UsageInsightTone::Neutral,
    }
}

fn assemble(input: &InsightInput, prose: ModelProse) -> wire::UsageInsightsReport {
    let total_share: f64 = prose.themes.iter().map(|theme| theme.share.clamp(0.0, 1.0)).sum();
    wire::UsageInsightsReport {
        headline: prose.headline,
        summary: prose.summary,
        highlights: prose
            .highlights
            .into_iter()
            .map(|item| wire::UsageInsightHighlight {
                title: bounded(&item.title, 48),
                detail: bounded(&item.detail, 240),
                tone: tone(item.tone.as_deref()),
            })
            .collect(),
        themes: prose
            .themes
            .into_iter()
            .map(|theme| wire::UsageInsightTheme {
                label: bounded(&theme.label, 32),
                // Normalise so the bars always add up, whatever the model summed to.
                share: if total_share > 0.0 { theme.share.clamp(0.0, 1.0) / total_share } else { 0.0 },
                example: theme.example.map(|text| bounded(&text, 120)).filter(|text| !text.is_empty()),
            })
            .collect(),
        recommendations: prose.recommendations.into_iter().map(|item| bounded(&item, 240)).collect(),
        harnesses: input.harnesses.clone(),
        hours: input.hours.clone(),
        days: input.days.clone(),
        github: input.github.clone(),
        prompts_analysed: input.prompts.len().min(PROMPT_SAMPLE) as i64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_pricing::CostSource;
    use crate::usage_summary::{UsageBucket, UsageBucketTotals};

    fn bucket(day: &str, harness: &str, tokens: i64, cost: i64) -> UsageBucket {
        UsageBucket {
            day: day.into(),
            hour_start: None,
            harness: harness.into(),
            model: "m".into(),
            totals: UsageBucketTotals {
                uncached_input_tokens: tokens / 2,
                cache_read_tokens: tokens / 4,
                cache_write_tokens: 0,
                output_tokens: tokens - tokens / 2 - tokens / 4,
                reasoning_tokens: 1,
            },
            cost_microusd: cost,
            cache_savings_microusd: 0,
            cost_source: CostSource::ModelPriced,
            records: 1,
            unpriced_records: 0,
            sessions: 1,
        }
    }

    fn prompt(harness: &str, text: &str, at: &str) -> PromptSample {
        PromptSample { harness: harness.into(), text: text.into(), created_at: at.into() }
    }

    #[test]
    fn input_folds_buckets_by_harness_and_day_and_counts_prompts_by_hour() {
        let buckets = vec![
            bucket("2026-09-08", "codex", 1_000, 300),
            bucket("2026-09-09", "codex", 2_000, 700),
            bucket("2026-09-09", "claude", 4_000, 1_500),
        ];
        let at = Utc::now().to_rfc3339();
        let prompts = vec![prompt("claude", "fix the meter", &at), prompt("claude", "add a chart", &at), prompt("cursor", "hello", &at)];
        let input = build_input(30, "2026-08-11".into(), "2026-09-09".into(), &buckets, prompts, None);
        // Reasoning tokens are inside output and are not double counted.
        let claude = input.harnesses.iter().find(|entry| entry.harness == "claude").unwrap();
        assert_eq!(claude.processed_tokens, 4_000);
        assert_eq!(claude.cost_microusd, 1_500);
        assert_eq!(claude.prompts, 2);
        let codex = input.harnesses.iter().find(|entry| entry.harness == "codex").unwrap();
        assert_eq!(codex.processed_tokens, 3_000);
        assert_eq!(codex.sessions, 2);
        // A harness that only prompted still appears, at zero tokens.
        assert!(input.harnesses.iter().any(|entry| entry.harness == "cursor" && entry.processed_tokens == 0 && entry.prompts == 1));
        // Heaviest first.
        assert_eq!(input.harnesses[0].harness, "claude");
        assert_eq!(input.hours.len(), 24);
        assert_eq!(input.hours.iter().map(|hour| hour.prompts).sum::<i64>(), 3);
        assert_eq!(input.days.iter().find(|day| day.day == "2026-09-09").unwrap().processed_tokens, 6_000);
    }

    #[test]
    fn task_prompt_carries_the_digest_and_truncates_prompts() {
        let long = "x".repeat(PROMPT_CHARS + 50);
        let input = build_input(7, "a".into(), "b".into(), &[], vec![prompt("codex", &long, &Utc::now().to_rfc3339())], Some(wire::UsageInsightGithub { repositories: 2, open_prs: 3, draft_prs: 1, failing_checks: 1, awaiting_review: 2 }));
        let text = task_prompt(&input);
        assert!(text.contains("GitHub across 2 repositories: 3 open PRs (1 drafts)"));
        assert!(text.contains("```json"));
        assert!(!text.contains(&long), "prompts must be truncated before they reach the model");
        assert!(text.contains(&format!("{}…", "x".repeat(PROMPT_CHARS))));
    }

    #[test]
    fn parse_prose_reads_a_fenced_object_and_bounds_it() {
        let answer = "Here you go.\n```json\n{\"headline\":\"Steady week\",\"summary\":\"Most work landed after lunch.\",\"highlights\":[{\"title\":\"Cache\",\"detail\":\"Two thirds cached.\",\"tone\":\"good\"}],\"themes\":[{\"label\":\"Refactors\",\"share\":0.6},{\"label\":\"Tests\",\"share\":0.6}],\"recommendations\":[\"Batch edits.\",\"\",\"Use Codex for reviews.\"]}\n```\n";
        let prose = parse_prose(answer).unwrap();
        assert_eq!(prose.headline, "Steady week");
        assert_eq!(prose.recommendations, vec!["Batch edits.", "Use Codex for reviews."]);
        let input = build_input(7, "a".into(), "b".into(), &[], vec![], None);
        let report = assemble(&input, prose);
        assert_eq!(report.highlights[0].tone, wire::UsageInsightTone::Good);
        // Shares are normalised to sum to one.
        let total: f64 = report.themes.iter().map(|theme| theme.share).sum();
        assert!((total - 1.0).abs() < 1e-9);
        assert_eq!(report.prompts_analysed, 0);
    }

    #[test]
    fn parse_prose_accepts_bare_json_and_refuses_prose_without_it() {
        assert!(parse_prose("{\"headline\":\"h\",\"summary\":\"s\"}").is_ok());
        assert!(parse_prose("I cannot help with that.").is_err());
        assert!(parse_prose("{\"headline\":\"\",\"summary\":\"s\"}").is_err());
    }

    #[test]
    fn observe_collects_assistant_text_and_stops_at_result() {
        let mut text = String::new();
        assert!(!observe("{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"one\"}]}}", &mut text));
        assert!(!observe("not json", &mut text));
        assert!(observe("{\"type\":\"result\",\"result\":\"ignored because text exists\"}", &mut text));
        assert_eq!(text, "one");
        let mut empty = String::new();
        assert!(observe("{\"type\":\"result\",\"result\":\"{}\"}", &mut empty));
        assert_eq!(empty, "{}");
    }

    #[test]
    fn store_round_trips_the_latest_report() {
        let db = Connection::open_in_memory().unwrap();
        install_store(&db).unwrap();
        assert!(latest(&db).unwrap().is_none());
        let input = build_input(30, "a".into(), "b".into(), &[], vec![], None);
        let prose = parse_prose("{\"headline\":\"h\",\"summary\":\"s\"}").unwrap();
        let result = wire::UsageInsightsResult {
            status: wire::UsageInsightsStatus::Ready,
            window_days: 30,
            generated_at: Some("2026-09-10T00:00:00Z".into()),
            harness: Some("claude".into()),
            model: Some("sonnet".into()),
            report: Some(assemble(&input, prose)),
            detail: None,
        };
        store(&db, &result).unwrap();
        let read = latest(&db).unwrap().unwrap();
        assert_eq!(read, result);
        // A second store replaces, never appends.
        store(&db, &result).unwrap();
        let count: i64 = db.query_row("SELECT count(*) FROM usage_insights", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn every_day_in_the_window_is_present_at_zero() {
        let buckets = vec![bucket("2026-09-03", "codex", 1_000, 1)];
        let input = build_input(7, "2026-09-01".into(), "2026-09-07".into(), &buckets, vec![], None);
        assert_eq!(input.days.len(), 7);
        assert_eq!(input.days[0].day, "2026-09-01");
        assert_eq!(input.days[0].processed_tokens, 0);
        assert_eq!(input.days[2].processed_tokens, 1_000);
        assert_eq!(input.days[6].day, "2026-09-07");
    }

    fn descriptor(available: bool) -> AdapterDescriptor {
        AdapterDescriptor {
            id: "claude".into(),
            label: "Claude".into(),
            available,
            auth_state: crate::model::AuthState::SignedIn,
            version: Some("2.1.0".into()),
            capabilities: vec![],
            sandbox_modes: vec![],
            unavailable_reason: (!available).then(|| "claude is not on PATH".to_owned()),
            models: vec![],
            default_model: None,
            model_catalog: Default::default(),
        }
    }

    #[test]
    fn selection_falls_back_to_claude_only_when_nothing_is_configured() {
        let ok = choose(Err(BriefingUnavailable::NotConfigured), Some(&descriptor(true))).unwrap();
        assert_eq!(ok.0, "claude");
        let missing = choose(Err(BriefingUnavailable::NotConfigured), None).unwrap_err();
        assert_eq!(missing.status, wire::UsageInsightsStatus::Unavailable);
        let off = choose(Err(BriefingUnavailable::NotConfigured), Some(&descriptor(false))).unwrap_err();
        assert!(off.detail.unwrap().contains("not on PATH"));
    }

    #[test]
    fn a_configured_profile_that_does_not_resolve_is_refused_not_replaced() {
        let refused = choose(
            Err(BriefingUnavailable::UnknownHarness { harness: "codex".into() }),
            Some(&descriptor(true)),
        )
        .unwrap_err();
        assert_eq!(refused.status, wire::UsageInsightsStatus::Unavailable);
        assert!(refused.detail.unwrap().contains("configured briefing profile"));
        let chosen = choose(
            Ok(BriefingSelection { harness: "claude".into(), model: "opus".into(), effort: Some("high".into()), certified_provider_version: "x".into() }),
            None,
        )
        .unwrap();
        assert_eq!(chosen, ("claude".into(), "opus".into(), Some("high".into())));
    }

    #[test]
    fn without_refresh_and_without_a_report_the_answer_is_empty() {
        assert_eq!(empty(30).status, wire::UsageInsightsStatus::Empty);
        assert!(empty(30).report.is_none());
    }

    #[test]
    fn window_is_clamped() {
        assert_eq!(clamp_window(0), 1);
        assert_eq!(clamp_window(365), 90);
        assert_eq!(clamp_window(30), 30);
    }
}
