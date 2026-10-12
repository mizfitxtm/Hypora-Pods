use crate::BATTERY_HEADER;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Component {
    Headset,
    Right,
    Left,
    Case,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeStatus {
    Charging,
    Discharging,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub component: Component,
    pub level: u8,
    pub status: ChargeStatus,
}

/// Parses a battery packet: header, count (<= 3), then 5 bytes per entry
/// `type 01 level status 01`. Any malformed entry rejects the whole packet.
/// Disconnected entries are returned; callers decide to keep the old level.
pub fn parse(data: &[u8]) -> Option<Vec<Reading>> {
    if data.len() < 7 || !data.starts_with(&BATTERY_HEADER) {
        return None;
    }
    let count = data[6] as usize;
    if count > 3 || data.len() != 7 + 5 * count {
        return None;
    }
    data[7..]
        .chunks_exact(5)
        .map(|e| {
            if e[1] != 1 || e[4] != 1 {
                return None;
            }
            let component = match e[0] {
                0x01 => Component::Headset,
                0x02 => Component::Right,
                0x04 => Component::Left,
                0x08 => Component::Case,
                _ => return None,
            };
            let status = match e[3] {
                0x01 => ChargeStatus::Charging,
                0x02 => ChargeStatus::Discharging,
                0x04 => ChargeStatus::Disconnected,
                _ => return None,
            };
            Some(Reading { component, level: e[2], status })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(entries: &[[u8; 5]]) -> Vec<u8> {
        let mut p = BATTERY_HEADER.to_vec();
        p.push(entries.len() as u8);
        for e in entries {
            p.extend_from_slice(e);
        }
        p
    }

    #[test]
    fn three_components() {
        let p = packet(&[[4, 1, 80, 2, 1], [2, 1, 75, 1, 1], [8, 1, 100, 1, 1]]);
        let r = parse(&p).unwrap();
        assert_eq!(r[0], Reading { component: Component::Left, level: 80, status: ChargeStatus::Discharging });
        assert_eq!(r[1].component, Component::Right);
        assert_eq!(r[2].status, ChargeStatus::Charging);
    }

    #[test]
    fn rejects_malformed() {
        assert!(parse(&packet(&[[4, 0, 80, 2, 1]])).is_none());
        assert!(parse(&packet(&[[9, 1, 80, 2, 1]])).is_none());
        let mut p = packet(&[[4, 1, 80, 2, 1]]);
        p.pop();
        assert!(parse(&p).is_none());
        let mut q = BATTERY_HEADER.to_vec();
        q.push(4);
        assert!(parse(&q).is_none());
    }

    #[test]
    fn empty_ok() {
        assert_eq!(parse(&packet(&[])), Some(vec![]));
    }
}
