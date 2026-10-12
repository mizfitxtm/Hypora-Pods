//! Classifies one packet received from the AirPods.

use crate::battery::{self, Reading};
use crate::control::{self, ID_CONVERSATIONAL_AWARENESS, ID_ONE_BUD_ANC};
use crate::ear::{self, EarStatus};
use crate::noise::NoiseMode;

pub const HANDSHAKE_ACK_PREFIX: [u8; 4] = [0x01, 0x00, 0x04, 0x00];
pub const FEATURES_ACK_PREFIX: [u8; 6] = [0x04, 0x00, 0x04, 0x00, 0x2b, 0x00];
pub const MAGIC_KEYS_PREFIX: [u8; 7] = [0x04, 0x00, 0x04, 0x00, 0x31, 0x00, 0x02];
pub const METADATA_PREFIX: [u8; 5] = [0x04, 0x00, 0x04, 0x00, 0x1d];

/// 16 secret bytes whose `Debug` output never shows them, so a stray `{:?}` in a log cannot leak keys.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Secret(pub [u8; 16]);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(redacted)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inbound {
    HandshakeAck,
    FeaturesAck,
    Battery(Vec<Reading>),
    Ear(EarStatus, EarStatus),
    Noise(NoiseMode),
    ConversationalAwareness(bool),
    OneBud(bool),
    /// Identity-resolving key and encryption key, 16 bytes each.
    MagicKeys { irk: Secret, enc_key: Secret },
    Metadata { device_name: String, model_number: String, manufacturer: String },
    Unknown,
}

/// Reads a null-terminated string at `*pos` and moves `*pos` past the terminator.
fn read_cstr(data: &[u8], pos: &mut usize) -> String {
    let start = (*pos).min(data.len());
    let end = data[start..].iter().position(|&b| b == 0).map_or(data.len(), |i| start + i);
    *pos = (end + 1).min(data.len());
    String::from_utf8_lossy(&data[start..end]).into_owned()
}

/// Two blocks, each `tag, length (2 bytes, big-endian), 1 reserved byte, 16 bytes of key`.
/// Tag 0x01 is the IRK and tag 0x04 the encryption key; anything else rejects the packet.
fn parse_magic_keys(data: &[u8]) -> Option<Inbound> {
    let mut pos = MAGIC_KEYS_PREFIX.len();
    let mut block = |tag: u8| -> Option<[u8; 16]> {
        let head = data.get(pos..pos + 4)?;
        if head[0] != tag || u16::from_be_bytes([head[1], head[2]]) != 16 {
            return None;
        }
        let key = data.get(pos + 4..pos + 20)?.try_into().ok()?;
        pos += 20;
        Some(key)
    };
    let irk = block(0x01)?;
    let enc_key = block(0x04)?;
    Some(Inbound::MagicKeys { irk: Secret(irk), enc_key: Secret(enc_key) })
}

/// Metadata layout: prefix, 6 skipped bytes, then three null-terminated strings.
fn parse_metadata(data: &[u8]) -> Option<Inbound> {
    if data.len() < METADATA_PREFIX.len() + 6 {
        return None;
    }
    let mut pos = METADATA_PREFIX.len() + 6;
    let device_name = read_cstr(data, &mut pos);
    let model_number = read_cstr(data, &mut pos);
    let manufacturer = read_cstr(data, &mut pos);
    Some(Inbound::Metadata { device_name, model_number, manufacturer })
}

pub fn classify(data: &[u8]) -> Inbound {
    if data.starts_with(&HANDSHAKE_ACK_PREFIX) {
        return Inbound::HandshakeAck;
    }
    if data.starts_with(&FEATURES_ACK_PREFIX) {
        return Inbound::FeaturesAck;
    }
    if data.starts_with(&MAGIC_KEYS_PREFIX) {
        return parse_magic_keys(data).unwrap_or(Inbound::Unknown);
    }
    if data.starts_with(&METADATA_PREFIX) {
        return parse_metadata(data).unwrap_or(Inbound::Unknown);
    }
    if let Some((a, b)) = ear::parse(data) {
        return Inbound::Ear(a, b);
    }
    if let Some(readings) = battery::parse(data) {
        return Inbound::Battery(readings);
    }
    if let Some(mode) = NoiseMode::parse(data) {
        return Inbound::Noise(mode);
    }
    if let Some(on) = control::parse_toggle(ID_CONVERSATIONAL_AWARENESS, data) {
        return Inbound::ConversationalAwareness(on);
    }
    if let Some(on) = control::parse_toggle(ID_ONE_BUD_ANC, data) {
        return Inbound::OneBud(on);
    }
    Inbound::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_control_and_acks() {
        assert_eq!(classify(&NoiseMode::Adaptive.packet()), Inbound::Noise(NoiseMode::Adaptive));
        assert_eq!(classify(&control::one_bud_anc(true)), Inbound::OneBud(true));
        assert_eq!(classify(&control::conversational_awareness(false)), Inbound::ConversationalAwareness(false));
        assert_eq!(classify(&[1, 0, 4, 0, 0, 0, 0, 0]), Inbound::HandshakeAck);
        assert_eq!(classify(&[4, 0, 4, 0, 0x2b, 0, 1]), Inbound::FeaturesAck);
        assert_eq!(classify(&[9, 9]), Inbound::Unknown);
    }

    #[test]
    fn parses_metadata_strings() {
        let mut p = METADATA_PREFIX.to_vec();
        p.extend_from_slice(&[0; 6]);
        p.extend_from_slice(b"My Pods\0A3064\0Apple Inc.\0rest");
        assert_eq!(
            classify(&p),
            Inbound::Metadata { device_name: "My Pods".into(), model_number: "A3064".into(), manufacturer: "Apple Inc.".into() }
        );
    }

    fn keys_packet(tag2: u8, len2: u16) -> Vec<u8> {
        let mut p = MAGIC_KEYS_PREFIX.to_vec();
        p.extend_from_slice(&[0x01, 0x00, 0x10, 0x00]);
        p.extend_from_slice(&[0x11; 16]);
        p.push(tag2);
        p.extend_from_slice(&len2.to_be_bytes());
        p.push(0x00);
        p.extend_from_slice(&[0x22; 16]);
        p
    }

    #[test]
    fn parses_magic_keys() {
        assert_eq!(classify(&keys_packet(0x04, 16)), Inbound::MagicKeys { irk: Secret([0x11; 16]), enc_key: Secret([0x22; 16]) });
    }

    #[test]
    fn keys_never_appear_in_debug_output() {
        let text = format!("{:?}", classify(&keys_packet(0x04, 16)));
        assert!(text.contains("redacted") && !text.contains("17") && !text.contains("34"), "{text}");
    }

    #[test]
    fn rejects_bad_magic_keys() {
        assert_eq!(classify(&keys_packet(0x05, 16)), Inbound::Unknown);
        assert_eq!(classify(&keys_packet(0x04, 15)), Inbound::Unknown);
        let short = keys_packet(0x04, 16);
        assert_eq!(classify(&short[..short.len() - 1]), Inbound::Unknown);
    }

    #[test]
    fn short_or_truncated_metadata_is_safe() {
        assert_eq!(classify(&METADATA_PREFIX), Inbound::Unknown);
        let mut p = METADATA_PREFIX.to_vec();
        p.extend_from_slice(&[0; 6]);
        p.extend_from_slice(b"Only name");
        assert_eq!(
            classify(&p),
            Inbound::Metadata { device_name: "Only name".into(), model_number: String::new(), manufacturer: String::new() }
        );
    }
}
