//! Which client posts OS banners.
//!
//! Every client renders its own toasts, but a banner is the operating system's
//! and one event should raise one. Clients hold a short lease here, renewed
//! while they run, and core names one owner. In memory only: a restart starts
//! empty and the first report elects again.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A client that stops reporting for this long is gone. Clients renew every
/// 10 s, so a crashed owner hands over within 40 s.
pub const PRESENCE_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Desktop,
    Web,
}

#[derive(Debug, Clone)]
struct Lease {
    kind: ClientKind,
    focused: bool,
    registered_at: Instant,
    seen_at: Instant,
    /// The last time it became focused; `None` if it never was.
    focused_at: Option<Instant>,
}

#[derive(Debug, Default)]
pub struct PresenceRegistry {
    leases: Mutex<HashMap<String, Lease>>,
}

impl PresenceRegistry {
    /// Record or renew one client's lease and say whether it owns banners.
    /// `gone` removes it at once, as a closing window does.
    pub fn report(
        &self,
        client_id: &str,
        kind: ClientKind,
        focused: bool,
        gone: bool,
        now: Instant,
    ) -> bool {
        let mut leases = self.leases.lock().unwrap();
        if gone {
            leases.remove(client_id);
            return false;
        }
        let lease = leases.entry(client_id.to_owned()).or_insert_with(|| Lease {
            kind,
            focused: false,
            registered_at: now,
            seen_at: now,
            focused_at: None,
        });
        if focused && !lease.focused {
            lease.focused_at = Some(now);
        }
        lease.kind = kind;
        lease.focused = focused;
        lease.seen_at = now;
        owner(&mut leases, now).as_deref() == Some(client_id)
    }
}

/// The focused client that became focused last; otherwise the desktop client
/// focused last, then the earliest-registered desktop client. Renewals move
/// none of those instants, so the owner holds still between reports.
fn owner(leases: &mut HashMap<String, Lease>, now: Instant) -> Option<String> {
    leases.retain(|_, lease| now.duration_since(lease.seen_at) < PRESENCE_TTL);
    let focused = leases
        .iter()
        .filter(|(_, lease)| lease.focused)
        .max_by(|(a_id, a), (b_id, b)| a.focused_at.cmp(&b.focused_at).then_with(|| b_id.cmp(a_id)));
    if let Some((id, _)) = focused {
        return Some(id.clone());
    }
    leases
        .iter()
        .filter(|(_, lease)| lease.kind == ClientKind::Desktop)
        .max_by(|(a_id, a), (b_id, b)| {
            a.focused_at
                .cmp(&b.focused_at)
                .then_with(|| b.registered_at.cmp(&a.registered_at))
                .then_with(|| b_id.cmp(a_id))
        })
        .map(|(id, _)| id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, seconds: u64) -> Instant {
        start + Duration::from_secs(seconds)
    }

    #[test]
    fn a_lone_desktop_client_owns_banners() {
        let registry = PresenceRegistry::default();
        let start = Instant::now();
        assert!(registry.report("desk", ClientKind::Desktop, false, false, start));
        assert!(registry.report("desk", ClientKind::Desktop, true, false, at(start, 1)));
    }

    #[test]
    fn the_focused_client_owns_and_hands_back_on_blur() {
        let registry = PresenceRegistry::default();
        let start = Instant::now();
        registry.report("desk", ClientKind::Desktop, false, false, start);
        assert!(registry.report("web", ClientKind::Web, true, false, at(start, 1)));
        assert!(!registry.report("desk", ClientKind::Desktop, false, false, at(start, 2)), "the person is at the web client");
        assert!(!registry.report("web", ClientKind::Web, false, false, at(start, 3)));
        assert!(registry.report("desk", ClientKind::Desktop, false, false, at(start, 4)), "back to the desktop");
    }

    #[test]
    fn renewals_do_not_move_the_owner() {
        let registry = PresenceRegistry::default();
        let start = Instant::now();
        registry.report("first", ClientKind::Desktop, false, false, start);
        registry.report("second", ClientKind::Desktop, false, false, at(start, 1));
        for second in 2..20 {
            assert!(registry.report("first", ClientKind::Desktop, false, false, at(start, second)));
            assert!(!registry.report("second", ClientKind::Desktop, false, false, at(start, second)));
        }
        // The second desktop was focused last, so it owns once both are blurred.
        registry.report("second", ClientKind::Desktop, true, false, at(start, 21));
        assert!(!registry.report("first", ClientKind::Desktop, false, false, at(start, 22)));
        assert!(registry.report("second", ClientKind::Desktop, false, false, at(start, 23)));
        assert!(!registry.report("first", ClientKind::Desktop, false, false, at(start, 24)));
    }

    #[test]
    fn an_expired_or_gone_owner_hands_over() {
        let registry = PresenceRegistry::default();
        let start = Instant::now();
        registry.report("owner", ClientKind::Desktop, false, false, start);
        assert!(!registry.report("other", ClientKind::Desktop, false, false, at(start, 1)));
        assert!(!registry.report("other", ClientKind::Desktop, false, false, at(start, 29)));
        assert!(registry.report("other", ClientKind::Desktop, false, false, at(start, 31)), "the owner stopped reporting");

        registry.report("owner", ClientKind::Desktop, true, false, at(start, 32));
        assert!(!registry.report("other", ClientKind::Desktop, false, false, at(start, 33)));
        assert!(!registry.report("owner", ClientKind::Desktop, false, true, at(start, 34)), "gone owns nothing");
        assert!(registry.report("other", ClientKind::Desktop, false, false, at(start, 35)), "a closing window hands over at once");
    }

    #[test]
    fn an_unfocused_web_client_never_owns() {
        let registry = PresenceRegistry::default();
        let start = Instant::now();
        assert!(!registry.report("web", ClientKind::Web, false, false, start));
        assert!(registry.report("web", ClientKind::Web, true, false, at(start, 1)));
        assert!(!registry.report("web", ClientKind::Web, false, false, at(start, 2)));
    }
}
