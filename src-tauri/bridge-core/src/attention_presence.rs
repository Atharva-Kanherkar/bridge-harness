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

impl ClientKind {
    /// What a client can do when it does not say: a desktop client posts
    /// banners, a web client did not before it had a browser channel.
    pub const fn posts_banners_by_default(self) -> bool {
        matches!(self, ClientKind::Desktop)
    }
}

#[derive(Debug, Clone)]
struct Lease {
    can_post_banners: bool,
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
        can_post_banners: Option<bool>,
        focused: bool,
        gone: bool,
        now: Instant,
    ) -> bool {
        let mut leases = self.leases.lock().unwrap();
        if gone {
            leases.remove(client_id);
            return false;
        }
        let can_post_banners = can_post_banners.unwrap_or(kind.posts_banners_by_default());
        let lease = leases.entry(client_id.to_owned()).or_insert_with(|| Lease {
            can_post_banners,
            focused: false,
            registered_at: now,
            seen_at: now,
            focused_at: None,
        });
        if focused && !lease.focused {
            lease.focused_at = Some(now);
        }
        lease.can_post_banners = can_post_banners;
        lease.focused = focused;
        lease.seen_at = now;
        owner(&mut leases, now).as_deref() == Some(client_id)
    }
}

/// The focused client that became focused last; otherwise, among clients that
/// can post banners, the one focused last, then the earliest to register.
/// Renewals move none of those instants, so the owner holds still between
/// reports.
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
        .filter(|(_, lease)| lease.can_post_banners)
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
        assert!(registry.report("desk", ClientKind::Desktop, None, false, false, start));
        assert!(registry.report("desk", ClientKind::Desktop, None, true, false, at(start, 1)));
    }

    #[test]
    fn the_focused_client_owns_and_hands_back_on_blur() {
        let registry = PresenceRegistry::default();
        let start = Instant::now();
        registry.report("desk", ClientKind::Desktop, None, false, false, start);
        assert!(registry.report("web", ClientKind::Web, None, true, false, at(start, 1)));
        assert!(!registry.report("desk", ClientKind::Desktop, None, false, false, at(start, 2)), "the person is at the web client");
        assert!(!registry.report("web", ClientKind::Web, None, false, false, at(start, 3)));
        assert!(registry.report("desk", ClientKind::Desktop, None, false, false, at(start, 4)), "back to the desktop");
    }

    #[test]
    fn renewals_do_not_move_the_owner() {
        let registry = PresenceRegistry::default();
        let start = Instant::now();
        registry.report("first", ClientKind::Desktop, None, false, false, start);
        registry.report("second", ClientKind::Desktop, None, false, false, at(start, 1));
        for second in 2..20 {
            assert!(registry.report("first", ClientKind::Desktop, None, false, false, at(start, second)));
            assert!(!registry.report("second", ClientKind::Desktop, None, false, false, at(start, second)));
        }
        // The second desktop was focused last, so it owns once both are blurred.
        registry.report("second", ClientKind::Desktop, None, true, false, at(start, 21));
        assert!(!registry.report("first", ClientKind::Desktop, None, false, false, at(start, 22)));
        assert!(registry.report("second", ClientKind::Desktop, None, false, false, at(start, 23)));
        assert!(!registry.report("first", ClientKind::Desktop, None, false, false, at(start, 24)));
    }

    #[test]
    fn an_expired_or_gone_owner_hands_over() {
        let registry = PresenceRegistry::default();
        let start = Instant::now();
        registry.report("owner", ClientKind::Desktop, None, false, false, start);
        assert!(!registry.report("other", ClientKind::Desktop, None, false, false, at(start, 1)));
        assert!(!registry.report("other", ClientKind::Desktop, None, false, false, at(start, 29)));
        assert!(registry.report("other", ClientKind::Desktop, None, false, false, at(start, 31)), "the owner stopped reporting");

        registry.report("owner", ClientKind::Desktop, None, true, false, at(start, 32));
        assert!(!registry.report("other", ClientKind::Desktop, None, false, false, at(start, 33)));
        assert!(!registry.report("owner", ClientKind::Desktop, None, false, true, at(start, 34)), "gone owns nothing");
        assert!(registry.report("other", ClientKind::Desktop, None, false, false, at(start, 35)), "a closing window hands over at once");
    }

    #[test]
    fn a_web_client_that_can_post_banners_owns_when_alone() {
        let registry = PresenceRegistry::default();
        let start = Instant::now();
        assert!(registry.report("web", ClientKind::Web, Some(true), false, false, start));
        assert!(registry.report("web", ClientKind::Web, Some(true), true, false, at(start, 1)));
        assert!(registry.report("web", ClientKind::Web, Some(true), false, false, at(start, 2)), "the tab went to the background");
    }

    #[test]
    fn the_capable_client_focused_last_owns() {
        let registry = PresenceRegistry::default();
        let start = Instant::now();
        registry.report("desk", ClientKind::Desktop, None, true, false, start);
        registry.report("desk", ClientKind::Desktop, None, false, false, at(start, 1));
        registry.report("web", ClientKind::Web, Some(true), true, false, at(start, 2));
        assert!(registry.report("web", ClientKind::Web, Some(true), false, false, at(start, 3)), "the person was last at the tab");
        assert!(!registry.report("desk", ClientKind::Desktop, None, false, false, at(start, 4)));
        // The person denies browser notifications: the desktop takes over.
        assert!(!registry.report("web", ClientKind::Web, Some(false), false, false, at(start, 5)));
        assert!(registry.report("desk", ClientKind::Desktop, None, false, false, at(start, 6)));
    }

    #[test]
    fn an_unfocused_web_client_never_owns() {
        let registry = PresenceRegistry::default();
        let start = Instant::now();
        assert!(!registry.report("web", ClientKind::Web, None, false, false, start));
        assert!(registry.report("web", ClientKind::Web, None, true, false, at(start, 1)));
        assert!(!registry.report("web", ClientKind::Web, None, false, false, at(start, 2)));
    }
}
