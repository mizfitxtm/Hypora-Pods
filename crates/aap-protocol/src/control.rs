//! Control commands: `04 00 04 00 09 00 <id> <d1> <d2> <d3> <d4>`.

pub const HEADER: [u8; 6] = [0x04, 0x00, 0x04, 0x00, 0x09, 0x00];

pub const ID_NOISE_MODE: u8 = 0x0D;
pub const ID_ONE_BUD_ANC: u8 = 0x1B;
pub const ID_CONVERSATIONAL_AWARENESS: u8 = 0x28;

pub fn command(id: u8, d1: u8, d2: u8, d3: u8, d4: u8) -> Vec<u8> {
    let mut p = HEADER.to_vec();
    p.extend_from_slice(&[id, d1, d2, d3, d4]);
    p
}

/// Byte 7 of a control packet, if the packet is one and is long enough.
pub fn parse_value(data: &[u8]) -> Option<u8> {
    if data.len() <= 7 || !data.starts_with(&HEADER) {
        return None;
    }
    Some(data[7])
}

/// A control packet's identifier byte (index 6).
pub fn parse_id(data: &[u8]) -> Option<u8> {
    parse_value(data).map(|_| data[6])
}

/// On/off toggle: 0x01 enabled, 0x02 disabled.
pub fn toggle(id: u8, enabled: bool) -> Vec<u8> {
    command(id, if enabled { 1 } else { 2 }, 0, 0, 0)
}

/// Reads a toggle for `id`; `None` for another command or an unknown value.
pub fn parse_toggle(id: u8, data: &[u8]) -> Option<bool> {
    if parse_id(data)? != id {
        return None;
    }
    match data[7] {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

/// Adaptive noise level (0-100).
pub fn adaptive_level(level: u8) -> Vec<u8> {
    let mut p = vec![0x04, 0x00, 0x04, 0x00, 0x09, 0x00, 0x2E];
    p.extend_from_slice(&[level.min(100), 0, 0, 0]);
    p
}

pub fn one_bud_anc(enabled: bool) -> Vec<u8> {
    toggle(ID_ONE_BUD_ANC, enabled)
}

pub fn conversational_awareness(enabled: bool) -> Vec<u8> {
    toggle(ID_CONVERSATIONAL_AWARENESS, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_roundtrip() {
        for on in [true, false] {
            assert_eq!(parse_toggle(ID_ONE_BUD_ANC, &one_bud_anc(on)), Some(on));
        }
    }

    #[test]
    fn toggle_wrong_id_or_short() {
        assert_eq!(parse_toggle(ID_ONE_BUD_ANC, &conversational_awareness(true)), None);
        assert_eq!(parse_toggle(ID_ONE_BUD_ANC, &HEADER), None);
        assert_eq!(parse_value(&[0x04, 0, 4, 0, 9, 0, 0x1B]), None);
    }

    #[test]
    fn known_bytes() {
        assert_eq!(
            conversational_awareness(true),
            [0x04, 0, 0x04, 0, 0x09, 0, 0x28, 0x01, 0, 0, 0]
        );
    }

    #[test]
    fn adaptive_clamps() {
        assert_eq!(adaptive_level(200)[7], 100);
        assert_eq!(adaptive_level(40).len(), 11);
    }
}
