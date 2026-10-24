//! Advertisements can be sent by any device in range, so no byte sequence may crash the
//! parser, and no battery level may come out above 100.

use ble_adv::{decrypt_payload, payload_battery, verify_rpa, Advert};

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
fn advert_parse_accepts_any_bytes_and_never_exceeds_100() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    for len in 0..=60 {
        for _ in 0..2000 {
            let mut data: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
            if len > 0 && rng.byte().is_multiple_of(2) {
                data[0] = 0x07; // the type we accept, so more inputs get past the first check
            }
            if let Some(a) = Advert::parse(&data) {
                for level in [a.left_battery, a.right_battery, a.case_battery].into_iter().flatten() {
                    assert!(level <= 100, "level {level} from {data:?}");
                }
                let key = [rng.byte(); 16];
                if let Some(plain) = decrypt_payload(&a.encrypted, &key) {
                    for headset in [false, true] {
                        for primary_left in [false, true] {
                            let _ = payload_battery(&plain, primary_left, headset);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn address_checks_reject_garbage() {
    let irk = [7u8; 16];
    for text in ["", ":", "::::::", "AA:BB:CC:DD:EE", "AA:BB:CC:DD:EE:FF:11", "ZZ:BB:CC:DD:EE:FF", "é:é:é:é:é:é", "A:B:C:D:E:F"] {
        assert!(!verify_rpa(text, &irk), "{text}");
    }
    for len in [0, 1, 15, 17, 32] {
        assert_eq!(decrypt_payload(&vec![0u8; len], &irk), None);
    }
}
