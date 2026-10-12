//! Applies control verbs to the status. In this stub phase nothing talks to
//! real AirPods; the same function will later decide what packet to send.

use aap_protocol::battery::{ChargeStatus, Component};
use aap_protocol::ear::EarStatus;
use aap_protocol::inbound::Inbound;
use crate::keys::Keys;
use aap_protocol::ipc::Verb;
use aap_protocol::model::Model;
use aap_protocol::noise::NoiseMode;
use aap_protocol::status::{Pod, Status};

/// A device with sample data, so the widget has something to draw.
pub fn sample(model_number: &str) -> Status {
    let mut s = Status::new();
    let model = aap_protocol::model::Model::from_model_number(model_number);
    s.set_model(model, model_number);
    s.connected = true;
    s.device_name = "AirPods (stub)".into();
    s.noise_mode = if model.supports_noise_control() { NoiseMode::NoiseCancellation as i32 } else { -1 };
    for (pod, level) in [(&mut s.left, 80), (&mut s.right, 75)] {
        pod.available = true;
        pod.level = level;
        pod.in_ear = true;
    }
    s.case_battery.available = !model.is_headset();
    s.case_battery.level = 90;
    s.headset.available = model.is_headset();
    s.headset.level = 60;
    s
}

/// What we remember between packets to place each pod correctly.
/// The pods number themselves "primary" and "secondary"; battery packets tell us which is left.
pub struct Live {
    pub primary_is_left: bool,
    pub ear: Option<(EarStatus, EarStatus)>,
}

impl Default for Live {
    fn default() -> Self {
        Live { primary_is_left: true, ear: None }
    }
}

fn place_ears(status: &mut Status, live: &Live) {
    let Some((primary, secondary)) = live.ear else { return };
    let (left, right) = if live.primary_is_left { (primary, secondary) } else { (secondary, primary) };
    status.left.in_ear = left == EarStatus::InEar;
    status.right.in_ear = right == EarStatus::InEar;
}

/// Folds one packet from the pods into the status.
pub fn apply_inbound(status: &mut Status, live: &mut Live, msg: Inbound) {
    match msg {
        Inbound::Battery(readings) => {
            // The first pod listed is the primary.
            if let Some(first) = readings.iter().find(|r| matches!(r.component, Component::Left | Component::Right)) {
                live.primary_is_left = first.component == Component::Left;
            }
            for r in readings {
                // A disconnected entry means "no new reading": keep the last known level.
                if r.status == ChargeStatus::Disconnected {
                    continue;
                }
                let charging = r.status == ChargeStatus::Charging;
                match r.component {
                    Component::Left => (status.left.available, status.left.level, status.left.charging) = (true, r.level, charging),
                    Component::Right => (status.right.available, status.right.level, status.right.charging) = (true, r.level, charging),
                    Component::Case => (status.case_battery.available, status.case_battery.level, status.case_battery.charging) = (true, r.level, charging),
                    Component::Headset => (status.headset.available, status.headset.level, status.headset.charging) = (true, r.level, charging),
                }
            }
            place_ears(status, live);
        }
        Inbound::Ear(primary, secondary) => {
            live.ear = Some((primary, secondary));
            place_ears(status, live);
        }
        Inbound::Noise(mode) => status.noise_mode = mode as i32,
        Inbound::ConversationalAwareness(on) => status.conversational_awareness = on,
        Inbound::OneBud(on) => status.one_bud_anc_mode = on,
        Inbound::Metadata { device_name, model_number, .. } => {
            status.set_model(Model::from_model_number(&model_number), &model_number);
            let device_name = aap_protocol::status::sanitize_name(&device_name);
            if !device_name.is_empty() {
                status.device_name = device_name;
            }
        }
        Inbound::HandshakeAck | Inbound::FeaturesAck | Inbound::MagicKeys { .. } | Inbound::Unknown => {}
    }
}

fn model_from_ble_id(id: u16) -> Model {
    match id {
        0x0220 => Model::AirPods1,
        0x0F20 => Model::AirPods2,
        0x1320 => Model::AirPods3,
        0x1920 => Model::AirPods4,
        0x1B20 => Model::AirPods4Anc,
        0x0A20 => Model::AirPodsMaxLightning,
        0x1F20 => Model::AirPodsMaxUsbC,
        0x0E20 => Model::AirPodsPro,
        0x1420 => Model::AirPodsPro2Lightning,
        0x2420 => Model::AirPodsPro2UsbC,
        0x2720 => Model::AirPodsPro3,
        _ => Model::Unknown,
    }
}

/// Folds a BLE advertisement into the status. Adverts not made from our IRK
/// (other people's AirPods, rotated addresses) are ignored.
pub fn apply_advert(status: &mut Status, live: &mut Live, keys: &Keys, address: &str, data: &[u8]) {
    use ble_adv::{decrypt_payload, payload_battery, verify_rpa, Lid};
    if !verify_rpa(address, &keys.irk) {
        return;
    }
    let Some(advert) = ble_adv::Advert::parse(data) else { return };

    // Only fill the model in when the control link has not named it already.
    if status.model_number.is_empty() {
        let model = model_from_ble_id(advert.model_id);
        if model != Model::Unknown {
            status.set_model(model, "");
        }
    }
    match advert.lid {
        Lid::Open => status.lid_state = 0,
        Lid::Closed => status.lid_state = 1,
        Lid::Unknown => {}
    }

    live.primary_is_left = advert.primary_left;
    if let Some(payload) = decrypt_payload(&advert.encrypted, &keys.enc_key) {
        let b = payload_battery(&payload, advert.primary_left, status.is_headset);
        let set = |slot: &mut Pod, level: Option<ble_adv::Level>| {
            if let Some(l) = level {
                (slot.available, slot.level, slot.charging) = (true, l.level, l.charging);
            }
        };
        set(&mut status.left, b.left);
        set(&mut status.right, b.right);
        if let Some(l) = b.headset {
            (status.headset.available, status.headset.level, status.headset.charging) = (true, l.level, l.charging);
        }
        if let Some(l) = b.case {
            (status.case_battery.available, status.case_battery.level, status.case_battery.charging) = (true, l.level, l.charging);
        }
    }
    // The case level in the readable part is fresher, so it wins when present.
    if !status.is_headset {
        if let Some(level) = advert.case_battery {
            (status.case_battery.available, status.case_battery.level, status.case_battery.charging) = (true, level, advert.case_charging);
        }
    }
    status.left.in_ear = advert.left_in_ear;
    status.right.in_ear = advert.right_in_ear;
}

/// `Ok(())` if applied. `Err` carries a message for the client.
/// `Verb::Status` is handled by the caller, since it changes nothing.
pub fn apply(status: &mut Status, verb: Verb) -> Result<(), &'static str> {
    match verb {
        Verb::Status => {}
        Verb::Noise(mode) => {
            let allowed = match mode {
                NoiseMode::Off => status.supports_noise_off,
                NoiseMode::Adaptive => status.supports_adaptive,
                _ => true,
            };
            if !status.supports_noise_control || !allowed {
                return Err("this device does not support that mode");
            }
            status.noise_mode = mode as i32;
        }
        Verb::Ear(behavior) => status.ear_detection_behavior = behavior as u8,
        Verb::ConversationalAwareness(on) => {
            if !status.supports_conversational_awareness {
                return Err("conversational awareness is not supported");
            }
            status.conversational_awareness = on;
        }
        Verb::OneBud(on) => {
            if !status.supports_one_bud_anc {
                return Err("one-bud ANC is not supported");
            }
            status.one_bud_anc_mode = on;
        }
        Verb::Adaptive(level) => {
            if !status.supports_adaptive {
                return Err("adaptive noise is not supported");
            }
            status.adaptive_noise_level = level;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aap_protocol::ipc::EarBehavior;

    #[test]
    fn pro3_rejects_off_accepts_adaptive() {
        let mut s = sample("A3064");
        assert!(apply(&mut s, Verb::Noise(NoiseMode::Off)).is_err());
        assert_eq!(s.noise_mode, 1);
        assert!(apply(&mut s, Verb::Noise(NoiseMode::Adaptive)).is_ok());
        assert_eq!(s.noise_mode, 3);
    }

    #[test]
    fn max2_rejects_one_bud() {
        let mut s = sample("A3454");
        assert!(apply(&mut s, Verb::OneBud(true)).is_err());
        assert!(s.headset.available && !s.case_battery.available);
    }

    #[test]
    fn battery_and_ears_follow_the_primary_pod() {
        use aap_protocol::battery::Reading;
        let mut s = Status::new();
        let mut live = Live::default();
        let r = |component, level, status| Reading { component, level, status };
        // Right pod listed first, so it is the primary.
        apply_inbound(&mut s, &mut live, Inbound::Battery(vec![
            r(Component::Right, 70, ChargeStatus::Discharging),
            r(Component::Left, 60, ChargeStatus::Charging),
            r(Component::Case, 0, ChargeStatus::Disconnected),
        ]));
        assert_eq!((s.right.level, s.left.level, s.left.charging), (70, 60, true));
        assert!(!s.case_battery.available); // disconnected entry ignored
        apply_inbound(&mut s, &mut live, Inbound::Ear(EarStatus::InEar, EarStatus::InCase));
        assert!(s.right.in_ear && !s.left.in_ear);
    }

    // Spec sample IRK (little-endian) and a FIPS-197 key/ciphertext pair that decrypts to 00 11 22 33 ...
    fn test_keys() -> Keys {
        let mut irk = [0xec, 0x02, 0x34, 0xa3, 0x57, 0xc8, 0xad, 0x05, 0x34, 0x10, 0x10, 0xa6, 0x0a, 0x39, 0x7d, 0x9b];
        irk.reverse();
        Keys { irk, enc_key: core::array::from_fn(|i| i as u8) }
    }

    fn test_advert(status_byte: u8, lid: u8) -> Vec<u8> {
        let mut d = vec![0x07, 0x19, 0x01, 0x27, 0x20, status_byte, 0x88, 0x59, lid, 0x00, 0x05];
        d.extend_from_slice(&[0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4, 0xc5, 0x5a]);
        d
    }

    #[test]
    fn advert_from_our_pods_updates_lid_and_battery() {
        let mut s = Status::new();
        let mut live = Live::default();
        // 0x60: primary left, this pod in the case. Lid bit set (0x08) means closed.
        apply_advert(&mut s, &mut live, &test_keys(), "70:81:94:0D:FB:AA", &test_advert(0x60, 0x08));
        assert_eq!(s.lid_state, 1);
        assert_eq!((s.left.level, s.right.level), (17, 34));
        assert_eq!(s.case_battery.level, 90); // readable nibble 9 wins over the decrypted 51
        assert!(s.case_battery.charging);
        assert_eq!(s.model_name, "AirPods Pro 3");
    }

    #[test]
    fn advert_from_other_pods_is_ignored() {
        let mut s = Status::new();
        apply_advert(&mut s, &mut Live::default(), &test_keys(), "70:81:94:0D:FB:AB", &test_advert(0x60, 0x08));
        assert_eq!(s, Status::new());
    }

    #[test]
    fn metadata_sets_model() {
        let mut s = Status::new();
        apply_inbound(&mut s, &mut Live::default(), Inbound::Metadata {
            device_name: "Pods".into(), model_number: "A3064".into(), manufacturer: "Apple Inc.".into(),
        });
        assert_eq!(s.model_name, "AirPods Pro 3");
        assert!(!s.supports_noise_off);
    }

    #[test]
    fn ear_always_applies() {
        let mut s = sample("A3050");
        assert!(apply(&mut s, Verb::Ear(EarBehavior::Never)).is_ok());
        assert_eq!(s.ear_detection_behavior, 2);
    }
}
