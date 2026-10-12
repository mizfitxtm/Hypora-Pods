//! The contract between `hypora-pods` (daemon) and `hypora-pods-ctl`:
//! the verbs sent over the socket, and where the socket and state file live.

use crate::noise::NoiseMode;
use std::path::PathBuf;

/// What the ear detection setting does (matches `ear_detection_behavior` in the status).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EarBehavior {
    PauseWhenOneOut = 0,
    PauseWhenBothOut = 1,
    Never = 2,
}

/// A parsed control request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Status,
    Noise(NoiseMode),
    Ear(EarBehavior),
    ConversationalAwareness(bool),
    OneBud(bool),
    Adaptive(u8),
}

/// Why a line was not a verb. The daemon sends this text back to the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Unknown,
    BadValue,
}

impl Verb {
    /// Parses one line such as `noise:anc` or `adaptive:40`.
    /// `&str` is a borrowed string slice: we read it without copying or owning it.
    pub fn parse(line: &str) -> Result<Verb, ParseError> {
        let line = line.trim_end_matches(['\n', '\r']);
        if line == "status" {
            return Ok(Verb::Status);
        }
        // `split_once` returns Option<(&str, &str)>; `ok_or` turns None into an error.
        let (name, value) = line.split_once(':').ok_or(ParseError::Unknown)?;
        match name {
            "noise" => match value {
                "off" => Ok(Verb::Noise(NoiseMode::Off)),
                "anc" => Ok(Verb::Noise(NoiseMode::NoiseCancellation)),
                "transparency" => Ok(Verb::Noise(NoiseMode::Transparency)),
                "adaptive" => Ok(Verb::Noise(NoiseMode::Adaptive)),
                _ => Err(ParseError::BadValue),
            },
            "ear" => match value {
                "one" => Ok(Verb::Ear(EarBehavior::PauseWhenOneOut)),
                "both" => Ok(Verb::Ear(EarBehavior::PauseWhenBothOut)),
                "off" => Ok(Verb::Ear(EarBehavior::Never)),
                _ => Err(ParseError::BadValue),
            },
            "ca" => parse_on_off(value).map(Verb::ConversationalAwareness),
            "onebud" => parse_on_off(value).map(Verb::OneBud),
            "adaptive" => parse_level(value).map(Verb::Adaptive),
            _ => Err(ParseError::Unknown),
        }
    }
}

impl Verb {
    /// The AAP packet that carries this verb to the pods, if it has one.
    /// `status` and `ear:*` are handled by the daemon and send nothing.
    pub fn packet(&self) -> Option<Vec<u8>> {
        match *self {
            Verb::Noise(mode) => Some(mode.packet()),
            Verb::ConversationalAwareness(on) => Some(crate::control::conversational_awareness(on)),
            Verb::OneBud(on) => Some(crate::control::one_bud_anc(on)),
            Verb::Adaptive(level) => Some(crate::control::adaptive_level(level)),
            Verb::Status | Verb::Ear(_) => None,
        }
    }
}

fn parse_on_off(value: &str) -> Result<bool, ParseError> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(ParseError::BadValue),
    }
}

/// Integer 0-100. Rejects whitespace, signs and empty input, so client bugs
/// like `adaptive: 50` or `adaptive:+5` surface instead of being tolerated.
fn parse_level(value: &str) -> Result<u8, ParseError> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ParseError::BadValue);
    }
    // `parse::<u8>` fails above 255; we then also cap at 100.
    match value.parse::<u8>() {
        Ok(n) if n <= 100 => Ok(n),
        _ => Err(ParseError::BadValue),
    }
}

/// `$XDG_RUNTIME_DIR/hypora-pods.sock`. `None` when the variable is unset:
/// callers must fail rather than fall back to a world-visible /tmp.
pub fn socket_path() -> Option<PathBuf> {
    socket_path_from(std::env::var_os("XDG_RUNTIME_DIR"))
}

// Split out so tests can pass a value instead of mutating the process environment.
fn socket_path_from(runtime_dir: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let dir = runtime_dir.filter(|d| !d.is_empty())?;
    Some(PathBuf::from(dir).join("hypora-pods.sock"))
}

/// `$XDG_STATE_HOME/hypora-pods/status.json`, falling back to `~/.local/state`.
pub fn state_path() -> Option<PathBuf> {
    state_path_from(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

fn state_path_from(state: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let base = match state.filter(|s| !s.is_empty()) {
        Some(s) => PathBuf::from(s),
        None => PathBuf::from(home.filter(|h| !h.is_empty())?).join(".local/state"),
    };
    Some(base.join("hypora-pods").join("status.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn parses_every_verb() {
        assert_eq!(Verb::parse("status"), Ok(Verb::Status));
        assert_eq!(Verb::parse("noise:anc\n"), Ok(Verb::Noise(NoiseMode::NoiseCancellation)));
        assert_eq!(Verb::parse("noise:adaptive"), Ok(Verb::Noise(NoiseMode::Adaptive)));
        assert_eq!(Verb::parse("ear:both"), Ok(Verb::Ear(EarBehavior::PauseWhenBothOut)));
        assert_eq!(Verb::parse("ca:on"), Ok(Verb::ConversationalAwareness(true)));
        assert_eq!(Verb::parse("onebud:off"), Ok(Verb::OneBud(false)));
        assert_eq!(Verb::parse("adaptive:0"), Ok(Verb::Adaptive(0)));
        assert_eq!(Verb::parse("adaptive:100"), Ok(Verb::Adaptive(100)));
    }

    #[test]
    fn verbs_map_to_packets() {
        assert_eq!(Verb::Noise(NoiseMode::Transparency).packet(), Some(NoiseMode::Transparency.packet()));
        assert_eq!(Verb::Adaptive(30).packet().unwrap()[7], 30);
        assert_eq!(Verb::Status.packet(), None);
        assert_eq!(Verb::Ear(EarBehavior::Never).packet(), None);
    }

    #[test]
    fn rejects_bad_levels() {
        for bad in ["adaptive:", "adaptive: 50", "adaptive:+5", "adaptive:-1", "adaptive:101", "adaptive:999", "adaptive:5x"] {
            assert_eq!(Verb::parse(bad), Err(ParseError::BadValue), "{bad}");
        }
    }

    #[test]
    fn rejects_unknown() {
        assert_eq!(Verb::parse(""), Err(ParseError::Unknown));
        assert_eq!(Verb::parse("connect"), Err(ParseError::Unknown));
        assert_eq!(Verb::parse("volume:3"), Err(ParseError::Unknown));
        assert_eq!(Verb::parse("noise:loud"), Err(ParseError::BadValue));
    }

    #[test]
    fn socket_requires_runtime_dir() {
        assert_eq!(socket_path_from(None), None);
        assert_eq!(socket_path_from(Some(OsString::new())), None);
        assert_eq!(
            socket_path_from(Some("/run/user/1000".into())),
            Some(PathBuf::from("/run/user/1000/hypora-pods.sock"))
        );
    }

    #[test]
    fn state_path_prefers_xdg_then_home() {
        assert_eq!(
            state_path_from(Some("/s".into()), Some("/h".into())),
            Some(PathBuf::from("/s/hypora-pods/status.json"))
        );
        assert_eq!(
            state_path_from(None, Some("/h".into())),
            Some(PathBuf::from("/h/.local/state/hypora-pods/status.json"))
        );
        assert_eq!(state_path_from(None, None), None);
    }
}
