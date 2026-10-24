//! Stem taps. The pods send every tap as its own media key over Bluetooth (AVRCP) and never merge
//! them into skip or previous, so the daemon counts: 1 = play/pause, 2 = next, 3 = previous.
//! It takes the pods' input device exclusively, so Hyprland never also acts on the same keys.

use std::fs::File;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use zbus::{Connection, Proxy};

/// Presses closer together than this belong to one gesture. It is also how long a single or double
/// tap waits before acting. A fast double tap on AirPods Pro arrives ~300-600 ms apart;
/// raise this if double taps turn into two pauses, lower it if single taps feel slow.
const WINDOW: Duration = Duration::from_millis(600);
const APPLE_VENDOR: &str = "004c";
// linux/input-event-codes.h
const KEY_NEXTSONG: u16 = 163;
const KEY_PLAYPAUSE: u16 = 164;
const KEY_PREVIOUSSONG: u16 = 165;
const KEY_PLAYCD: u16 = 200;
const KEY_PAUSECD: u16 = 201;
/// _IOW('E', 0x90, int)
const EVIOCGRAB: libc::c_ulong = 0x4004_4590;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    PlayPause,
    Next,
    Previous,
}

impl Cmd {
    fn method(self) -> &'static str {
        match self {
            Cmd::PlayPause => "PlayPause",
            Cmd::Next => "Next",
            Cmd::Previous => "Previous",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Tap,
    Cmd(Cmd),
}

fn key(code: u16) -> Option<Key> {
    match code {
        KEY_PLAYCD | KEY_PAUSECD | KEY_PLAYPAUSE => Some(Key::Tap),
        KEY_NEXTSONG => Some(Key::Cmd(Cmd::Next)),
        KEY_PREVIOUSSONG => Some(Key::Cmd(Cmd::Previous)),
        _ => None,
    }
}

fn gesture(taps: u32) -> Cmd {
    match taps {
        1 => Cmd::PlayPause,
        2 => Cmd::Next,
        _ => Cmd::Previous,
    }
}

/// Groups taps into gestures. Time is passed in so tests do not wait.
#[derive(Default)]
pub struct Tapper {
    count: u32,
    last: Option<Instant>,
}

impl Tapper {
    /// A third tap ends the gesture at once: nothing can follow it.
    pub fn press(&mut self, now: Instant) -> Option<Cmd> {
        self.count += 1;
        self.last = Some(now);
        if self.count >= 3 {
            *self = Tapper::default();
            return Some(Cmd::Previous);
        }
        None
    }

    /// When the current gesture ends, if one is open.
    pub fn deadline(&self) -> Option<Instant> {
        self.last.map(|t| t + WINDOW)
    }

    pub fn due(&mut self, now: Instant) -> Option<Cmd> {
        if self.deadline().is_some_and(|d| now >= d) {
            let cmd = gesture(self.count);
            *self = Tapper::default();
            return Some(cmd);
        }
        None
    }
}

/// BlueZ names its virtual keyboard "<device> (AVRCP)". Only Apple's is ours to take, and not an
/// iPhone or Mac's: a device that reports a different Bluetooth address than the connected pods is skipped.
fn is_pods_input(name: &str, vendor: &str, uniq: &str, address: &str) -> bool {
    let uniq = uniq.trim();
    // shortcut: a device with no address in `uniq` is accepted, so a kernel that leaves it blank still works; tighten if that picks wrong.
    let other_device = uniq.len() == 17 && uniq.matches(':').count() == 5 && !uniq.eq_ignore_ascii_case(address);
    name.trim().ends_with("(AVRCP)") && vendor.trim().eq_ignore_ascii_case(APPLE_VENDOR) && !other_device
}

fn find_device(address: &str) -> Option<PathBuf> {
    for entry in std::fs::read_dir("/sys/class/input").ok()?.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if !file.starts_with("event") {
            continue;
        }
        let dev = entry.path().join("device");
        let read = |p: &str| std::fs::read_to_string(dev.join(p)).unwrap_or_default();
        if is_pods_input(&read("name"), &read("id/vendor"), &read("uniq"), address) {
            return Some(PathBuf::from("/dev/input").join(file));
        }
    }
    None
}

/// Reads key presses on a thread of its own: a blocking read must not hold up the runtime's exit.
fn read_keys(mut dev: File, tx: mpsc::Sender<Key>) {
    let size = std::mem::size_of::<libc::input_event>();
    let mut buf = vec![0u8; size];
    // The type, code and value fields are the last 8 bytes, whatever the width of `timeval`.
    while dev.read_exact(&mut buf).is_ok() {
        let kind = u16::from_ne_bytes([buf[size - 8], buf[size - 7]]);
        let code = u16::from_ne_bytes([buf[size - 6], buf[size - 5]]);
        let value = i32::from_ne_bytes(buf[size - 4..].try_into().expect("4 bytes"));
        // EV_KEY (1), value 1 = pressed
        if kind == 1 && value == 1 {
            if let Some(k) = key(code) {
                if tx.blocking_send(k).is_err() {
                    return;
                }
            }
        }
    }
}

async fn send(conn: &Connection, cmd: Cmd) {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(conn).await else { return };
    let mut names: Vec<String> = dbus
        .list_names()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|n| n.to_string())
        .filter(|n| n.starts_with("org.mpris.MediaPlayer2.") && !n.ends_with("playerctld"))
        .collect();
    names.sort();
    let mut target = None;
    for name in names {
        let Ok(p) = Proxy::new(conn, name, "/org/mpris/MediaPlayer2", "org.mpris.MediaPlayer2.Player").await else { continue };
        let playing = p.get_property::<String>("PlaybackStatus").await.is_ok_and(|s| s == "Playing");
        // shortcut: with nothing playing the first player by name gets the command, upgrade if that picks wrong.
        if playing {
            target = Some(p);
            break;
        }
        if target.is_none() {
            target = Some(p);
        }
    }
    if let Some(p) = target {
        let _ = p.call_method(cmd.method(), &()).await;
    }
}

/// Runs for the life of the daemon: waits for the pods' input device, serves it until it goes away.
/// `address` is the connected pods' Bluetooth address, empty while none are connected.
pub async fn run(address: tokio::sync::watch::Receiver<String>) {
    let Ok(conn) = Connection::session().await else {
        eprintln!("tap controls disabled: no session bus");
        return;
    };
    let mut warned = false;
    loop {
        // shortcut: polls sysfs every 3 s instead of watching udev, fine for one device.
        tokio::time::sleep(Duration::from_secs(3)).await;
        let addr = address.borrow().clone();
        if addr.is_empty() {
            continue;
        }
        let Some(path) = find_device(&addr) else { continue };
        let dev = match File::open(&path) {
            Ok(d) => d,
            Err(e) => {
                if !warned {
                    eprintln!("tap controls: cannot open {} ({e}); see the udev rule in the README", path.display());
                    warned = true;
                }
                continue;
            }
        };
        // Exclusive: the compositor stops seeing these keys, so nothing acts on a tap twice.
        if unsafe { libc::ioctl(dev.as_raw_fd(), EVIOCGRAB, 1) } != 0 {
            eprintln!("tap controls: cannot take {} exclusively; leaving it to the compositor", path.display());
            tokio::time::sleep(Duration::from_secs(30)).await;
            continue;
        }
        warned = false;
        eprintln!("tap controls: serving {}", path.display());
        let (tx, mut rx) = mpsc::channel(16);
        std::thread::spawn(move || read_keys(dev, tx));
        let mut tapper = Tapper::default();
        loop {
            let wait = tapper.deadline().map(tokio::time::Instant::from_std);
            tokio::select! {
                k = rx.recv() => match k {
                    Some(Key::Tap) => {
                        if let Some(c) = tapper.press(Instant::now()) {
                            eprintln!("tap controls: {c:?}");
                            send(&conn, c).await;
                        }
                    }
                    Some(Key::Cmd(c)) => send(&conn, c).await,
                    None => break, // the device went away
                },
                _ = async { tokio::time::sleep_until(wait.expect("guarded")).await }, if wait.is_some() => {
                    if let Some(c) = tapper.due(Instant::now()) {
                        eprintln!("tap controls: {c:?}");
                        send(&conn, c).await;
                    }
                }
            }
        }
        eprintln!("tap controls: input device gone");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(gaps_ms: &[u64]) -> Option<Cmd> {
        let t0 = Instant::now();
        let mut t = Tapper::default();
        let mut now = t0;
        let mut out = t.press(now);
        for g in gaps_ms {
            now += Duration::from_millis(*g);
            assert_eq!(t.due(now), None, "gesture ended early");
            out = t.press(now);
        }
        // a third tap answers by itself; otherwise the answer comes when the window ends
        out.or_else(|| t.due(now + WINDOW))
    }

    #[test]
    fn one_two_three_taps() {
        assert_eq!(run(&[]), Some(Cmd::PlayPause));
        assert_eq!(run(&[300]), Some(Cmd::Next));
        assert_eq!(run(&[300, 330]), Some(Cmd::Previous));
        // spacing measured on AirPods Pro
        assert_eq!(run(&[582]), Some(Cmd::Next));
        assert_eq!(run(&[406, 582]), Some(Cmd::Previous));
    }

    #[test]
    fn slow_taps_are_separate_gestures() {
        let t0 = Instant::now();
        let mut t = Tapper::default();
        t.press(t0);
        assert_eq!(t.due(t0 + WINDOW + Duration::from_millis(50)), Some(Cmd::PlayPause));
        assert_eq!(t.deadline(), None);
    }

    #[test]
    fn only_apple_avrcp_devices_are_taken() {
        let pods = "40:DA:5C:57:3F:0D";
        assert!(is_pods_input("AirPods Pro (AVRCP)\n", "004c\n", "", pods));
        assert!(!is_pods_input("Sony WH-1000XM5 (AVRCP)", "054c", "", pods));
        assert!(!is_pods_input("Apple Keyboard", "004c", "", pods));
    }

    #[test]
    fn another_apple_device_is_left_alone() {
        let pods = "40:DA:5C:57:3F:0D";
        // an iPhone connected to the same laptop has its own address
        assert!(!is_pods_input("iPhone (AVRCP)", "004c", "AA:BB:CC:DD:EE:FF\n", pods));
        // same address in any letter case is the pods
        assert!(is_pods_input("AirPods Pro (AVRCP)", "004c", "40:da:5c:57:3f:0d\n", pods));
        // a blank or odd uniq keeps the old behaviour
        assert!(is_pods_input("AirPods Pro (AVRCP)", "004c", "", pods));
        assert!(is_pods_input("AirPods Pro (AVRCP)", "004c", "not-an-address", pods));
    }

    #[test]
    fn keys_map() {
        assert_eq!(key(200), Some(Key::Tap));
        assert_eq!(key(163), Some(Key::Cmd(Cmd::Next)));
        assert_eq!(key(30), None);
    }
}
