//! Packets come from the AirPods over the air, so no byte sequence may crash a parser.
//! This feeds `classify` every length from 0 to 80 using each known packet prefix followed by
//! pseudo-random bytes. It cannot prove there is no bug, but it catches bad indexing.

use aap_protocol::inbound::{classify, FEATURES_ACK_PREFIX, HANDSHAKE_ACK_PREFIX, METADATA_PREFIX, MAGIC_KEYS_PREFIX};
use aap_protocol::{BATTERY_HEADER, EAR_DETECTION_HEADER};

/// A tiny deterministic generator (xorshift), so a failure can be repeated.
struct Rng(u64);
impl Rng {
    fn byte(&mut self) -> u8 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 24) as u8
    }
}

#[test]
fn classify_accepts_any_bytes() {
    let prefixes: [&[u8]; 7] = [
        &[],
        &HANDSHAKE_ACK_PREFIX,
        &FEATURES_ACK_PREFIX,
        &MAGIC_KEYS_PREFIX,
        &METADATA_PREFIX,
        &EAR_DETECTION_HEADER,
        &BATTERY_HEADER,
    ];
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for prefix in prefixes {
        for len in 0..=80 {
            for _ in 0..200 {
                let mut data = prefix.to_vec();
                data.truncate(len);
                while data.len() < len {
                    data.push(rng.byte());
                }
                let _ = classify(&data);
            }
        }
    }
}
