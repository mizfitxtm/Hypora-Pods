//! Which AirPods this is, and what it can do.

// `#[derive(...)]` asks the compiler to write trait implementations for us.
//   Debug      -> lets us print the value with {:?}
//   Clone/Copy -> cheap duplication; Copy means `let b = a;` leaves `a` usable
//   PartialEq/Eq -> lets us compare with ==
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    Unknown,
    AirPods1,
    AirPods2,
    AirPods3,
    AirPodsPro,
    AirPodsPro2Lightning,
    AirPodsPro2UsbC,
    AirPodsMaxLightning,
    AirPodsMaxUsbC,
    AirPods4,
    AirPods4Anc,
    AirPodsPro3,
    AirPodsMax2,
}

impl Model {
    /// Maps Apple's "A1234" model number to a `Model`.
    /// Source: https://support.apple.com/en-us/109525
    pub fn from_model_number(number: &str) -> Model {
        // A `match` must cover every possibility; `_` is the catch-all.
        // `|` lets several patterns share one result.
        match number {
            "A1523" | "A1722" => Model::AirPods1,
            "A2032" | "A2031" => Model::AirPods2,
            "A2565" | "A2564" => Model::AirPods3,
            "A2084" | "A2083" => Model::AirPodsPro,
            "A2931" | "A2699" | "A2698" => Model::AirPodsPro2Lightning,
            "A3047" | "A3048" | "A3049" => Model::AirPodsPro2UsbC,
            "A2096" => Model::AirPodsMaxLightning,
            "A3184" => Model::AirPodsMaxUsbC,
            "A3454" => Model::AirPodsMax2,
            "A3053" | "A3050" | "A3054" => Model::AirPods4,
            "A3056" | "A3055" | "A3057" => Model::AirPods4Anc,
            "A3063" | "A3064" | "A3065" => Model::AirPodsPro3,
            _ => Model::Unknown,
        }
    }

    /// `&'static str` is a string slice that lives for the whole program,
    /// which is what string literals are. No allocation needed.
    pub fn display_name(self) -> &'static str {
        match self {
            Model::AirPods1 => "AirPods",
            Model::AirPods2 => "AirPods (2nd generation)",
            Model::AirPods3 => "AirPods (3rd generation)",
            Model::AirPods4 | Model::AirPods4Anc => "AirPods 4",
            Model::AirPodsPro => "AirPods Pro",
            Model::AirPodsPro2Lightning => "AirPods Pro 2",
            Model::AirPodsPro2UsbC => "AirPods Pro 2 (USB-C)",
            Model::AirPodsMaxLightning => "AirPods Max",
            Model::AirPodsMaxUsbC => "AirPods Max (USB-C)",
            Model::AirPodsMax2 => "AirPods Max 2",
            Model::AirPodsPro3 => "AirPods Pro 3",
            Model::Unknown => "",
        }
    }

    // `matches!` is shorthand for a match that returns true/false.
    pub fn is_headset(self) -> bool {
        matches!(self, Model::AirPodsMaxLightning | Model::AirPodsMaxUsbC | Model::AirPodsMax2)
    }

    pub fn is_pro_series(self) -> bool {
        matches!(
            self,
            Model::AirPodsPro | Model::AirPodsPro2Lightning | Model::AirPodsPro2UsbC | Model::AirPodsPro3
        )
    }

    /// The Pro 3 dropped Off and silently ignores the packet.
    pub fn supports_noise_off(self) -> bool {
        self != Model::AirPodsPro3
    }

    /// Unknown reads as true: better to offer modes than to hide them.
    pub fn supports_noise_control(self) -> bool {
        matches!(
            self,
            Model::AirPods4Anc
                | Model::AirPodsPro
                | Model::AirPodsPro2Lightning
                | Model::AirPodsPro2UsbC
                | Model::AirPodsPro3
                | Model::AirPodsMaxLightning
                | Model::AirPodsMaxUsbC
                | Model::AirPodsMax2
                | Model::Unknown
        )
    }

    /// The H2 parts with noise control. Pro 1 and Max 1 lack this.
    fn has_h2_listening_features(self) -> bool {
        matches!(
            self,
            Model::AirPods4Anc
                | Model::AirPodsPro2Lightning
                | Model::AirPodsPro2UsbC
                | Model::AirPodsPro3
                | Model::AirPodsMax2
        )
    }

    pub fn supports_adaptive(self) -> bool {
        self.has_h2_listening_features()
    }

    pub fn supports_conversational_awareness(self) -> bool {
        self.has_h2_listening_features()
    }

    /// Needs noise control and a second bud, so never on a Max.
    pub fn supports_one_bud_anc(self) -> bool {
        matches!(
            self,
            Model::AirPods4Anc
                | Model::AirPodsPro
                | Model::AirPodsPro2Lightning
                | Model::AirPodsPro2UsbC
                | Model::AirPodsPro3
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pro3_has_no_off_but_has_adaptive() {
        let m = Model::from_model_number("A3064");
        assert_eq!(m, Model::AirPodsPro3);
        assert!(!m.supports_noise_off());
        assert!(m.supports_adaptive());
    }

    #[test]
    fn pro1_lacks_adaptive_but_has_one_bud() {
        let m = Model::from_model_number("A2084");
        assert!(!m.supports_adaptive());
        assert!(m.supports_one_bud_anc());
    }

    #[test]
    fn max2_is_headset_without_one_bud() {
        let m = Model::from_model_number("A3454");
        assert!(m.is_headset() && !m.supports_one_bud_anc() && m.supports_adaptive());
    }

    #[test]
    fn plain_airpods4_has_no_modes() {
        assert!(!Model::AirPods4.supports_noise_control());
        assert!(Model::AirPods4Anc.supports_noise_control());
    }

    #[test]
    fn unretired_pro3_codes_are_unknown() {
        for n in ["A3066", "A3334", ""] {
            assert_eq!(Model::from_model_number(n), Model::Unknown);
        }
    }
}
