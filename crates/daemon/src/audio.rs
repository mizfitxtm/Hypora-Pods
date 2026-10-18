//! Gets the AirPods' PulseAudio/PipeWire card onto a playback profile.
//! The codec is PipeWire's own pick (AAC for AirPods): higher bitrates cost airtime a weak link cannot spare.

use serde_json::Value;
use std::time::Duration;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub description: String,
    pub sinks: u64,
    pub sources: u64,
    pub priority: u64,
    pub available: bool,
}

/// The playback-only profile PipeWire ranks highest.
/// Profiles with a source (the headset ones) are never chosen: they replace playback quality.
pub fn best_playback_profile(profiles: &[Profile]) -> Option<&Profile> {
    profiles
        .iter()
        .filter(|p| p.available && p.sinks > 0 && p.sources == 0)
        .max_by_key(|p| p.priority)
}

/// Active profile and profile list of the named card, from `pactl -f json list cards`.
pub fn parse_card(json: &str, card_name: &str) -> Option<(String, Vec<Profile>)> {
    let cards: Value = serde_json::from_str(json).ok()?;
    let card = cards.as_array()?.iter().find(|c| c["name"] == card_name)?;
    let active = card["active_profile"].as_str()?.to_string();
    let profiles = card["profiles"]
        .as_object()?
        .iter()
        .map(|(name, p)| Profile {
            name: name.clone(),
            description: p["description"].as_str().unwrap_or_default().to_string(),
            sinks: p["sinks"].as_u64().unwrap_or(0),
            sources: p["sources"].as_u64().unwrap_or(0),
            priority: p["priority"].as_u64().unwrap_or(0),
            available: p["available"].as_bool().unwrap_or(false),
        })
        .collect();
    Some((active, profiles))
}

async fn pactl(args: &[&str]) -> Option<String> {
    let out = Command::new("pactl").args(args).output().await.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// What to do to put audio on the pods: the sink to make default, and the
/// indexes of playing streams that are not on it yet.
#[derive(Debug, PartialEq, Eq)]
pub struct Routing {
    pub sink: String,
    pub streams_to_move: Vec<u64>,
}

/// Finds the pods' sink among `pactl -f json list sinks` and the streams to move
/// from `pactl -f json list sink-inputs`. `None` while the sink does not exist yet.
pub fn plan_routing(sinks_json: &str, inputs_json: &str, address: &str) -> Option<Routing> {
    let id = address.replace(':', "_");
    let sinks: Value = serde_json::from_str(sinks_json).ok()?;
    let sink = sinks.as_array()?.iter().find(|s| s["name"].as_str().is_some_and(|n| n.starts_with("bluez_") && n.contains(&id)))?;
    let (name, index) = (sink["name"].as_str()?.to_string(), sink["index"].as_u64()?);
    let inputs: Value = serde_json::from_str(inputs_json).unwrap_or(Value::Null);
    let streams_to_move = inputs
        .as_array()
        .map(|a| a.iter().filter(|i| i["sink"].as_u64() != Some(index)).filter_map(|i| i["index"].as_u64()).collect())
        .unwrap_or_default();
    Some(Routing { sink: name, streams_to_move })
}

/// Makes the pods the default output and moves what is playing onto them.
/// Waits for the sink, which appears a moment after the card profile is set.
pub async fn route_to_pods(address: &str) -> bool {
    for _ in 0..10 {
        if let (Some(sinks), Some(inputs)) = (pactl(&["-f", "json", "list", "sinks"]).await, pactl(&["-f", "json", "list", "sink-inputs"]).await) {
            if let Some(plan) = plan_routing(&sinks, &inputs, address) {
                let _ = pactl(&["set-default-sink", &plan.sink]).await;
                for index in plan.streams_to_move {
                    let _ = pactl(&["move-sink-input", &index.to_string(), &plan.sink]).await;
                }
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
    false
}

/// The full "pods just connected" audio step: best codec, then make them the output.
pub async fn activate(address: String) {
    select_best(address.clone()).await;
    if route_to_pods(&address).await {
        eprintln!("audio: output switched to the AirPods");
    } else {
        eprintln!("audio: no AirPods output appeared, leaving the default alone");
    }
}

pub fn card_name(address: &str) -> String {
    format!("bluez_card.{}", address.replace(':', "_"))
}

/// True if the default output is this device.
pub async fn is_default_sink(address: &str) -> bool {
    pactl(&["get-default-sink"]).await.is_some_and(|s| s.contains(&address.replace(':', "_")))
}

/// Moves a card that came up on no profile onto the best playback one. The card appears a moment
/// after BlueZ reports the connection, so this retries. A headset profile is left alone (someone
/// is on a call) and so is an A2DP one: that is the codec PipeWire, or the user, already chose.
pub async fn select_best(address: String) {
    let card = card_name(&address);
    for _ in 0..10 {
        if let Some(json) = pactl(&["-f", "json", "list", "cards"]).await {
            if let Some((active, profiles)) = parse_card(&json, &card) {
                if active.starts_with("headset-head-unit") || active.starts_with("a2dp-sink") {
                    return;
                }
                match best_playback_profile(&profiles) {
                    Some(best) if best.name != active => {
                        let _ = pactl(&["set-card-profile", &card, &best.name]).await;
                    }
                    _ => {}
                }
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, desc: &str, sinks: u64, sources: u64, priority: u64, available: bool) -> Profile {
        Profile { name: name.into(), description: desc.into(), sinks, sources, priority, available }
    }

    #[test]
    fn follows_pipewire_priority_not_bitrate() {
        let profiles = vec![
            p("off", "Off", 0, 0, 0, true),
            p("a2dp-sink-sbc", "High Fidelity Playback (A2DP Sink, codec SBC)", 1, 0, 133, true),
            p("a2dp-sink-sbc_xq", "High Fidelity Playback (A2DP Sink, codec SBC-XQ)", 1, 0, 132, true),
            p("a2dp-sink", "High Fidelity Playback (A2DP Sink, codec AAC)", 1, 0, 134, true),
            p("headset-head-unit", "Headset Head Unit (HSP/HFP)", 1, 1, 200, true),
        ];
        assert_eq!(best_playback_profile(&profiles).unwrap().name, "a2dp-sink");
    }

    #[test]
    fn skips_unavailable_and_headset_profiles() {
        let profiles = vec![
            p("a2dp-sink-sbc_xq", "x", 1, 0, 90, false),
            p("headset-head-unit", "x", 1, 1, 200, true),
        ];
        assert_eq!(best_playback_profile(&profiles), None);
    }

    #[test]
    fn parses_pactl_json() {
        let json = r#"[{"name":"bluez_card.AA_BB","active_profile":"a2dp-sink-aac","profiles":{
            "a2dp-sink-aac":{"description":"d","sinks":1,"sources":0,"priority":5,"available":true}}}]"#;
        let (active, profiles) = parse_card(json, "bluez_card.AA_BB").unwrap();
        assert_eq!(active, "a2dp-sink-aac");
        assert_eq!(profiles[0].priority, 5);
        assert!(parse_card(json, "other").is_none());
        assert!(parse_card("nope", "x").is_none());
    }

    #[test]
    fn plans_default_and_moves_only_streams_elsewhere() {
        let sinks = r#"[{"index":1,"name":"alsa_output.pci.analog-stereo"},{"index":7,"name":"bluez_output.AA_BB_CC_DD_EE_FF.1"}]"#;
        let inputs = r#"[{"index":10,"sink":1},{"index":11,"sink":7},{"index":12,"sink":1}]"#;
        let plan = plan_routing(sinks, inputs, "AA:BB:CC:DD:EE:FF").unwrap();
        assert_eq!(plan, Routing { sink: "bluez_output.AA_BB_CC_DD_EE_FF.1".into(), streams_to_move: vec![10, 12] });
    }

    #[test]
    fn no_plan_without_a_pods_sink() {
        let sinks = r#"[{"index":1,"name":"alsa_output.pci.analog-stereo"},{"index":2,"name":"bluez_output.11_22_33_44_55_66.1"}]"#;
        assert_eq!(plan_routing(sinks, "[]", "AA:BB:CC:DD:EE:FF"), None);
        assert_eq!(plan_routing("garbage", "[]", "AA:BB:CC:DD:EE:FF"), None);
    }

    #[test]
    fn card_name_uses_underscores() {
        assert_eq!(card_name("AA:BB:CC:DD:EE:FF"), "bluez_card.AA_BB_CC_DD_EE_FF");
    }
}
