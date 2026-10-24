//! Apple "proximity pairing" BLE advertisements: who sent it, and what it says.
//! Pure functions, no I/O. Derived from librepods by Kavish Devar (GPL-3.0).

#![forbid(unsafe_code)]

use aes::cipher::{Block, BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use aes::Aes128;

pub const APPLE_COMPANY_ID: u16 = 0x004C;

fn reversed<const N: usize>(bytes: &[u8; N]) -> [u8; N] {
    let mut out = *bytes;
    out.reverse();
    out
}

/// Bluetooth's `e` function: AES-128 on one block, with key and data in
/// big-endian, while we hold them little-endian, hence the reversals.
fn e(key: &[u8; 16], data: &[u8; 16]) -> [u8; 16] {
    let cipher = Aes128::new(&reversed(key).into());
    let mut block = Block::<Aes128>::from(reversed(data));
    cipher.encrypt_block(&mut block);
    let mut out = [0u8; 16];
    out.copy_from_slice(block.as_slice());
    reversed(&out)
}

/// Bluetooth's `ah` function: the 3-byte hash of a resolvable private address.
fn ah(irk: &[u8; 16], prand: &[u8]) -> [u8; 3] {
    let mut padded = [0u8; 16];
    padded[..3].copy_from_slice(&prand[..3]);
    let enc = e(irk, &padded);
    [enc[0], enc[1], enc[2]]
}

/// True if `address` ("AA:BB:CC:DD:EE:FF") is a resolvable private address made from `irk`.
/// This is how we know an advertisement belongs to *our* AirPods, since their address rotates.
pub fn verify_rpa(address: &str, irk: &[u8; 16]) -> bool {
    let parts: Vec<&str> = address.split(':').collect();
    if parts.len() != 6 {
        return false;
    }
    let mut rpa = [0u8; 6];
    for (slot, part) in rpa.iter_mut().zip(parts.iter().rev()) {
        match u8::from_str_radix(part, 16) {
            Ok(b) if part.len() == 2 => *slot = b,
            _ => return false,
        }
    }
    ah(irk, &rpa[3..6]) == rpa[..3]
}

/// Decrypts the 16-byte encrypted tail of an advertisement.
/// One block of AES-CBC with a zero IV is the same as AES-ECB on that block.
pub fn decrypt_payload(encrypted: &[u8], key: &[u8; 16]) -> Option<[u8; 16]> {
    if encrypted.len() != 16 {
        return None;
    }
    let cipher = Aes128::new(&(*key).into());
    let mut block = Block::<Aes128>::try_from(encrypted).ok()?;
    cipher.decrypt_block(&mut block);
    let mut out = [0u8; 16];
    out.copy_from_slice(block.as_slice());
    Some(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lid {
    Open,
    Closed,
    Unknown,
}

/// The readable part of an advertisement (everything except the encrypted tail).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advert {
    /// Big-endian model id, e.g. 0x2720 for AirPods Pro 3.
    pub model_id: u16,
    pub primary_left: bool,
    pub this_pod_in_case: bool,
    pub left_battery: Option<u8>,
    pub right_battery: Option<u8>,
    pub case_battery: Option<u8>,
    pub left_charging: bool,
    pub right_charging: bool,
    pub case_charging: bool,
    pub left_in_ear: bool,
    pub right_in_ear: bool,
    pub lid: Lid,
    pub encrypted: [u8; 16],
}

impl Advert {
    /// Parses Apple manufacturer data. `None` for anything that is not a
    /// proximity-pairing message in the normal (not pairing-mode) layout.
    pub fn parse(data: &[u8]) -> Option<Advert> {
        if data.len() < 11 + 16 || data[0] != 0x07 || data[2] == 0x00 {
            return None;
        }
        let status = data[5];
        let pods_battery = data[6];
        let flags_and_case = data[7];
        let lid_indicator = data[8];

        let primary_left = status & 0x20 != 0;
        let flipped = !primary_left;
        // 0-10 are tenths of a full battery; 15 means "unknown". 11-14 are not valid, so they are
        // dropped rather than shown as a level above 100%.
        let nibble = |n: u8| if n <= 10 { Some(n * 10) } else { None };

        let (left_nibble, right_nibble) = if flipped {
            (pods_battery >> 4, pods_battery & 0x0F)
        } else {
            (pods_battery & 0x0F, pods_battery >> 4)
        };
        let flags = flags_and_case >> 4;
        let (left_charging, right_charging) = if flipped {
            (flags & 0x02 != 0, flags & 0x01 != 0)
        } else {
            (flags & 0x01 != 0, flags & 0x02 != 0)
        };

        let this_pod_in_case = status & 0x40 != 0;
        let swap = flipped ^ this_pod_in_case;
        let (left_in_ear, right_in_ear) = if swap {
            (status & 0x08 != 0, status & 0x02 != 0)
        } else {
            (status & 0x02 != 0, status & 0x08 != 0)
        };

        // The lid bit only means something when this pod reports from inside the case.
        let lid = if !this_pod_in_case {
            Lid::Unknown
        } else if (lid_indicator >> 3) & 1 == 0 {
            Lid::Open
        } else {
            Lid::Closed
        };

        let mut encrypted = [0u8; 16];
        encrypted.copy_from_slice(&data[data.len() - 16..]);

        Some(Advert {
            model_id: u16::from_be_bytes([data[3], data[4]]),
            primary_left,
            this_pod_in_case,
            left_battery: nibble(left_nibble & 0x0F),
            right_battery: nibble(right_nibble & 0x0F),
            case_battery: nibble(flags_and_case & 0x0F),
            left_charging,
            right_charging,
            case_charging: flags & 0x04 != 0,
            left_in_ear,
            right_in_ear,
            lid,
            encrypted,
        })
    }
}

/// One battery reading from the decrypted payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub level: u8,
    pub charging: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PayloadBattery {
    pub left: Option<Level>,
    pub right: Option<Level>,
    pub case: Option<Level>,
    /// Set instead of left/right for a headset such as AirPods Max.
    pub headset: Option<Level>,
}

fn level_of(byte: u8) -> Option<Level> {
    let level = byte & 0x7F;
    // 0x7F means the pod has not reported; callers keep the last value.
    if level > 100 { None } else { Some(Level { level, charging: byte & 0x80 != 0 }) }
}

/// Battery levels from the decrypted 16 bytes. Byte 1 and 2 hold the pods
/// (primary first), byte 3 the case; a headset uses byte 1 alone.
pub fn payload_battery(decrypted: &[u8; 16], primary_left: bool, headset: bool) -> PayloadBattery {
    if headset {
        return PayloadBattery { headset: level_of(decrypted[1]), ..Default::default() };
    }
    let (left, right) = if primary_left { (1, 2) } else { (2, 1) };
    PayloadBattery {
        left: level_of(decrypted[left]),
        right: level_of(decrypted[right]),
        case: level_of(decrypted[3]),
        headset: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Bluetooth Core spec, Vol 3 Part H, appendix D.7: ah(k, r) = 0dfbaa.
    // The spec prints big-endian; we hold little-endian, so reverse.
    const SPEC_IRK_BE: [u8; 16] = [0xec, 0x02, 0x34, 0xa3, 0x57, 0xc8, 0xad, 0x05, 0x34, 0x10, 0x10, 0xa6, 0x0a, 0x39, 0x7d, 0x9b];

    #[test]
    fn rpa_matches_the_spec_sample() {
        let irk = reversed(&SPEC_IRK_BE);
        assert!(verify_rpa("70:81:94:0D:FB:AA", &irk));
        assert!(!verify_rpa("70:81:94:0D:FB:AB", &irk));
        let mut other = irk;
        other[0] ^= 1;
        assert!(!verify_rpa("70:81:94:0D:FB:AA", &other));
    }

    #[test]
    fn rpa_rejects_malformed_addresses() {
        let irk = [0u8; 16];
        for bad in ["", "70:81:94:0D:FB", "70:81:94:0D:FB:AA:00", "70:81:94:0D:FB:ZZ", "7:81:94:0D:FB:AA"] {
            assert!(!verify_rpa(bad, &irk), "{bad}");
        }
    }

    #[test]
    fn decrypt_inverts_aes() {
        // FIPS-197 appendix C.1: key 000102..0f, plaintext 00112233..ff.
        let key: [u8; 16] = core::array::from_fn(|i| i as u8);
        let cipher: [u8; 16] = [0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4, 0xc5, 0x5a];
        let plain: [u8; 16] = core::array::from_fn(|i| (i * 0x11) as u8);
        assert_eq!(decrypt_payload(&cipher, &key), Some(plain));
        assert_eq!(decrypt_payload(&cipher[..15], &key), None);
    }

    fn advert(status: u8, pods: u8, flags_case: u8, lid: u8) -> Vec<u8> {
        let mut d = vec![0x07, 0x19, 0x01, 0x27, 0x20, status, pods, flags_case, lid, 0x00, 0x05];
        d.extend_from_slice(&[0xAA; 16]);
        d
    }

    #[test]
    fn parses_batteries_and_flags() {
        // primary left (0x20), left nibble 8, right nibble 6, case nibble 9, flags: left charging + case charging
        let a = Advert::parse(&advert(0x20, 0x68, 0x59, 0x00)).unwrap();
        assert_eq!(a.model_id, 0x2720);
        assert_eq!((a.left_battery, a.right_battery, a.case_battery), (Some(80), Some(60), Some(90)));
        assert!(a.left_charging && !a.right_charging && a.case_charging);
        assert_eq!(a.encrypted, [0xAA; 16]);
    }

    #[test]
    fn flipped_primary_swaps_pods() {
        // Right pod primary (bit 5 clear): the low nibble belongs to the right... per reference: left = high nibble.
        let a = Advert::parse(&advert(0x00, 0x68, 0x0F, 0x00)).unwrap();
        assert_eq!((a.left_battery, a.right_battery), (Some(60), Some(80)));
        assert_eq!(a.case_battery, None); // nibble 15 means unknown
    }

    #[test]
    fn lid_only_known_from_inside_the_case() {
        let out = Advert::parse(&advert(0x20, 0x88, 0x00, 0x08)).unwrap();
        assert_eq!(out.lid, Lid::Unknown);
        let open = Advert::parse(&advert(0x60, 0x88, 0x00, 0x00)).unwrap();
        assert_eq!(open.lid, Lid::Open);
        let closed = Advert::parse(&advert(0x60, 0x88, 0x00, 0x08)).unwrap();
        assert_eq!(closed.lid, Lid::Closed);
    }

    #[test]
    fn rejects_other_messages() {
        assert!(Advert::parse(&[]).is_none());
        let mut wrong_type = advert(0x20, 0x88, 0, 0);
        wrong_type[0] = 0x10;
        assert!(Advert::parse(&wrong_type).is_none());
        let mut pairing_mode = advert(0x20, 0x88, 0, 0);
        pairing_mode[2] = 0x00;
        assert!(Advert::parse(&pairing_mode).is_none());
        assert!(Advert::parse(&advert(0, 0, 0, 0)[..20]).is_none());
    }

    #[test]
    fn payload_levels() {
        let mut p = [0u8; 16];
        p[1] = 0x80 | 55; // charging, 55%
        p[2] = 70;
        p[3] = 0x7F; // unknown
        let b = payload_battery(&p, true, false);
        assert_eq!(b.left, Some(Level { level: 55, charging: true }));
        assert_eq!(b.right, Some(Level { level: 70, charging: false }));
        assert_eq!(b.case, None);
        let swapped = payload_battery(&p, false, false);
        assert_eq!(swapped.left.unwrap().level, 70);
        assert_eq!(payload_battery(&p, true, true).headset.unwrap().level, 55);
    }
}
