//! Turns the status JSON into something a person can read at a glance.

use aap_protocol::status::{Battery, Pod, Status};

fn percent(available: bool, level: u8) -> String {
    if available { format!("{level}%") } else { "unknown".to_string() }
}

fn pod_line(name: &str, pod: &Pod) -> String {
    let mut notes = Vec::new();
    if pod.charging {
        notes.push("charging");
    }
    if pod.in_ear {
        notes.push("in ear");
    }
    let notes = if notes.is_empty() { String::new() } else { format!("  ({})", notes.join(", ")) };
    format!("  {name:<8}{}{notes}", percent(pod.available, pod.level))
}

fn battery_line(name: &str, b: &Battery) -> String {
    let note = if b.charging { "  (charging)" } else { "" };
    format!("  {name:<8}{}{note}", percent(b.available, b.level))
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

pub fn render(s: &Status) -> String {
    // The daemon already cleans names; do it again so a stray file cannot inject terminal escapes.
    let mut s = s.clone();
    s.device_name = aap_protocol::status::sanitize_name(&s.device_name);
    s.model_name = aap_protocol::status::sanitize_name(&s.model_name);
    let s = &s;
    let mut out = Vec::new();
    let title = if !s.model_name.is_empty() { s.model_name.as_str() } else { "AirPods" };
    out.push(if s.connected {
        format!("{title}: connected")
    } else {
        format!("{title}: not connected")
    });
    if !s.device_name.is_empty() && s.device_name != s.model_name {
        out.push(format!("  name    {}", s.device_name));
    }

    out.push(String::new());
    out.push("Battery".to_string());
    if s.is_headset {
        out.push(battery_line("Headset", &s.headset));
    } else {
        out.push(pod_line("Left", &s.left));
        out.push(pod_line("Right", &s.right));
        if s.case_battery.available {
            out.push(battery_line("Case", &s.case_battery));
        }
    }

    if s.connected && s.supports_noise_control {
        out.push(String::new());
        let mode = match s.noise_mode {
            0 => "Off",
            1 => "Noise Cancellation",
            2 => "Transparency",
            3 => "Adaptive",
            _ => "unknown",
        };
        out.push(format!("Listening mode  {mode}"));
        if s.noise_mode == 3 {
            out.push(format!("Adaptive level  {}%", s.adaptive_noise_level));
        }
        if s.supports_conversational_awareness {
            out.push(format!("Conversation Awareness  {}", on_off(s.conversational_awareness)));
        }
        if s.supports_one_bud_anc {
            out.push(format!("One-Bud ANC  {}", on_off(s.one_bud_anc_mode)));
        }
    }

    out.push(String::new());
    out.push(format!(
        "Ear detection  {}",
        match s.ear_detection_behavior {
            0 => "pause when one pod is out",
            1 => "pause when both pods are out",
            _ => "never pause",
        }
    ));
    if !s.is_headset {
        let lid = match s.lid_state {
            0 => Some("open"),
            1 => Some("closed"),
            _ => None,
        };
        if let Some(lid) = lid {
            out.push(format!("Case lid  {lid}"));
        }
    }
    out.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use aap_protocol::model::Model;

    fn pro3() -> Status {
        let mut s = Status::new();
        s.set_model(Model::AirPodsPro3, "A3064");
        s.connected = true;
        s.device_name = "Josh's AirPods".into();
        s.noise_mode = 3;
        s.adaptive_noise_level = 40;
        s.left = Pod { available: true, level: 80, charging: false, in_ear: true };
        s.right = Pod { available: true, level: 75, charging: true, in_ear: false };
        s.case_battery = Battery { available: true, level: 90, charging: false };
        s.lid_state = 1;
        s
    }

    #[test]
    fn shows_the_important_things() {
        let text = render(&pro3());
        assert!(text.starts_with("AirPods Pro 3: connected\n"));
        assert!(text.contains("Left    80%  (in ear)"));
        assert!(text.contains("Right   75%  (charging)"));
        assert!(text.contains("Case    90%"));
        assert!(text.contains("Listening mode  Adaptive"));
        assert!(text.contains("Adaptive level  40%"));
        assert!(text.contains("Case lid  closed"));
        assert!(!text.contains('{'));
    }

    #[test]
    fn disconnected_with_no_data_is_short_and_clear() {
        let text = render(&Status::new());
        assert!(text.starts_with("AirPods: not connected\n"));
        assert!(text.contains("Left    unknown"));
        assert!(!text.contains("Listening mode"));
        assert!(!text.contains("Case lid"));
    }

    #[test]
    fn headset_shows_one_battery_and_no_case() {
        let mut s = Status::new();
        s.set_model(Model::AirPodsMax2, "A3454");
        s.connected = true;
        s.headset = Battery { available: true, level: 55, charging: false };
        let text = render(&s);
        assert!(text.contains("Headset 55%"));
        assert!(!text.contains("Left"));
    }
}
