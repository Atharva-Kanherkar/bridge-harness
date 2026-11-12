//! Identity and time for notes: UUIDv7 ids and a hybrid logical clock.
//!
//! Both exist so a later sync service can merge two devices' notes without a
//! migration. A UUIDv7 is generated offline with no coordination and sorts by
//! creation time, which keeps the index compact. Wall clocks on two laptops
//! disagree and can step backwards; the clock below still orders one device's
//! changes strictly and stays close to wall time across devices.

use uuid::Uuid;

/// A new page, space or job id.
pub fn new_id() -> String {
    Uuid::now_v7().to_string()
}

/// A hybrid logical clock for one device. Stamps sort as strings in the same
/// order as the clock: `<millis:013>-<counter:06>-<device>`.
#[derive(Debug, Clone)]
pub struct Hlc {
    device: String,
    last_millis: i64,
    counter: u32,
}

impl Hlc {
    pub fn new(device: impl Into<String>) -> Self {
        Self { device: device.into(), last_millis: 0, counter: 0 }
    }

    /// Resume after a restart from the newest stamp this device wrote, so a
    /// clock that moved backwards across the restart still cannot reuse it.
    pub fn resume(device: impl Into<String>, newest: Option<&str>) -> Self {
        let mut clock = Self::new(device);
        if let Some((millis, counter)) = newest.and_then(parse) {
            clock.last_millis = millis;
            clock.counter = counter;
        }
        clock
    }

    /// The next stamp, given the wall clock in milliseconds.
    pub fn tick(&mut self, wall_millis: i64) -> String {
        if wall_millis > self.last_millis {
            self.last_millis = wall_millis;
            self.counter = 0;
        } else {
            self.counter += 1;
        }
        format!("{:013}-{:06}-{}", self.last_millis, self.counter, self.device)
    }

    pub fn now(&mut self) -> String {
        self.tick(chrono::Utc::now().timestamp_millis())
    }
}

fn parse(stamp: &str) -> Option<(i64, u32)> {
    let mut parts = stamp.splitn(3, '-');
    let millis = parts.next()?.parse().ok()?;
    let counter = parts.next()?.parse().ok()?;
    Some((millis, counter))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_uuidv7_and_time_ordered() {
        let first = new_id();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = new_id();
        let parsed = Uuid::parse_str(&first).unwrap();
        assert_eq!(parsed.get_version_num(), 7);
        assert!(first < second, "{first} should sort before {second}");
    }

    #[test]
    fn hlc_is_monotonic_when_the_wall_clock_goes_back() {
        let mut clock = Hlc::new("dev1");
        let a = clock.tick(1_000);
        let b = clock.tick(1_000);
        let c = clock.tick(900); // the wall clock stepped backwards
        let d = clock.tick(1_200);
        assert!(a < b && b < c && c < d, "{a} {b} {c} {d}");
        assert!(d.starts_with("0000000001200-000000-"));

        let mut resumed = Hlc::resume("dev1", Some(&d));
        let e = resumed.tick(500);
        assert!(d < e, "a resumed clock must not reuse {d}: got {e}");
    }
}
