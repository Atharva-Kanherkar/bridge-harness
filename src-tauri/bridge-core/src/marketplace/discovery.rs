use super::{ClaudeDiscovery, ClaudeSdkConfiguration};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, TryLockError};
use std::thread;
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(60);

struct CachedDiscovery {
    configuration: ClaudeSdkConfiguration,
    discovered_at: Instant,
    checked_at: Instant,
    has_successful_value: bool,
}

#[derive(Default)]
struct State {
    cached: Option<CachedDiscovery>,
    refreshing: bool,
    generation: u64,
}

#[derive(Default)]
struct Slot {
    state: Mutex<State>,
    ready: Condvar,
}

impl Slot {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn get(
        self: &Arc<Self>,
        probe: impl Fn() -> ClaudeSdkConfiguration + Send + 'static,
        clock: impl Fn() -> Instant + Send + 'static,
    ) -> ClaudeSdkConfiguration {
        let mut state = self.lock();
        loop {
            if let Some(cached) = &state.cached {
                let configuration = cached.configuration.clone();
                if clock().saturating_duration_since(cached.checked_at) >= TTL && !state.refreshing
                {
                    state.refreshing = true;
                    let generation = state.generation;
                    drop(state);
                    let slot = Arc::clone(self);
                    if let Err(error) = thread::Builder::new()
                        .name("claude-discovery-refresh".into())
                        .spawn(move || slot.refresh(generation, probe, clock))
                    {
                        self.lock().refreshing = false;
                        self.ready.notify_all();
                        eprintln!("bridge: could not start Claude discovery refresh: {error}");
                    }
                }
                return configuration;
            }

            if state.refreshing {
                state = self
                    .ready
                    .wait(state)
                    .unwrap_or_else(PoisonError::into_inner);
                continue;
            }

            state.refreshing = true;
            let generation = state.generation;
            drop(state);
            self.refresh(generation, &probe, &clock);
            // Re-read under the lock in case a marketplace action invalidated
            // the result while this cold probe was running.
            state = self.lock();
        }
    }

    fn refresh(
        &self,
        generation: u64,
        probe: impl FnOnce() -> ClaudeSdkConfiguration,
        clock: impl FnOnce() -> Instant,
    ) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(probe));
        let now = clock();
        let mut state = self.lock();
        state.refreshing = false;
        self.ready.notify_all();
        let configuration = match result {
            Ok(configuration) => configuration,
            Err(panic) => {
                drop(state);
                std::panic::resume_unwind(panic);
            }
        };
        if generation != state.generation {
            return;
        }
        let success = configuration.diagnostics.is_empty();
        match &mut state.cached {
            Some(cached) if cached.has_successful_value && !success => {
                // Keep the last usable result, and retry at the usual cadence.
                cached.checked_at = now;
                cached.configuration.diagnostics = configuration.diagnostics;
            }
            cached => {
                *cached = Some(CachedDiscovery {
                    configuration,
                    discovered_at: now,
                    checked_at: now,
                    has_successful_value: success,
                });
            }
        }
    }

    fn invalidate(&self) {
        let mut state = self.lock();
        state.cached = None;
        state.generation += 1;
    }

    fn peek(&self) -> Option<(ClaudeSdkConfiguration, Duration)> {
        let state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        let cached = state.cached.as_ref()?;
        Some((cached.configuration.clone(), cached.discovered_at.elapsed()))
    }
}

#[derive(Default)]
pub(super) struct DiscoveryCache {
    plugins: Arc<Slot>,
    connectors: Arc<Slot>,
}

impl DiscoveryCache {
    pub(super) fn get(
        &self,
        discovery: ClaudeDiscovery,
        probe: impl Fn(ClaudeDiscovery) -> ClaudeSdkConfiguration + Clone + Send + 'static,
    ) -> ClaudeSdkConfiguration {
        let mut configuration = ClaudeSdkConfiguration::default();
        if discovery.plugins {
            let probe = probe.clone();
            configuration = self
                .plugins
                .get(move || probe(ClaudeDiscovery::PLUGINS), Instant::now);
        }
        if discovery.connectors {
            let connectors = self
                .connectors
                .get(move || probe(ClaudeDiscovery::CONNECTORS), Instant::now);
            configuration.mcp_servers = connectors.mcp_servers;
            configuration.connector_health = connectors.connector_health;
            for diagnostic in connectors.diagnostics {
                if !configuration
                    .diagnostics
                    .iter()
                    .any(|seen| seen.stage == diagnostic.stage)
                {
                    configuration.diagnostics.push(diagnostic);
                }
            }
        }
        configuration
    }

    pub(super) fn refresh_after_change(
        &self,
        discovery: ClaudeDiscovery,
        probe: impl Fn(ClaudeDiscovery) -> ClaudeSdkConfiguration + Clone + Send + 'static,
    ) {
        if discovery.plugins {
            self.plugins.invalidate();
        }
        if discovery.connectors {
            self.connectors.invalidate();
        }
        thread::scope(|scope| {
            if discovery.plugins {
                let probe = probe.clone();
                scope.spawn(move || self.get(ClaudeDiscovery::PLUGINS, probe));
            }
            if discovery.connectors {
                self.get(ClaudeDiscovery::CONNECTORS, probe);
            }
        });
    }

    pub(super) fn peek_connectors(&self) -> Option<(ClaudeSdkConfiguration, Duration)> {
        self.connectors.peek()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marketplace::CapabilityDiscoveryDiagnostic;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Barrier};

    const TEST_WAIT: Duration = Duration::from_secs(2);
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn plugins(path: &str) -> ClaudeSdkConfiguration {
        ClaudeSdkConfiguration {
            plugins: vec![path.into()],
            ..Default::default()
        }
    }

    fn wait_for_refresh(slot: &Slot) {
        let (state, _) = slot
            .ready
            .wait_timeout_while(slot.lock(), TEST_WAIT, |state| state.refreshing)
            .unwrap_or_else(PoisonError::into_inner);
        assert!(!state.refreshing, "discovery did not finish");
    }

    #[test]
    fn cold_plugin_read_finishes_during_a_connector_probe() -> TestResult {
        let cache = DiscoveryCache::default();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (plugins_tx, plugins_rx) = mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));

        thread::scope(|scope| -> TestResult {
            scope.spawn(|| {
                cache.get(ClaudeDiscovery::CONNECTORS, move |part| {
                    assert_eq!(part, ClaudeDiscovery::CONNECTORS);
                    assert!(started_tx.send(()).is_ok());
                    let release = release_rx.lock().unwrap_or_else(PoisonError::into_inner);
                    assert!(release.recv_timeout(TEST_WAIT).is_ok());
                    ClaudeSdkConfiguration::default()
                });
            });
            started_rx.recv_timeout(TEST_WAIT)?;
            assert!(cache.peek_connectors().is_none());
            let cache = &cache;
            scope.spawn(move || {
                let started = Instant::now();
                let cold = cache.get(ClaudeDiscovery::PLUGINS, |part| {
                    assert_eq!(part, ClaudeDiscovery::PLUGINS);
                    plugins("/plugins/one")
                });
                let warm = cache.get(ClaudeDiscovery::PLUGINS, |_| plugins("/wrong"));
                assert!(plugins_tx.send((cold, warm, started.elapsed())).is_ok());
            });
            let result = plugins_rx.recv_timeout(Duration::from_millis(500));
            release_tx.send(())?;
            let (cold, warm, elapsed) =
                result.map_err(|_| "plugin read waited for the connector probe")?;
            assert_eq!(cold.plugins, ["/plugins/one"]);
            assert_eq!(warm.plugins, cold.plugins);
            eprintln!("Cold and warm plugin reads during a blocked connector probe: {elapsed:?}");
            Ok(())
        })
    }

    #[test]
    fn discovery_cache_single_flight_on_a_cold_key() {
        let cache = DiscoveryCache::default();
        let probes = Arc::new(AtomicUsize::new(0));
        let readers = Barrier::new(8);
        thread::scope(|scope| {
            for _ in 0..8 {
                let probes = Arc::clone(&probes);
                let (cache, readers) = (&cache, &readers);
                scope.spawn(move || {
                    readers.wait();
                    let result = cache.get(ClaudeDiscovery::PLUGINS, move |_| {
                        probes.fetch_add(1, Ordering::Relaxed);
                        thread::sleep(Duration::from_millis(20));
                        plugins("/plugins/one")
                    });
                    assert_eq!(result.plugins, ["/plugins/one"]);
                });
            }
        });
        assert_eq!(probes.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn expired_read_returns_stale_while_one_probe_replaces_it() -> TestResult {
        let slot = Arc::new(Slot::default());
        let now = Instant::now();
        let expired = move || now + TTL;
        assert_eq!(slot.get(|| plugins("/old"), move || now).plugins, ["/old"]);

        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let result = slot.get(
            move || {
                assert!(started_tx.send(()).is_ok());
                assert!(release_rx.recv_timeout(TEST_WAIT).is_ok());
                plugins("/new")
            },
            expired,
        );
        assert_eq!(result.plugins, ["/old"]);
        started_rx.recv_timeout(TEST_WAIT)?;

        let duplicates = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&duplicates);
        assert_eq!(
            slot.get(
                move || {
                    counter.fetch_add(1, Ordering::Relaxed);
                    plugins("/wrong")
                },
                expired
            )
            .plugins,
            ["/old"]
        );
        release_tx.send(())?;
        wait_for_refresh(&slot);
        assert_eq!(duplicates.load(Ordering::Relaxed), 0);
        assert_eq!(slot.get(|| plugins("/wrong"), expired).plugins, ["/new"]);
        Ok(())
    }

    #[test]
    fn failed_refresh_keeps_last_good_value_and_waits_before_retrying() -> TestResult {
        let slot = Arc::new(Slot::default());
        let now = Instant::now();
        let expired = move || now + TTL;
        slot.get(|| plugins("/good"), move || now);
        let (attempted_tx, attempted_rx) = mpsc::channel();
        assert_eq!(
            slot.get(
                move || {
                    assert!(attempted_tx.send(()).is_ok());
                    ClaudeSdkConfiguration {
                        diagnostics: vec![CapabilityDiscoveryDiagnostic {
                            stage: "plugins".into(),
                            status: "failed".into(),
                            message: "fixture failed".into(),
                        }],
                        ..Default::default()
                    }
                },
                expired
            )
            .plugins,
            ["/good"]
        );
        attempted_rx.recv_timeout(TEST_WAIT)?;
        wait_for_refresh(&slot);
        let stale = slot.get(|| plugins("/wrong"), expired);
        assert_eq!(stale.plugins, ["/good"]);
        assert_eq!(
            stale.diagnostics.first().map(|d| d.status.as_str()),
            Some("failed")
        );
        wait_for_refresh(&slot);
        assert_eq!(
            slot.get(|| plugins("/recovered"), move || now + TTL * 2)
                .plugins,
            ["/good"]
        );
        wait_for_refresh(&slot);
        assert_eq!(
            slot.get(|| plugins("/wrong"), move || now + TTL * 2)
                .plugins,
            ["/recovered"]
        );
        Ok(())
    }

    #[test]
    fn invalidation_discards_a_probe_started_before_the_change() -> TestResult {
        let slot = Arc::new(Slot::default());
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        thread::scope(|scope| -> TestResult {
            scope.spawn(|| {
                let attempts = AtomicUsize::new(0);
                let result = slot.get(
                    move || {
                        if attempts.fetch_add(1, Ordering::Relaxed) == 0 {
                            assert!(started_tx.send(()).is_ok());
                            assert!(release_rx.recv_timeout(TEST_WAIT).is_ok());
                            return plugins("/old");
                        }
                        plugins("/new")
                    },
                    Instant::now,
                );
                assert_eq!(result.plugins, ["/new"]);
            });
            started_rx.recv_timeout(TEST_WAIT)?;
            slot.invalidate();
            release_tx.send(())?;
            assert_eq!(slot.get(|| plugins("/new"), Instant::now).plugins, ["/new"]);
            Ok(())
        })
    }

    #[test]
    fn marketplace_refresh_updates_affected_parts_before_returning() {
        let cache = DiscoveryCache::default();
        assert!(cache.peek_connectors().is_none());
        cache.get(ClaudeDiscovery::PLUGINS, |_| plugins("/old"));
        assert!(cache.peek_connectors().is_none());
        let connector = |connected| ClaudeSdkConfiguration {
            connector_health: BTreeMap::from([("claude.ai Example".into(), Some(connected))]),
            ..Default::default()
        };
        cache.get(ClaudeDiscovery::CONNECTORS, move |_| connector(false));
        cache.refresh_after_change(ClaudeDiscovery::CONNECTORS, move |part| {
            assert_eq!(part, ClaudeDiscovery::CONNECTORS);
            connector(true)
        });
        assert_eq!(
            cache
                .get(ClaudeDiscovery::PLUGINS, |_| plugins("/wrong"))
                .plugins,
            ["/old"]
        );
        assert_eq!(
            cache
                .peek_connectors()
                .map(|(value, _)| value.connector_health),
            Some(connector(true).connector_health)
        );

        cache.refresh_after_change(ClaudeDiscovery::ALL, move |part| {
            if part.plugins {
                plugins("/new")
            } else {
                connector(false)
            }
        });
        assert_eq!(
            cache
                .get(ClaudeDiscovery::PLUGINS, |_| plugins("/wrong"))
                .plugins,
            ["/new"]
        );
        assert_eq!(
            cache
                .peek_connectors()
                .map(|(value, _)| value.connector_health),
            Some(connector(false).connector_health)
        );
    }
}
