//! The event hub: one core-bus subscription per daemon, fanned out to
//! per-connection bounded queues.
//!
//! Why not one bus subscription per connection: the bus receiver can only be
//! waited on indefinitely (`blocking_recv` has no timeout and no external
//! wake), so a per-connection forwarder parked on a quiet bus could never be
//! joined — and with it, the connection's slot could never be released. The
//! hub is the daemon's single indefinitely-parked reader; connections drain
//! plain `mpsc` queues with a timeout and exit promptly when their socket
//! closes.
//!
//! Lag is per connection and never silent. A connection whose queue is full
//! stops receiving events; the moment its queue has room again it receives a
//! `stream-lagged` marker followed by the idempotent refetch hints, and the
//! client replays durable history from its cursors. If the hub's own bus
//! subscription lags (the bus dropped events before the hub read them), every
//! connection gets the same treatment.

use bridge_core::events::{CoreEvent, EventReceiver, ReceiveError};
use bridge_core::BridgeCore;
use bridge_protocol::{NotificationName, Params, RpcNotification};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};

/// Bounded per-connection queue. Generous — a queue this deep only fills when
/// a client stops reading its socket for a while.
const SINK_CAPACITY: usize = 1024;

struct Sink {
    sender: SyncSender<RpcNotification>,
    /// Events were dropped for this connection; owe it a lag marker and the
    /// refetch hints as soon as the queue has room.
    lagged: bool,
    missed: u64,
}

#[derive(Clone)]
pub struct EventHub {
    sinks: Arc<Mutex<Vec<Sink>>>,
}

impl EventHub {
    /// Subscribe to the core bus and start the distributor thread. Runs for
    /// the daemon's lifetime.
    pub fn start(core: &Arc<BridgeCore>) -> EventHub {
        let hub = EventHub { sinks: Arc::new(Mutex::new(Vec::new())) };
        let receiver = core.events.subscribe();
        let sinks = hub.sinks.clone();
        std::thread::Builder::new()
            .name("bridged-event-hub".into())
            .spawn(move || distribute(receiver, sinks))
            .expect("event hub thread spawns");
        hub
    }

    /// Register a connection. The connection owns the receiving end; dropping
    /// it unregisters the sink on the hub's next delivery attempt.
    pub fn register(&self) -> Receiver<RpcNotification> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(SINK_CAPACITY);
        self.sinks.lock().unwrap().push(Sink { sender, lagged: false, missed: 0 });
        receiver
    }
}

fn distribute(mut receiver: EventReceiver, sinks: Arc<Mutex<Vec<Sink>>>) {
    loop {
        match receiver.blocking_recv() {
            Ok(event) => {
                let notification = notification_for(&event);
                fan_out(&sinks, &receiver, Some(&notification));
            }
            Err(ReceiveError::Lagged(missed)) => {
                // The hub itself missed events: every connection lagged.
                let mut registered = sinks.lock().unwrap();
                for sink in registered.iter_mut() {
                    sink.lagged = true;
                    sink.missed = sink.missed.saturating_add(missed);
                }
                drop(registered);
                fan_out(&sinks, &receiver, None);
            }
            Err(ReceiveError::Closed) => break,
        }
    }
}

/// Deliver to every sink: recovery frames first for lagged sinks, then the
/// event itself. A full queue marks the sink lagged and drops the event (the
/// marker owed to it announces exactly that); a disconnected queue removes
/// the sink.
fn fan_out(
    sinks: &Arc<Mutex<Vec<Sink>>>,
    receiver: &EventReceiver,
    event: Option<&RpcNotification>,
) {
    // Recovery frames are built lazily, once, outside any sink's send path.
    let mut recovery: Option<Vec<RpcNotification>> = None;
    let mut registered = sinks.lock().unwrap();
    registered.retain_mut(|sink| {
        if sink.lagged {
            let frames = recovery.get_or_insert_with(|| {
                receiver
                    .reconciliation_events()
                    .iter()
                    .map(notification_for)
                    .collect()
            });
            match sink.sender.try_send(lag_marker(sink.missed)) {
                Ok(()) => {}
                // Still no room: the marker stays owed; skip this event too.
                Err(TrySendError::Full(_)) => {
                    sink.missed = sink.missed.saturating_add(u64::from(event.is_some()));
                    return true;
                }
                Err(TrySendError::Disconnected(_)) => return false,
            }
            for frame in frames.iter() {
                // Hints are idempotent; losing one to a re-filled queue is
                // covered by the next owed marker.
                match sink.sender.try_send(frame.clone()) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => break,
                    Err(TrySendError::Disconnected(_)) => return false,
                }
            }
            sink.lagged = false;
            sink.missed = 0;
        }
        let Some(event) = event else { return true };
        match sink.sender.try_send(event.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                sink.lagged = true;
                sink.missed = sink.missed.saturating_add(1);
                true
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    });
}

pub(crate) fn notification_for(event: &CoreEvent) -> RpcNotification {
    RpcNotification::new(event.kind().as_str(), Params::new(event.payload()).ok())
}

fn lag_marker(missed: u64) -> RpcNotification {
    RpcNotification::new(
        NotificationName::StreamLagged.as_str(),
        Params::new(serde_json::json!({ "missed": missed })).ok(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_core::events::EventBus;
    use std::time::Duration;

    /// Comfortably more than the core bus can buffer, so a non-draining sink
    /// forces a drop somewhere regardless of scheduler timing.
    const EVENT_BUS_TEST_MARGIN: usize = 2048;

    fn hub_over(bus: &EventBus) -> EventHub {
        // EventHub::start needs a BridgeCore; the distributor itself only
        // needs a receiver and the sink list, so tests wire it directly.
        let hub = EventHub { sinks: Arc::new(Mutex::new(Vec::new())) };
        let receiver = bus.subscribe();
        let sinks = hub.sinks.clone();
        std::thread::spawn(move || distribute(receiver, sinks));
        hub
    }

    #[test]
    fn events_fan_out_to_every_registered_connection() {
        let bus = EventBus::new();
        let hub = hub_over(&bus);
        let first = hub.register();
        let second = hub.register();
        bus.publish(CoreEvent::StateChanged);
        for receiver in [&first, &second] {
            let frame = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(frame.method, "state-changed");
        }
    }

    #[test]
    fn a_lagged_connection_gets_the_marker_and_hints_when_it_drains() {
        let bus = EventBus::new();
        let hub = hub_over(&bus);
        let receiver = hub.register();
        // A hint the recovery pass must resend.
        bus.publish(CoreEvent::StateChanged);
        // Overflow the sink while the connection reads nothing. More events
        // than either the bus or the sink can hold guarantees a drop on one
        // of the two layers — both of which owe the connection a marker.
        for index in 0..(SINK_CAPACITY + EVENT_BUS_TEST_MARGIN) {
            bus.publish(CoreEvent::SessionOutput {
                session_id: "s".into(),
                terminal_id: "t1".into(),
                data: index.to_string(),
            });
        }
        // Do not drain yet: wait until the hub has observed the overflow —
        // draining concurrently would relieve the pressure this test needs.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !hub.sinks.lock().unwrap()[0].lagged {
            assert!(std::time::Instant::now() < deadline, "the sink never overflowed");
            std::thread::sleep(Duration::from_millis(10));
        }

        let mut saw_marker = false;
        'outer: loop {
            assert!(std::time::Instant::now() < deadline, "no lag marker arrived");
            // Drain what is queued, then poke the hub with another event —
            // recovery frames are delivered on the next fan-out with room.
            while let Ok(frame) = receiver.recv_timeout(Duration::from_millis(200)) {
                if frame.method == "stream-lagged" {
                    let missed = frame.params.unwrap().into_value()["missed"].as_u64().unwrap();
                    assert!(missed > 0, "the marker reports how much was dropped");
                    saw_marker = true;
                    continue;
                }
                if saw_marker && frame.method == "state-changed" {
                    // The hint follows the marker: recovery is ordered.
                    break 'outer;
                }
            }
            bus.publish(CoreEvent::StateChanged);
        }
        assert!(saw_marker);
    }

    #[test]
    fn a_dropped_connection_unregisters_without_blocking_the_hub() {
        let bus = EventBus::new();
        let hub = hub_over(&bus);
        let kept = hub.register();
        drop(hub.register());
        bus.publish(CoreEvent::StateChanged);
        bus.publish(CoreEvent::AdaptersChanged);
        assert_eq!(
            kept.recv_timeout(Duration::from_secs(10)).unwrap().method,
            "state-changed"
        );
        assert_eq!(
            kept.recv_timeout(Duration::from_secs(10)).unwrap().method,
            "adapters-changed"
        );
        assert_eq!(hub.sinks.lock().unwrap().len(), 1, "the dropped sink is gone");
    }
}
