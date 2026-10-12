use crate::EAR_DETECTION_HEADER;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EarStatus {
    InEar,
    NotInEar,
    InCase,
    Disconnected,
}

impl EarStatus {
    fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::InEar,
            1 => Self::NotInEar,
            2 => Self::InCase,
            _ => Self::Disconnected,
        }
    }
}

/// Primary and secondary pod status from an 8-byte ear detection packet.
pub fn parse(data: &[u8]) -> Option<(EarStatus, EarStatus)> {
    if data.len() != 8 || !data.starts_with(&EAR_DETECTION_HEADER) {
        return None;
    }
    Some((EarStatus::from_byte(data[6]), EarStatus::from_byte(data[7])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        let p = [0x04, 0, 0x04, 0, 0x06, 0, 0x00, 0x02];
        assert_eq!(parse(&p), Some((EarStatus::InEar, EarStatus::InCase)));
        assert_eq!(parse(&[0x04, 0, 0x04, 0, 0x06, 0, 0x07, 0x01]).unwrap().0, EarStatus::Disconnected);
    }

    #[test]
    fn rejects() {
        assert_eq!(parse(&[0x04, 0, 0x04, 0, 0x06, 0, 0]), None);
        assert_eq!(parse(&[0x04, 0, 0x04, 0, 0x05, 0, 0, 0]), None);
    }
}
