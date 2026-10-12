//! The `status.json` wire format shared by the daemon and the widget.

use crate::model::Model;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

/// Names come from the other end of a Bluetooth link, so they are untrusted: strip
/// control characters (terminal escapes, newlines in logs) and cap the length.
pub fn sanitize_name(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(64).collect::<String>().trim().to_string()
}

/// One pod. `Default` gives all-zero/false, which is what a fresh daemon shows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pod {
    pub available: bool,
    pub level: u8,
    pub charging: bool,
    pub in_ear: bool,
}

/// Case and headset have no `in_ear`, so they get their own type.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Battery {
    pub available: bool,
    pub level: u8,
    pub charging: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Status {
    pub schema_version: u32,
    pub connected: bool,
    pub device_name: String,
    /// 0 Off, 1 NC, 2 Transparency, 3 Adaptive, -1 unknown.
    pub noise_mode: i32,
    pub left: Pod,
    pub right: Pod,
    #[serde(rename = "case")] // `case` is not a Rust keyword, but this shows how to rename a key
    pub case_battery: Battery,
    pub headset: Battery,
    pub conversational_awareness: bool,
    pub adaptive_noise_level: u8,
    pub one_bud_anc_mode: bool,
    pub model_name: String,
    pub model_number: String,
    pub is_pro_series: bool,
    pub is_headset: bool,
    pub supports_noise_off: bool,
    pub supports_noise_control: bool,
    pub supports_adaptive: bool,
    pub supports_conversational_awareness: bool,
    pub supports_one_bud_anc: bool,
    /// 0 pause when one is out, 1 when both are out, 2 never.
    pub ear_detection_behavior: u8,
    /// 0 open, 1 closed, 2 unknown.
    pub lid_state: u8,
}

impl Status {
    /// The state before any device is known. Mirrors the reference daemon's defaults.
    pub fn new() -> Self {
        let mut s = Status {
            schema_version: SCHEMA_VERSION,
            connected: false,
            device_name: String::new(),
            noise_mode: -1,
            left: Pod::default(),
            right: Pod::default(),
            case_battery: Battery::default(),
            headset: Battery::default(),
            conversational_awareness: false,
            adaptive_noise_level: 0,
            one_bud_anc_mode: false,
            model_name: String::new(),
            model_number: String::new(),
            is_pro_series: false,
            is_headset: false,
            supports_noise_off: true,
            supports_noise_control: true,
            supports_adaptive: false,
            supports_conversational_awareness: false,
            supports_one_bud_anc: false,
            ear_detection_behavior: 0,
            lid_state: 2,
        };
        s.set_model(Model::Unknown, "");
        s
    }

    /// Copies every model-derived field in one place so they cannot drift apart.
    /// `&mut self` borrows the struct mutably: we may change it, but not take ownership.
    pub fn set_model(&mut self, model: Model, number: &str) {
        self.model_name = model.display_name().to_string();
        self.model_number = number.to_string();
        self.is_pro_series = model.is_pro_series();
        self.is_headset = model.is_headset();
        self.supports_noise_off = model.supports_noise_off();
        self.supports_noise_control = model.supports_noise_control();
        self.supports_adaptive = model.supports_adaptive();
        self.supports_conversational_awareness = model.supports_conversational_awareness();
        self.supports_one_bud_anc = model.supports_one_bud_anc();
    }

    /// One compact line plus newline, as the widget expects.
    pub fn to_line(&self) -> String {
        // `expect` panics if serialization fails; for plain data like this it cannot.
        let mut line = serde_json::to_string(self).expect("Status always serializes");
        line.push('\n');
        line
    }
}

impl Default for Status {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_and_renames_case() {
        let mut s = Status::new();
        s.set_model(Model::from_model_number("A3064"), "A3064");
        s.case_battery.level = 90;
        let line = s.to_line();
        assert!(line.ends_with('\n') && line.contains("\"case\":{"));
        assert!(!line.contains("case_battery"));
        let back: Status = serde_json::from_str(&line).unwrap();
        assert_eq!(back, s);
        assert!(!back.supports_noise_off);
    }

    #[test]
    fn names_lose_control_characters_and_length() {
        assert_eq!(sanitize_name("Pods\x1b[31m\nEvil"), "Pods[31mEvil");
        assert_eq!(sanitize_name("  Josh's AirPods \u{7} "), "Josh's AirPods");
        assert_eq!(sanitize_name(&"x".repeat(200)).len(), 64);
        assert_eq!(sanitize_name(""), "");
    }

    #[test]
    fn fresh_status_defaults() {
        let s = Status::new();
        assert_eq!(s.noise_mode, -1);
        assert_eq!(s.lid_state, 2);
        assert!(s.supports_noise_control && !s.supports_adaptive);
    }
}
