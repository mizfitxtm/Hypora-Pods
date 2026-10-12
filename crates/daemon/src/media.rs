//! Pauses playback when the AirPods come out, and resumes what we paused.
//! Talks to media players through MPRIS on the session bus.

use crate::audio;
use std::time::Duration;
use tokio::sync::watch;
use zbus::{Connection, Proxy};

/// What the media task needs to know; changes are published through a `watch` channel.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EarState {
    pub left_in: bool,
    pub right_in: bool,
    /// 0 pause when one is out, 1 when both are out, 2 never. 2 also while disconnected.
    pub behavior: u8,
    pub address: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Pause,
    Resume,
}

/// The rule from the user's setting. `None` means leave playback alone.
pub fn decide(behavior: u8, left_in: bool, right_in: bool) -> Option<Action> {
    match behavior {
        0 if left_in && right_in => Some(Action::Resume),
        0 => Some(Action::Pause),
        1 if left_in || right_in => Some(Action::Resume),
        1 => Some(Action::Pause),
        _ => None,
    }
}

/// ANC and transparency changes briefly report both pods out; wait this long before believing it.
const BOTH_OUT_SETTLE: Duration = Duration::from_millis(1500);

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";

async fn players(conn: &Connection) -> zbus::Result<Vec<String>> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await?;
    Ok(dbus
        .list_names()
        .await?
        .into_iter()
        .map(|n| n.to_string())
        .filter(|n| n.starts_with(MPRIS_PREFIX) && !n.ends_with("playerctld"))
        .collect())
}

async fn player(conn: &Connection, name: &str) -> zbus::Result<Proxy<'static>> {
    Proxy::new(conn, name.to_string(), "/org/mpris/MediaPlayer2", "org.mpris.MediaPlayer2.Player").await
}

async fn pause_playing(conn: &Connection) -> Vec<String> {
    let mut paused = Vec::new();
    for name in players(conn).await.unwrap_or_default() {
        let Ok(p) = player(conn, &name).await else { continue };
        if p.get_property::<String>("PlaybackStatus").await.is_ok_and(|s| s == "Playing")
            && p.call_method("Pause", &()).await.is_ok()
        {
            paused.push(name);
        }
    }
    paused
}

async fn resume(conn: &Connection, names: &[String]) {
    for name in names {
        if let Ok(p) = player(conn, name).await {
            let _ = p.call_method("Play", &()).await;
        }
    }
}

/// Runs until the sender is dropped.
pub async fn run(mut rx: watch::Receiver<EarState>) {
    let Ok(conn) = Connection::session().await else {
        eprintln!("media control disabled: no session bus");
        return;
    };
    let mut paused: Vec<String> = Vec::new();
    let mut state = rx.borrow().clone();
    loop {
        // Pods gone: whatever we paused is no longer ours to resume.
        if state.behavior == 2 {
            paused.clear();
        }
        // A both-out report is held back; any newer report replaces it.
        if !state.left_in && !state.right_in && state.behavior != 2 {
            match tokio::time::timeout(BOTH_OUT_SETTLE, rx.changed()).await {
                Ok(Ok(())) => {
                    state = rx.borrow_and_update().clone();
                    continue;
                }
                Ok(Err(_)) => return,
                Err(_) => {} // settled
            }
        }
        match decide(state.behavior, state.left_in, state.right_in) {
            // Only pause for the AirPods: if audio plays elsewhere, leave it alone.
            Some(Action::Pause) if audio::is_default_sink(&state.address).await => {
                let newly = pause_playing(&conn).await;
                paused.extend(newly);
            }
            Some(Action::Resume) if !paused.is_empty() => {
                resume(&conn, &std::mem::take(&mut paused)).await;
            }
            _ => {}
        }
        if rx.changed().await.is_err() {
            return;
        }
        state = rx.borrow_and_update().clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_when_one_out() {
        assert_eq!(decide(0, true, false), Some(Action::Pause));
        assert_eq!(decide(0, false, false), Some(Action::Pause));
        assert_eq!(decide(0, true, true), Some(Action::Resume));
    }

    #[test]
    fn pause_when_both_out() {
        assert_eq!(decide(1, true, false), Some(Action::Resume));
        assert_eq!(decide(1, false, false), Some(Action::Pause));
    }

    #[test]
    fn never_does_nothing() {
        assert_eq!(decide(2, false, false), None);
        assert_eq!(decide(2, true, true), None);
    }
}
