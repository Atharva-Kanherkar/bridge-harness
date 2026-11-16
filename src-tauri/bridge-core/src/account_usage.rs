//! Limits for the extra accounts in Settings, beside the user's own login.
//!
//! Each account is read the way a chat on it runs: the provider's own
//! collector, launched with the account's folder ([`provider_accounts`]) and
//! none of the inherited credentials that would outrank it. Readings are cached
//! next to the default ones under `<provider>:account:<id>`, the same id the
//! account's sign-in terminal uses, so an account and its limits never share a
//! row with the user's own login.
//!
//! Local token history has no account dimension, so an account row reports
//! limits only; the device ledger stays on the default row.

use crate::{
    provider_accounts::{self, HostHomes, ProviderAccount},
    provider_usage::AccountUsage,
    usage_overview::{self, CachedProvider, ProviderRefreshState},
    BridgeCore, BridgeError,
};
use bridge_protocol::messages::{MenuBarProvider, MenuBarSettings, UsageOverviewSnapshot};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

/// Providers whose extra accounts have limits Bridge can show, in the order
/// the default rows appear.
const PROVIDERS: [MenuBarProvider; 4] = MenuBarProvider::ALL;

/// The cache row for one account. Matches the account's sign-in terminal id.
pub(crate) fn cache_id(provider: &str, account: &str) -> String {
    format!("{provider}:account:{account}")
}

/// The configured accounts of every enabled harness that has limits, in
/// Settings order. A harness whose accounts fail validation contributes none:
/// Settings refuses to save such a record, so it only comes from hand edits.
pub(crate) fn configured<'a>(
    harnesses: impl IntoIterator<Item = (&'a str, bool, &'a serde_json::Value)>,
) -> Vec<(MenuBarProvider, ProviderAccount)> {
    let harnesses: Vec<_> = harnesses.into_iter().collect();
    PROVIDERS
        .into_iter()
        .flat_map(|provider| {
            harnesses
                .iter()
                .filter(move |(id, enabled, _)| *enabled && *id == provider.id())
                .flat_map(move |(id, _, advanced)| {
                    provider_accounts::accounts(id, advanced)
                        .unwrap_or_default()
                        .into_iter()
                        .map(move |account| (provider, account))
                })
        })
        .collect()
}

fn configured_in(db: &Connection) -> Vec<(MenuBarProvider, ProviderAccount)> {
    let Ok(state) = crate::agent_config::state(db) else { return Vec::new() };
    configured(state.harnesses.iter().map(|item| (item.id.as_str(), item.enabled, &item.advanced)))
}

/// A failed read cannot prove the cached quota still belongs to this account
/// (its login may have been replaced), so nothing from before survives it.
pub(crate) fn cache_result(result: Result<AccountUsage, String>) -> CachedProvider {
    match result {
        Ok(usage) => CachedProvider { usage: Some(usage), error: None },
        Err(error) => CachedProvider { usage: None, error: Some(error) },
    }
}

/// The snapshot for one account, from its cached reading.
pub(crate) fn snapshot(
    provider: MenuBarProvider,
    account: &ProviderAccount,
    cache: CachedProvider,
    now: DateTime<Utc>,
) -> UsageOverviewSnapshot {
    let failed = cache.error.is_some();
    let mut quota = cache.usage.unwrap_or_default();
    usage_overview::expire_provider(&mut quota, failed, now.timestamp());
    UsageOverviewSnapshot {
        schema_version: 1,
        generated_at: now.timestamp(),
        provider: provider.id().into(),
        account: quota.account,
        plan: quota.plan,
        observed_at: (quota.observed_at > 0).then_some(quota.observed_at),
        quota_source: quota.source,
        windows: quota.windows,
        // Resets redeem against the default login only.
        reset_credits: None,
        account_metrics: quota.metrics,
        today: usage_overview::unavailable_account_period(),
        month: usage_overview::unavailable_account_period(),
        daily: Vec::new(),
        coverage: "Account limits only · usage history is recorded with your own login".into(),
        error: if cache.error.is_none() && quota.observed_at == 0 {
            Some("Not read yet. Refresh to load this account's limits.".into())
        } else {
            cache.error
        },
        account_id: Some(account.id.clone()),
        account_label: Some(account.label.clone()),
    }
}

/// Every configured account of `provider`, for the overview that lists them
/// right after that provider's own row.
pub(crate) fn snapshots(db: &Connection, provider: &str, now: DateTime<Utc>) -> Vec<UsageOverviewSnapshot> {
    configured_in(db)
        .into_iter()
        .filter(|(item, _)| item.id() == provider)
        .map(|(item, account)| {
            let cache = usage_overview::load_provider(db, &cache_id(provider, &account.id)).unwrap_or_default();
            snapshot(item, &account, cache, now)
        })
        .collect()
}

fn gate(core: &BridgeCore, id: &str) -> Arc<Mutex<ProviderRefreshState>> {
    let mut gates = core.usage_overview.accounts.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    Arc::clone(gates.entry(id.to_owned()).or_default())
}

fn read(
    core: &BridgeCore,
    provider: MenuBarProvider,
    account: &ProviderAccount,
    interactive: bool,
) -> Result<AccountUsage, String> {
    let launch = provider_accounts::prepare_launch(provider.id(), account, &HostHomes::from_env())
        .map_err(|error| error.to_string())?;
    if provider == MenuBarProvider::Codex {
        let quota = crate::codex_adapter::account::read_as(interactive, Some(&launch))
            .map_err(|error| error.to_string())?;
        return Ok(AccountUsage {
            windows: usage_overview::windows(Some(&quota), false, quota.observed_at),
            account: quota.account,
            plan: quota.plan,
            observed_at: quota.observed_at,
            ..AccountUsage::default()
        });
    }
    crate::provider_usage::read_account(core, provider, &launch).map_err(|error| error.message)
}

fn refresh_one(core: &BridgeCore, provider: MenuBarProvider, account: &ProviderAccount, interactive: bool) -> Result<(), BridgeError> {
    let id = cache_id(provider.id(), &account.id);
    let requested_at = Instant::now();
    let gate = gate(core, &id);
    let mut last = gate.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if last.should_skip(interactive, requested_at) {
        return Ok(());
    }
    // Never hold the database across the provider process or network.
    let cache = cache_result(read(core, provider, account, interactive));
    core.db.lock().unwrap_or_else(std::sync::PoisonError::into_inner).execute(
        "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at) VALUES('usage_overview',?1,?2,?3,?3) ON CONFLICT(kind,id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
        params![id, serde_json::to_string(&cache).map_err(|e| BridgeError::Invalid(e.to_string()))?, Utc::now().to_rfc3339()],
    )?;
    last.record(interactive);
    Ok(())
}

/// Drop cached readings for accounts no longer in Settings, so a removed
/// account's limits cannot reappear under a new account with its id.
fn forget_removed(db: &Connection, keep: &[String]) -> Result<(), BridgeError> {
    let mut statement = db.prepare("SELECT id FROM configuration_entries WHERE kind='usage_overview' AND id LIKE '%:account:%'")?;
    let stale: Vec<String> = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .filter_map(Result::ok)
        .filter(|id| !keep.contains(id))
        .collect();
    for id in stale {
        db.execute("DELETE FROM configuration_entries WHERE kind='usage_overview' AND id=?1", [id])?;
    }
    Ok(())
}

/// Refresh every configured account whose provider the Menu Bar settings
/// enable, each on its own thread and deadline: one rejected login cannot hold
/// up another. Failures are cached per account, never returned.
pub(crate) fn refresh_all(core: &BridgeCore, settings: &MenuBarSettings, interactive: bool) {
    let accounts = {
        let db = core.db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let accounts = configured_in(&db);
        let keep: Vec<_> = accounts.iter().map(|(provider, account)| cache_id(provider.id(), &account.id)).collect();
        // A failed cleanup only leaves rows nothing lists; the refresh goes on.
        if forget_removed(&db, &keep).is_err() {
            eprintln!("bridge: could not drop cached limits for removed accounts");
        }
        accounts
    };
    std::thread::scope(|scope| {
        for (provider, account) in accounts.iter().filter(|(provider, _)| settings.provider_enabled(*provider)) {
            scope.spawn(move || {
                // Read failures are cached on the row; only a database write
                // can fail here, and the next refresh retries it.
                if refresh_one(core, *provider, account, interactive).is_err() {
                    eprintln!("bridge: could not store limits for {} account {}", provider.id(), account.id);
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn account(id: &str) -> ProviderAccount {
        ProviderAccount { id: id.into(), label: format!("Label {id}"), home_path: format!("/tmp/{id}"), shadow_home_path: String::new() }
    }

    #[test]
    fn cache_ids_never_collide_with_default_rows() {
        for provider in MenuBarProvider::ALL {
            assert_ne!(cache_id(provider.id(), "work"), provider.id());
            assert_eq!(cache_id(provider.id(), "work"), format!("{}:account:work", provider.id()));
        }
    }

    #[test]
    fn configured_lists_accounts_per_enabled_provider_in_order() {
        let claude = json!({"accounts": [{"id": "work", "label": "Work", "homePath": "~/.claude_work"}]});
        let codex = json!({"accounts": [{"id": "personal", "label": "Personal", "shadowHomePath": "~/.codex_personal"}]});
        let cursor = json!({"accounts": [{"id": "team", "label": "Team"}]});
        let disabled = json!({"accounts": [{"id": "off", "label": "Off", "homePath": "~/.oc"}]});
        let got = configured([
            ("claude", true, &claude),
            ("codex", true, &codex),
            ("cursor", true, &cursor),
            ("opencode", false, &disabled),
            ("grok", true, &claude),
        ]);
        let ids: Vec<_> = got.iter().map(|(provider, account)| (provider.id(), account.id.as_str())).collect();
        assert_eq!(ids, [("codex", "personal"), ("claude", "work"), ("cursor", "team")]);
    }

    #[test]
    fn a_failed_read_keeps_nothing_from_an_older_login() {
        let cache = cache_result(Err("expired".into()));
        assert!(cache.usage.is_none());
        let snap = snapshot(MenuBarProvider::Claude, &account("work"), cache, Utc::now());
        assert!(snap.windows.is_empty());
        assert_eq!(snap.account, None);
        assert_eq!(snap.error.as_deref(), Some("expired"));
        assert_eq!(snap.account_id.as_deref(), Some("work"));
        assert_eq!(snap.reset_credits, None);
    }

    #[test]
    fn an_unread_account_says_so_instead_of_showing_empty_limits() {
        let snap = snapshot(MenuBarProvider::Codex, &account("new"), CachedProvider::default(), Utc::now());
        assert!(snap.error.unwrap().contains("Refresh"));
        assert_eq!(snap.account_label.as_deref(), Some("Label new"));
    }

    /// Fuzz the Settings record the overview reads: arbitrary JSON in
    /// `advanced` must never panic, and anything it yields is a valid account
    /// whose cache id stays inside the account namespace.
    #[test]
    fn configured_survives_arbitrary_advanced_records() {
        let mut seed: u64 = 0x5eed_1234_abcd_0001;
        let mut next = || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; seed };
        let pieces = ["work", "", " ", "../x", "a/b", "claude", "Work Plan", "~", "/tmp/x", "~/.x", "rel/dir", "\u{0}", "é", "a:account:b", "x".repeat(80).leak()];
        for _ in 0..4_000 {
            let mut entries = Vec::new();
            for _ in 0..(next() % 4) {
                let mut entry = serde_json::Map::new();
                for key in ["id", "label", "homePath", "shadowHomePath", "extra"] {
                    if next() % 3 == 0 { continue; }
                    let value = match next() % 4 {
                        0 => json!(pieces[(next() % pieces.len() as u64) as usize]),
                        1 => json!(next() % 100),
                        2 => json!(null),
                        _ => json!([pieces[(next() % pieces.len() as u64) as usize]]),
                    };
                    entry.insert(key.into(), value);
                }
                entries.push(serde_json::Value::Object(entry));
            }
            let advanced = if next() % 5 == 0 { json!({"accounts": "nope"}) } else { json!({"accounts": entries}) };
            for harness in ["claude", "codex", "opencode", "cursor", "grok"] {
                for (provider, account) in configured([(harness, true, &advanced)]) {
                    assert_eq!(provider.id(), harness);
                    assert!(account.id.chars().next().unwrap().is_ascii_alphabetic());
                    assert!(account.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
                    assert!(cache_id(harness, &account.id).starts_with(&format!("{harness}:account:")));
                }
            }
        }
    }
}
