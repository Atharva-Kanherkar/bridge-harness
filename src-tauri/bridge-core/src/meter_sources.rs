//! Cold-start limit sources for the menu-bar meter.
//!
//! `SessionStore::request_codex_usage` asks a *running* Codex adapter for its
//! rate limits, so with no Codex chat open the meter had nothing to show and
//! stayed blank until the user started one. Codex already writes the same
//! numbers to disk: every `token_count` event in a rollout carries the
//! account-wide `rate_limits` payload, so the limits are readable without any
//! session at all. This module reads them.
//!
//! The payload is published unchanged, in the shape the live adapter uses, so
//! the client parses one format regardless of which path produced it.
//!
//! The one transformation is the reset rule. `resets_at` is an absolute Unix
//! second, and a rollout is a historical record: a file from last week reports
//! a `used_percent` for a window that has since rolled over. Reporting that as
//! current would be worse than reporting nothing — but dropping the window
//! hides a limit the account really has. Any Codex use on this machine writes
//! fresh limits to a rollout, so a window that reset with no newer line means
//! nothing has been spent in it since: it is reported at zero, marked `fresh`,
//! with its stale reset removed. A payload with no window at all is discarded.

use serde_json::{Map, Value};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Newest rollouts to try before giving up. A rollout only carries limits once
/// its session has spent tokens, so the newest file can legitimately have none;
/// the bound keeps a large history from turning this into a full scan.
const MAX_ROLLOUTS_EXAMINED: usize = 25;

/// Keys in a `rate_limits` payload that are not quota windows.
const NON_WINDOW_KEYS: &[&str] = &[
    "limit_id",
    "limit_name",
    "credits",
    "plan_type",
    "rate_limit_reached_type",
    "spend_control_reached",
    "individual_limit",
];

/// Codex's account-wide rate limits, read from the newest rollout that reports
/// them. `None` when nothing is on disk or nothing reports a quota window.
///
/// `now_unix` is the comparison instant for the reset rule, injected so it is
/// testable rather than clock-dependent.
pub fn codex_rate_limits(sessions_dir: &Path, now_unix: i64) -> Option<Value> {
    for rollout in newest_rollouts(sessions_dir, MAX_ROLLOUTS_EXAMINED) {
        if let Some(limits) = last_rate_limits_in(&rollout) {
            if let Some(live) = settle_reset_windows(limits, now_unix) {
                return Some(live);
            }
        }
    }
    None
}

/// Rollout paths under `sessions_dir`, most recently written first.
///
/// Ordered by modification time rather than by the timestamp in the filename:
/// a session opened yesterday and still being appended to holds fresher limits
/// than one opened today and already finished.
fn newest_rollouts(sessions_dir: &Path, limit: usize) -> Vec<PathBuf> {
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    collect_rollouts(sessions_dir, &mut found, 0);
    found.sort_by(|left, right| right.0.cmp(&left.0));
    found.into_iter().take(limit).map(|(_, path)| path).collect()
}

/// Codex partitions rollouts as `sessions/YYYY/MM/DD/rollout-*.jsonl`; the
/// depth bound keeps an unexpected tree from becoming an unbounded walk.
fn collect_rollouts(dir: &Path, out: &mut Vec<(std::time::SystemTime, PathBuf)>, depth: usize) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            collect_rollouts(&path, out, depth + 1);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            out.push((
                metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
                path,
            ));
        }
    }
}

/// The last `rate_limits` payload in one rollout. Limits are cumulative
/// account state rather than per-turn deltas, so the newest line wins.
fn last_rate_limits_in(rollout: &Path) -> Option<Value> {
    let file = File::open(rollout).ok()?;
    let mut newest: Option<Value> = None;
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            // A rollout being appended to can yield a partial read; earlier
            // lines are still good, so stop rather than discard them.
            break;
        };
        // Cheap reject before parsing: most lines are conversation turns.
        if !line.contains("\"rate_limits\"") {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(limits) = parsed
            .get("payload")
            .and_then(|payload| payload.get("rate_limits"))
            .filter(|limits| limits.is_object())
        {
            newest = Some(limits.clone());
        }
    }
    newest
}

/// Apply the reset rule: a window whose reset has passed is reported at zero
/// and marked `fresh`, with the stale `resets_at` removed so no client counts
/// down to an instant that is already behind it. Returns `None` when the
/// payload holds no quota window at all. Non-window metadata (`plan_type`,
/// `credits`) is preserved so the client can still label the provider.
fn settle_reset_windows(limits: Value, now_unix: i64) -> Option<Value> {
    let Value::Object(fields) = limits else {
        return None;
    };
    let mut kept = Map::new();
    let mut windows = 0usize;
    for (key, value) in fields {
        if NON_WINDOW_KEYS.contains(&key.as_str()) || !value.is_object() {
            kept.insert(key, value);
            continue;
        }
        // A window with no reset carries no way to tell current from stale.
        let Some(resets_at) = value.get("resets_at").and_then(Value::as_i64) else {
            continue;
        };
        windows += 1;
        if resets_at <= now_unix {
            let mut fresh = value.as_object().cloned().unwrap_or_default();
            fresh.insert("used_percent".into(), Value::from(0.0));
            fresh.remove("resets_at");
            fresh.insert("fresh".into(), Value::Bool(true));
            kept.insert(key, Value::Object(fresh));
        } else {
            kept.insert(key, value);
        }
    }
    (windows > 0).then(|| Value::Object(kept))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_788_900_000;

    fn rollout_line(primary_reset: i64, secondary_reset: i64, primary_used: f64) -> String {
        json!({
            "timestamp": "2026-09-09T17:48:48.154Z",
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": { "model_context_window": 258_400 },
                "rate_limits": {
                    "limit_id": "codex",
                    "primary": { "used_percent": primary_used, "window_minutes": 300, "resets_at": primary_reset },
                    "secondary": { "used_percent": 8.0, "window_minutes": 10_080, "resets_at": secondary_reset },
                    "credits": { "has_credits": false, "unlimited": false, "balance": "0" },
                    "plan_type": "plus"
                }
            }
        })
        .to_string()
    }

    /// Writes `sessions/2026/09/<day>/rollout-<name>.jsonl`, newest mtime last.
    fn write_rollout(root: &Path, day: &str, name: &str, body: &str) -> PathBuf {
        let dir = root.join("2026").join("09").join(day);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-{name}.jsonl"));
        std::fs::write(&path, body).unwrap();
        path
    }

    fn touch(path: &Path, seconds_ago: u64) {
        let when = std::time::SystemTime::now() - std::time::Duration::from_secs(seconds_ago);
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    #[test]
    fn codex_rate_limits_from_rollout_reads_newest() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let old = write_rollout(root, "07", "old", &rollout_line(NOW + 900, NOW + 90_000, 11.0));
        let mid = write_rollout(root, "08", "mid", &rollout_line(NOW + 900, NOW + 90_000, 22.0));
        let new = write_rollout(root, "09", "new", &rollout_line(NOW + 900, NOW + 90_000, 33.0));
        touch(&old, 3_000);
        touch(&mid, 2_000);
        touch(&new, 10);

        let limits = codex_rate_limits(root, NOW).expect("newest rollout reports limits");
        assert_eq!(limits["primary"]["used_percent"], json!(33.0));
    }

    #[test]
    fn codex_rate_limits_takes_last_payload_in_file() {
        let temp = tempfile::tempdir().unwrap();
        let body = format!(
            "{}\n{}\n",
            rollout_line(NOW + 900, NOW + 90_000, 40.0),
            rollout_line(NOW + 900, NOW + 90_000, 65.0)
        );
        write_rollout(temp.path(), "09", "only", &body);

        let limits = codex_rate_limits(temp.path(), NOW).unwrap();
        assert_eq!(limits["primary"]["used_percent"], json!(65.0));
    }

    #[test]
    fn codex_rate_limits_reports_a_reset_window_as_fresh() {
        let temp = tempfile::tempdir().unwrap();
        // The 5h window reset an hour ago; its 50% is history, not current —
        // but the window itself is still a limit the account has.
        write_rollout(
            temp.path(),
            "09",
            "stale-primary",
            &rollout_line(NOW - 3_600, NOW + 90_000, 50.0),
        );

        let limits = codex_rate_limits(temp.path(), NOW).unwrap();
        assert_eq!(limits["primary"]["used_percent"], json!(0.0));
        assert_eq!(limits["primary"]["fresh"], json!(true));
        assert!(
            limits["primary"].get("resets_at").is_none(),
            "a reset already behind us must not be counted down to"
        );
        assert_eq!(limits["primary"]["window_minutes"], json!(300));
        assert_eq!(limits["secondary"]["used_percent"], json!(8.0));
        assert!(limits["secondary"].get("fresh").is_none());
        // Labelling metadata survives.
        assert_eq!(limits["plan_type"], json!("plus"));
    }

    #[test]
    fn codex_rate_limits_reports_every_window_fresh_when_all_have_reset() {
        let temp = tempfile::tempdir().unwrap();
        write_rollout(
            temp.path(),
            "09",
            "all-stale",
            &rollout_line(NOW - 7_200, NOW - 3_600, 50.0),
        );
        let limits = codex_rate_limits(temp.path(), NOW).unwrap();
        assert_eq!(limits["primary"]["used_percent"], json!(0.0));
        assert_eq!(limits["secondary"]["used_percent"], json!(0.0));
        assert_eq!(limits["secondary"]["fresh"], json!(true));
    }

    #[test]
    fn codex_rate_limits_none_without_rate_limits() {
        let temp = tempfile::tempdir().unwrap();
        let body = json!({
            "type": "event_msg",
            "payload": { "type": "token_count", "info": { "model_context_window": 1 } }
        })
        .to_string();
        write_rollout(temp.path(), "09", "no-limits", &body);
        assert!(codex_rate_limits(temp.path(), NOW).is_none());
    }

    #[test]
    fn codex_rate_limits_ignores_malformed_lines() {
        let temp = tempfile::tempdir().unwrap();
        let body = format!(
            "{{\"rate_limits\": truncated\n{}\nnot json at all\n",
            rollout_line(NOW + 900, NOW + 90_000, 12.0)
        );
        write_rollout(temp.path(), "09", "messy", &body);

        let limits = codex_rate_limits(temp.path(), NOW).unwrap();
        assert_eq!(limits["primary"]["used_percent"], json!(12.0));
    }

    #[test]
    fn codex_rate_limits_falls_back_to_an_older_rollout() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        // The newest session has not spent tokens yet, so it reports nothing.
        let empty = write_rollout(root, "09", "fresh", "{\"type\":\"session_meta\"}\n");
        let older = write_rollout(root, "08", "spent", &rollout_line(NOW + 900, NOW + 90_000, 27.0));
        touch(&older, 600);
        touch(&empty, 5);

        let limits = codex_rate_limits(root, NOW).unwrap();
        assert_eq!(limits["primary"]["used_percent"], json!(27.0));
    }

    #[test]
    fn codex_rate_limits_none_when_sessions_dir_missing() {
        let temp = tempfile::tempdir().unwrap();
        assert!(codex_rate_limits(&temp.path().join("absent"), NOW).is_none());
    }
}

