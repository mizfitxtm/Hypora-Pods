//! Desktop notifications (org.freedesktop.Notifications) and the low-battery rule.

use std::collections::HashMap;
use zbus::zvariant::Value;

/// Fires once when a level drops to `trigger`, then stays quiet until the
/// level climbs back to `reset` or the device charges. The gap stops flapping.
#[derive(Debug, Clone, Copy)]
pub struct LowBatteryLatch {
    trigger: u8,
    reset: u8,
    notified: bool,
}

impl Default for LowBatteryLatch {
    fn default() -> Self {
        Self { trigger: 10, reset: 15, notified: false }
    }
}

impl LowBatteryLatch {
    /// `Some(level)` when a notification should be sent now.
    pub fn evaluate(&mut self, level: u8, available: bool, charging: bool) -> Option<u8> {
        if !available {
            return None;
        }
        if charging {
            self.notified = false;
            return None;
        }
        if level <= self.trigger && !self.notified {
            self.notified = true;
            return Some(level);
        }
        if level >= self.reset {
            self.notified = false;
        }
        None
    }
}

/// One latch per battery.
#[derive(Debug, Default)]
pub struct BatteryWatch {
    pub left: LowBatteryLatch,
    pub right: LowBatteryLatch,
    pub case: LowBatteryLatch,
    pub headset: LowBatteryLatch,
}

impl BatteryWatch {
    /// Notification texts due for this status.
    pub fn check(&mut self, s: &aap_protocol::status::Status) -> Vec<String> {
        let mut out = Vec::new();
        let mut one = |latch: &mut LowBatteryLatch, name: &str, b: (bool, u8, bool)| {
            if let Some(level) = latch.evaluate(b.1, b.0, b.2) {
                out.push(format!("{name} battery low ({level}%)"));
            }
        };
        if s.is_headset {
            one(&mut self.headset, "AirPods", (s.headset.available, s.headset.level, s.headset.charging));
        } else {
            one(&mut self.left, "Left AirPod", (s.left.available, s.left.level, s.left.charging));
            one(&mut self.right, "Right AirPod", (s.right.available, s.right.level, s.right.charging));
            one(&mut self.case, "AirPods case", (s.case_battery.available, s.case_battery.level, s.case_battery.charging));
        }
        out
    }
}

/// Sends one notification. Failure is logged, never fatal: a missing notification
/// daemon must not take the AirPods daemon down.
pub async fn send(title: &str, body: &str) {
    let result: zbus::Result<()> = async {
        let conn = zbus::Connection::session().await?;
        conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &("AirPods", 0u32, "audio-headphones", title, body, Vec::<&str>::new(), HashMap::<&str, Value>::new(), 5000i32),
        )
        .await?;
        Ok(())
    }
    .await;
    if let Err(e) = result {
        eprintln!("cannot send notification: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_once_then_waits_for_reset() {
        let mut l = LowBatteryLatch::default();
        assert_eq!(l.evaluate(30, true, false), None);
        assert_eq!(l.evaluate(10, true, false), Some(10));
        assert_eq!(l.evaluate(9, true, false), None); // already told
        assert_eq!(l.evaluate(12, true, false), None); // between trigger and reset: still latched
        assert_eq!(l.evaluate(10, true, false), None);
        assert_eq!(l.evaluate(15, true, false), None); // reset
        assert_eq!(l.evaluate(8, true, false), Some(8));
    }

    #[test]
    fn charging_and_unavailable_never_fire() {
        let mut l = LowBatteryLatch::default();
        assert_eq!(l.evaluate(5, true, true), None);
        assert_eq!(l.evaluate(5, false, false), None);
        assert_eq!(l.evaluate(5, true, false), Some(5)); // charging cleared the latch
    }

    #[test]
    fn watch_names_the_pod() {
        let mut s = aap_protocol::status::Status::new();
        s.left = aap_protocol::status::Pod { available: true, level: 7, charging: false, in_ear: true };
        let mut w = BatteryWatch::default();
        assert_eq!(w.check(&s), vec!["Left AirPod battery low (7%)".to_string()]);
        assert!(w.check(&s).is_empty());
    }
}
