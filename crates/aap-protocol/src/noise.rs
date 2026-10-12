use crate::control::{self, ID_NOISE_MODE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseMode {
    Off = 0,
    NoiseCancellation = 1,
    Transparency = 2,
    Adaptive = 3,
}

impl NoiseMode {
    pub fn packet(self) -> Vec<u8> {
        control::command(ID_NOISE_MODE, self as u8 + 1, 0, 0, 0)
    }

    /// Wire value is mode + 1; anything outside 1..=4 is unknown.
    pub fn parse(data: &[u8]) -> Option<Self> {
        if control::parse_id(data)? != ID_NOISE_MODE {
            return None;
        }
        match data[7] {
            1 => Some(Self::Off),
            2 => Some(Self::NoiseCancellation),
            3 => Some(Self::Transparency),
            4 => Some(Self::Adaptive),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for m in [NoiseMode::Off, NoiseMode::NoiseCancellation, NoiseMode::Transparency, NoiseMode::Adaptive] {
            assert_eq!(NoiseMode::parse(&m.packet()), Some(m));
        }
    }

    #[test]
    fn rejects_bad() {
        assert_eq!(NoiseMode::parse(&control::command(ID_NOISE_MODE, 9, 0, 0, 0)), None);
        assert_eq!(NoiseMode::parse(&control::command(0x28, 1, 0, 0, 0)), None);
    }
}
