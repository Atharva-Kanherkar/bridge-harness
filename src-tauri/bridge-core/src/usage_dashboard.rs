//! Account dashboard history projected into the main Usage surface.
//! The collector, credentials and cache belong to `usage_overview`; this
//! module never imports account aggregates as transcript observations.

use crate::analytics::{CoverageState, ImporterCapability};
use crate::provider_usage::AccountHistory;
use crate::usage_history::UsageHistorySource;
use crate::usage_import::{ScanReport, SourceEnv, SourceScanOutcome};
use crate::usage_summary::{
    UsageResolution, UsageSummary, UsageSummaryRequest, UsageSummarySource,
};
use crate::{usage_history, usage_overview, usage_summary, BridgeCore, BridgeError};
use bridge_protocol::messages::{MenuBarProvider, UsageHistoryOrigin};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rusqlite::Connection;

// Resolves the currently verified account only. Callers cannot name or load
// a previous account's cache by passing a different source id.
const CURSOR_SOURCE: &str = "cursor:dashboard";

fn stamp(at: i64) -> Option<String> {
    DateTime::from_timestamp(at, 0).map(|at| at.to_rfc3339())
}

fn cursor_source(
    db: &Connection,
    now: i64,
) -> Result<(UsageHistorySource, Option<AccountHistory>), BridgeError> {
    let mut source = UsageHistorySource {
        id: CURSOR_SOURCE.into(),
        origin: UsageHistoryOrigin::Dashboard,
        agent: "cursor".into(),
        provider: "cursor".into(),
        location: "https://cursor.com/dashboard".into(),
        detected_version: None,
        capability: ImporterCapability::Supported,
        coverage_state: CoverageState::Unreadable,
        coverage_reason: Some("Sign in to Cursor desktop, then scan history.".into()),
        coverage_start_at: None,
        coverage_end_at: None,
        records_imported: 0,
        records_skipped: 0,
        last_successful_scan_at: None,
        last_error: None,
    };
    if !crate::menu_bar::load(db)?.provider_enabled(MenuBarProvider::Cursor) {
        source.coverage_state = CoverageState::Unsupported;
        source.coverage_reason =
            Some("Enable Cursor in Menu Bar settings to refresh dashboard history.".into());
        return Ok((source, None));
    }
    let cache = usage_overview::load_provider(db, "cursor")?;
    let usage = cache.usage.unwrap_or_default();
    source.last_error = cache.error.or(usage.history_error);
    let Some(history) = usage.history.filter(|history| {
        !history.account_scope.is_empty()
            && usage.account_scope.as_deref() == Some(history.account_scope.as_str())
    }) else {
        if source.last_error.is_some() {
            source.coverage_reason = source.last_error.clone();
        }
        return Ok((source, None));
    };
    source.last_successful_scan_at = stamp(history.observed_at);
    source.coverage_end_at = source.last_successful_scan_at.clone();
    let Some(breakdown) = &history.breakdown else {
        source.coverage_state = CoverageState::Partial;
        source.coverage_reason =
            Some("Scan history to update the cached Cursor token breakdown.".into());
        return Ok((source, None));
    };
    source.records_imported = breakdown.buckets.iter().map(|row| row.records).sum();
    source.coverage_start_at = breakdown
        .time_zone
        .parse::<chrono_tz::Tz>()
        .ok()
        .zip(NaiveDate::parse_from_str(&breakdown.since_day, "%Y-%m-%d").ok())
        .and_then(|(zone, day)| {
            zone.from_local_datetime(&day.and_hms_opt(0, 0, 0)?)
                .earliest()
        })
        .map(|at| at.to_rfc3339());
    let stale = source.last_error.is_some()
        || now < history.observed_at
        || now - history.observed_at >= 600;
    source.coverage_state = if stale {
        CoverageState::Stale
    } else if breakdown.buckets.is_empty() {
        CoverageState::Empty
    } else {
        CoverageState::Complete
    };
    source.coverage_reason = Some(format!(
        "Cursor dashboard account history · {} to {} · as of the last update. API-rate cost, not plan billing. Session counts and cache savings are unavailable.{}",
        breakdown.since_day, history.through_day,
        if stale { " Showing last-known usage; scan history to refresh." } else { "" },
    ));
    Ok((source, Some(history)))
}

pub(crate) fn list_sources(
    db: &Connection,
    env: &SourceEnv,
) -> Result<Vec<UsageHistorySource>, BridgeError> {
    let mut sources = usage_history::list_history_sources(db, env)?;
    sources.retain(|source| source.agent != "cursor");
    sources.push(cursor_source(db, Utc::now().timestamp())?.0);
    Ok(sources)
}

// WebKit and the OS may name the same zone differently (Asia/Calcutta and
// Asia/Kolkata). Daily aggregates are compatible exactly when every covered
// day boundary maps to the same instant, including DST transitions.
fn matching_calendar_zone(requested: &str, cached: &str, since: &str, until: &str) -> bool {
    let (Ok(requested), Ok(cached)) = (
        requested.parse::<chrono_tz::Tz>(),
        cached.parse::<chrono_tz::Tz>(),
    ) else {
        return false;
    };
    if requested == cached {
        return true;
    }
    let (Ok(since), Ok(until)) = (
        NaiveDate::parse_from_str(since, "%Y-%m-%d"),
        NaiveDate::parse_from_str(until, "%Y-%m-%d"),
    ) else {
        return false;
    };
    let days = (until - since).num_days();
    (0..=30).contains(&days)
        && (0..=days + 1).all(|offset| {
            let midnight = (since + chrono::Duration::days(offset))
                .and_hms_opt(0, 0, 0)
                .unwrap();
            let left = requested
                .from_local_datetime(&midnight)
                .earliest()
                .map(|at| at.timestamp());
            let right = cached
                .from_local_datetime(&midnight)
                .earliest()
                .map(|at| at.timestamp());
            left.is_some() && left == right
        })
}

pub(crate) fn summarize(
    db: &Connection,
    request: &UsageSummaryRequest,
    now: i64,
) -> Result<UsageSummary, BridgeError> {
    // New result domains are explicitly requested. A pre-dashboard client can
    // still decode every local bucket's integer session count on this daemon.
    if !request.include_dashboard {
        return usage_summary::summarize(db, request);
    }
    let (mut source, history) = cursor_source(db, now)?;
    let enabled = source.coverage_state != CoverageState::Unsupported;
    let mut selected = false;
    if let Some(history) = &history {
        let breakdown = history.breakdown.as_ref().unwrap();
        let reason = if request.workspace_id.is_some() {
            Some("Cursor dashboard history has no workspace attribution; only local usage is included.".into())
        } else if request.resolution != UsageResolution::Day {
            Some("Cursor dashboard history supports daily totals. Choose 7d or longer to include it.".into())
        } else {
            let zone = request
                .time_zone
                .as_deref()
                .map(str::trim)
                .filter(|zone| !zone.is_empty())
                .unwrap_or("UTC");
            (!matching_calendar_zone(zone, &breakdown.time_zone, &breakdown.since_day, &history.through_day)).then(|| format!(
                "Cursor history was captured in {}. Scan history in your current time zone before including it.",
                breakdown.time_zone,
            ))
        };
        if let Some(reason) = reason {
            source.coverage_state = CoverageState::Unsupported;
            source.coverage_reason = Some(reason);
        } else {
            selected = request.include_imported;
            if request.since_day.as_str() < breakdown.since_day.as_str()
                || request.until_day.as_str() > history.through_day.as_str()
            {
                if source.coverage_state != CoverageState::Stale {
                    source.coverage_state = CoverageState::Partial;
                }
                source.coverage_reason = Some(format!(
                    "Cursor dashboard covers {} to {}, as of the last update; this range is incomplete.",
                    breakdown.since_day, history.through_day,
                ));
            }
        }
    }
    // Exclusive remote authority, like CodexBar. Never add unbound device-local
    // Cursor records to an account total. Today is explicitly an as-of snapshot,
    // including when more local activity happened since the last refresh.
    let mut summary =
        usage_summary::summarize_excluding(db, request, selected.then_some("cursor"))?;
    if request.include_imported {
        summary.sources.retain(|source| source.agent != "cursor");
        if enabled {
            summary.sources.push(UsageSummarySource {
                id: source.id,
                origin: UsageHistoryOrigin::Dashboard,
                agent: source.agent,
                provider: source.provider,
                coverage_state: source.coverage_state.as_str().into(),
                coverage_reason: source.coverage_reason,
                records_imported: source.records_imported,
                records_skipped: 0,
                last_successful_scan_at: source.last_successful_scan_at,
            });
        }
    }
    if selected {
        summary.buckets.extend(
            history
                .unwrap()
                .breakdown
                .unwrap()
                .buckets
                .into_iter()
                .filter(|row| row.day >= summary.since_day && row.day <= summary.until_day),
        );
        summary
            .buckets
            .sort_by(|a, b| (&a.day, &a.harness, &a.model).cmp(&(&b.day, &b.harness, &b.model)));
    }
    Ok(summary)
}

pub(crate) fn scan(
    core: &BridgeCore,
    env: &SourceEnv,
    max_records: Option<usize>,
    source_ids: Option<&[String]>,
) -> Result<ScanReport, BridgeError> {
    scan_with_refresh(
        core,
        env,
        max_records,
        source_ids,
        usage_overview::refresh_cursor_history,
    )
}

fn scan_with_refresh(
    core: &BridgeCore,
    env: &SourceEnv,
    max_records: Option<usize>,
    source_ids: Option<&[String]>,
    refresh: impl FnOnce(&BridgeCore) -> Result<(), BridgeError>,
) -> Result<ScanReport, BridgeError> {
    let started = std::time::Instant::now();
    let refresh_cursor = source_ids.is_none_or(|ids| ids.iter().any(|id| id == CURSOR_SOURCE));
    let local_ids = source_ids.map(|ids| {
        ids.iter()
            .filter(|id| *id != CURSOR_SOURCE)
            .cloned()
            .collect::<Vec<_>>()
    });
    // Validates unknown ids before any remote work. Neither local scans nor
    // the provider's bounded network calls hold the database lock throughout.
    let mut report = usage_history::scan_history(core, env, max_records, local_ids.as_deref())?;
    if refresh_cursor {
        report.sources.retain(|source| source.agent != "cursor");
        let enabled = crate::menu_bar::load(&core.db.lock().unwrap())?
            .provider_enabled(MenuBarProvider::Cursor);
        if enabled {
            refresh(core)?;
        }
        let source = cursor_source(&core.db.lock().unwrap(), Utc::now().timestamp())?.0;
        report.sources.push(SourceScanOutcome {
            source_id: source.id,
            agent: source.agent,
            provider: source.provider,
            location: source.location,
            capability: source.capability,
            // No remote resume cursor: the dashboard request is all-or-nothing.
            coverage: if source.coverage_state == CoverageState::Partial {
                CoverageState::Unreadable
            } else {
                source.coverage_state
            },
            records_imported: 0,
            records_skipped: 0,
            next_cursor: None,
            warning: source.last_error.or(source.coverage_reason),
        });
    }
    report.duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_usage::{AccountHistoryBreakdown, AccountUsage};
    use crate::usage_overview::CachedProvider;
    use crate::usage_pricing::CostSource;
    use crate::usage_summary::{UsageBucket, UsageBucketTotals};
    use bridge_protocol::messages::{UsageMetric, UsageMetricSource, UsagePeriodOverview};

    #[test]
    fn equivalent_calendar_zone_names_are_accepted_without_rebucketing() {
        assert!(matching_calendar_zone(
            "Asia/Calcutta",
            "Asia/Kolkata",
            "2026-08-15",
            "2026-09-13"
        ));
        assert!(matching_calendar_zone(
            "US/Eastern",
            "America/New_York",
            "2026-03-01",
            "2026-03-30"
        ));
        assert!(!matching_calendar_zone(
            "America/Phoenix",
            "America/Denver",
            "2026-03-01",
            "2026-03-30"
        ));
        assert!(!matching_calendar_zone(
            "UTC",
            "Asia/Kolkata",
            "2026-08-15",
            "2026-09-13"
        ));
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        let mut cache = cached();
        cache
            .usage
            .as_mut()
            .unwrap()
            .history
            .as_mut()
            .unwrap()
            .breakdown
            .as_mut()
            .unwrap()
            .time_zone = "Asia/Kolkata".into();
        save(&db, &cache);
        let result = summarize(
            &db,
            &UsageSummaryRequest {
                time_zone: Some("Asia/Calcutta".into()),
                ..request()
            },
            now(),
        )
        .unwrap();
        assert_eq!(result.buckets.len(), 1);
        assert_eq!(result.sources.last().unwrap().coverage_state, "complete");
    }

    #[test]
    fn unknown_sources_and_disabled_cursor_never_invoke_the_collector() {
        let dir = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(dir.path());
        let env = SourceEnv::for_home(dir.path().join("empty-home"));
        let disabled = scan_with_refresh(&core, &env, None, Some(&[CURSOR_SOURCE.into()]), |_| {
            panic!("disabled collector ran")
        })
        .unwrap();
        assert_eq!(disabled.sources.len(), 1);
        assert_eq!(disabled.sources[0].coverage, CoverageState::Unsupported);
        save(&core.db.lock().unwrap(), &cached());
        let invalid = scan_with_refresh(
            &core,
            &env,
            None,
            Some(&[CURSOR_SOURCE.into(), "unknown:source".into()]),
            |_| panic!("invalid source list reached collector"),
        );
        assert!(invalid
            .unwrap_err()
            .to_string()
            .contains("unknown usage history source"));
        let mut calls = 0;
        let refreshed = scan_with_refresh(&core, &env, None, Some(&[CURSOR_SOURCE.into()]), |_| {
            calls += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(refreshed.records_imported, 0);
        assert_eq!(refreshed.sources[0].next_cursor, None);
        let listed = list_sources(&core.db.lock().unwrap(), &env).unwrap();
        assert_eq!(
            listed
                .iter()
                .filter(|source| source.agent == "cursor")
                .count(),
            1
        );
        assert_eq!(listed.last().unwrap().origin, UsageHistoryOrigin::Dashboard);
    }

    fn now() -> i64 {
        DateTime::parse_from_rfc3339("2026-09-13T12:00:00Z")
            .unwrap()
            .timestamp()
    }
    fn request() -> UsageSummaryRequest {
        UsageSummaryRequest {
            since_day: "2026-09-07".into(),
            until_day: "2026-09-13".into(),
            resolution: UsageResolution::Day,
            time_zone: Some("UTC".into()),
            workspace_id: None,
            include_imported: true,
            include_dashboard: true,
            since_time: None,
            until_time: None,
        }
    }
    fn cached() -> CachedProvider {
        let period = UsagePeriodOverview {
            tokens: UsageMetric::known(190.0, UsageMetricSource::Reported),
            cost_microusd: UsageMetric::known(12_500.0, UsageMetricSource::Reported),
            models: vec![],
        };
        CachedProvider {
            usage: Some(AccountUsage {
                account_scope: Some("account-a".into()),
                observed_at: now(),
                history: Some(AccountHistory {
                    account_scope: "account-a".into(),
                    observed_at: now(),
                    through_day: "2026-09-13".into(),
                    today: period.clone(),
                    month: period,
                    daily: vec![],
                    coverage: "dashboard".into(),
                    breakdown: Some(AccountHistoryBreakdown {
                        time_zone: "UTC".into(),
                        since_day: "2026-08-15".into(),
                        buckets: vec![UsageBucket {
                            day: "2026-09-13".into(),
                            hour_start: None,
                            harness: "cursor".into(),
                            model: "cursor-model".into(),
                            totals: UsageBucketTotals {
                                uncached_input_tokens: 100,
                                cache_read_tokens: 30,
                                cache_write_tokens: 40,
                                output_tokens: 20,
                                reasoning_tokens: 0,
                            },
                            cost_microusd: 12_500,
                            cache_savings_microusd: 0,
                            cost_source: CostSource::ProviderReported,
                            records: 2,
                            unpriced_records: 0,
                            sessions: None,
                        }],
                    }),
                }),
                ..Default::default()
            }),
            error: None,
        }
    }
    fn save(db: &Connection, cache: &CachedProvider) {
        let mut settings = crate::menu_bar::load(db).unwrap();
        settings.cursor_enabled = true;
        crate::menu_bar::save(db, &settings).unwrap();
        db.execute("INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES('usage_overview','cursor',?1,'now','now') ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload", [serde_json::to_string(cache).unwrap()]).unwrap();
    }

    #[test]
    fn daily_summary_selects_dashboard_exclusively_without_importing_ledger_rows() {
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        save(&db, &cached());
        for (harness, at) in [
            ("cursor", "2026-09-13T11:00:00Z"),
            ("cursor", "2026-09-13T13:00:00Z"),
            ("claude", "2026-09-13T13:00:00Z"),
        ] {
            db.execute("INSERT INTO usage_ledger(workspace_id,uncached_input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,harness,model,source,created_at) VALUES('w',1000,0,0,0,?1,'local',?2,?3)", rusqlite::params![harness, format!("provider.{harness}"), at]).unwrap();
        }
        let result = summarize(&db, &request(), now() + 30).unwrap();
        assert_eq!(result.buckets.len(), 2);
        let row = result
            .buckets
            .iter()
            .find(|row| row.harness == "cursor")
            .unwrap();
        assert_eq!(row.records, 2);
        assert_eq!(row.totals.cache_read_tokens, 30);
        assert_eq!(row.totals.cache_write_tokens, 40);
        assert_eq!(row.sessions, None);
        assert_eq!(result.live_records, 1);
        assert_eq!(result.imported_records, 0);
        assert_eq!(
            result.duplicates_dropped, 0,
            "account authority is not a transcript dedupe claim"
        );
        assert_eq!(
            result.sources.last().unwrap().origin,
            UsageHistoryOrigin::Dashboard
        );
        assert!(result
            .sources
            .last()
            .unwrap()
            .coverage_reason
            .as_ref()
            .unwrap()
            .contains("as of the last update"));
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM agent_usage_observations", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            0
        );
        let local = summarize(
            &db,
            &UsageSummaryRequest {
                include_imported: false,
                ..request()
            },
            now(),
        )
        .unwrap();
        assert_eq!(local.live_records, 3);
        assert!(local
            .sources
            .iter()
            .all(|source| source.origin != UsageHistoryOrigin::Dashboard));
    }

    #[test]
    fn source_survives_restart_but_preserves_staleness_and_exact_coverage_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.db");
        let mut cache = cached();
        cache.usage.as_mut().unwrap().history_error =
            Some("Cursor response could not be read".into());
        save(&crate::store::open(&path).unwrap(), &cache);
        let db = crate::store::open(&path).unwrap();
        let (source, history) = cursor_source(&db, now() + 60).unwrap();
        assert_eq!(source.coverage_state, CoverageState::Stale);
        assert_eq!(source.coverage_end_at, stamp(now()));
        assert_eq!(
            source.coverage_start_at.as_deref(),
            Some("2026-08-15T00:00:00+00:00")
        );
        assert_eq!(
            history.unwrap().breakdown.unwrap().buckets[0].cost_microusd,
            12_500
        );
        assert_eq!(
            summarize(&db, &request(), now() + 60).unwrap().buckets[0].cost_microusd,
            12_500
        );
    }

    #[test]
    fn cache_never_crosses_accounts_or_invents_data_for_old_cache_formats() {
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        for scope in [None, Some("account-b")] {
            let mut cache = cached();
            cache.usage.as_mut().unwrap().account_scope = scope.map(str::to_owned);
            save(&db, &cache);
            let result = summarize(&db, &request(), now()).unwrap();
            assert!(result.buckets.is_empty());
            assert_eq!(result.sources.last().unwrap().coverage_state, "unreadable");
        }
        let mut legacy = cached();
        legacy
            .usage
            .as_mut()
            .unwrap()
            .history
            .as_mut()
            .unwrap()
            .breakdown = None;
        save(&db, &legacy);
        let result = summarize(&db, &request(), now()).unwrap();
        assert!(result.buckets.is_empty());
        assert_eq!(result.sources.last().unwrap().coverage_state, "partial");
    }

    #[test]
    fn unsupported_windows_and_missing_dates_are_explicit() {
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        save(&db, &cached());
        for req in [
            UsageSummaryRequest {
                workspace_id: Some("w".into()),
                ..request()
            },
            UsageSummaryRequest {
                time_zone: Some("Asia/Kolkata".into()),
                ..request()
            },
            UsageSummaryRequest {
                resolution: UsageResolution::Hour,
                since_time: Some("2026-09-12T12:00:00Z".into()),
                until_time: Some("2026-09-13T12:00:00Z".into()),
                ..request()
            },
        ] {
            let result = summarize(&db, &req, now()).unwrap();
            assert!(result.buckets.is_empty());
            assert_eq!(result.sources.last().unwrap().coverage_state, "unsupported");
        }
        for req in [
            UsageSummaryRequest {
                since_day: "2026-08-01".into(),
                ..request()
            },
            UsageSummaryRequest {
                until_day: "2026-09-14".into(),
                ..request()
            },
        ] {
            let result = summarize(&db, &req, now()).unwrap();
            assert_eq!(result.buckets.len(), 1);
            assert_eq!(result.sources.last().unwrap().coverage_state, "partial");
        }
    }

    #[test]
    fn confirmed_empty_is_distinct_from_unavailable_and_disabled() {
        let db = crate::store::open(std::path::Path::new(":memory:")).unwrap();
        let mut settings = crate::menu_bar::load(&db).unwrap();
        settings.cursor_enabled = true;
        crate::menu_bar::save(&db, &settings).unwrap();
        assert_eq!(
            cursor_source(&db, now()).unwrap().0.coverage_state,
            CoverageState::Unreadable
        );
        let mut cache = cached();
        cache
            .usage
            .as_mut()
            .unwrap()
            .history
            .as_mut()
            .unwrap()
            .breakdown
            .as_mut()
            .unwrap()
            .buckets
            .clear();
        save(&db, &cache);
        assert_eq!(
            cursor_source(&db, now()).unwrap().0.coverage_state,
            CoverageState::Empty
        );
        let mut settings = crate::menu_bar::load(&db).unwrap();
        settings.cursor_enabled = false;
        crate::menu_bar::save(&db, &settings).unwrap();
        assert_eq!(
            cursor_source(&db, now()).unwrap().0.coverage_state,
            CoverageState::Unsupported
        );
        assert!(summarize(&db, &request(), now())
            .unwrap()
            .buckets
            .is_empty());
    }
}
