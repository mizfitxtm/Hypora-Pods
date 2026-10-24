mod audio;
mod ble;
mod bluez;
mod keys;
mod media;
mod notify;
mod sleep;
mod l2cap;
mod link;
mod state;
mod taps;

use aap_protocol::ipc::{socket_path, state_path, Verb};
use aap_protocol::status::Status;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

/// The status plus the last line written, shared by every client task.
/// `Arc` lets many tasks own the same value; `Mutex` lets one at a time change it.
struct Shared {
    status: Status,
    last_line: String,
    live: state::Live,
    /// Packets for the pods. `None` when no control link is running.
    link_tx: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,
    link_task: Option<tokio::task::JoinHandle<()>>,
    audio_task: Option<tokio::task::JoinHandle<()>>,
    /// True in `--stub` mode, where verbs only change the fake status.
    stub: bool,
    keys: Option<keys::Keys>,
    /// Address of the connected pods, for audio decisions.
    address: String,
    sleeping: bool,
    /// BlueZ object path of the pods we are serving, so a second pair cannot disturb them.
    active_path: Option<String>,
    /// True once the pods have sent us something on the current connection.
    link_ready: bool,
    battery_watch: notify::BatteryWatch,
}

type SharedState = Arc<Mutex<Shared>>;

/// A panic while the lock was held poisons it, and `unwrap` would then panic every other
/// task too. The data is plain status values, so carrying on with it is safe.
fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Writes the state file only when the line changed. Writing to a temp file and
/// renaming means a reader never sees half a line.
fn publish(shared: &mut Shared, path: &Path) -> std::io::Result<()> {
    let line = shared.status.to_line();
    if line == shared.last_line {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        // Owner-only, like the file inside: the status names the user's device.
        std::os::unix::fs::DirBuilderExt::mode(std::fs::DirBuilder::new().recursive(true), 0o700).create(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    // Clear a leftover from a crash, then create_new refuses anything (a planted link too) that reappears.
    let _ = std::fs::remove_file(&tmp);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600) // owner only: the file carries the device identity
        .open(&tmp)?;
    file.write_all(line.as_bytes())?;
    std::fs::rename(&tmp, path)?;
    shared.last_line = line;
    Ok(())
}

/// Sends the verb's packet to the pods. Checks capabilities first so we never
/// send a packet the device would ignore. Stub mode has no pods to send to.
fn send_to_pods(shared: &Shared, verb: &Verb) -> Result<(), &'static str> {
    if shared.stub {
        return Ok(());
    }
    let Some(packet) = verb.packet() else { return Ok(()) };
    if !shared.link_ready {
        return Err("the AirPods control link is not up yet");
    }
    // Dry run on a copy: the real change is applied after the packet is queued.
    state::apply(&mut shared.status.clone(), *verb)?;
    let tx = shared.link_tx.as_ref().ok_or("AirPods are not connected")?;
    tx.try_send(packet).map_err(|_| "control link is busy or down")
}

/// A verb is a few bytes; anything longer is not one, and an unbounded read would let a
/// client that never sends a newline grow our memory.
const MAX_LINE: u64 = 256;

async fn handle_client(stream: UnixStream, shared: SharedState, state_file: std::path::PathBuf) {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    loop {
        let mut line = String::new();
        // `take` caps how much one read_line may consume.
        let read = (&mut reader).take(MAX_LINE).read_line(&mut line).await;
        let reply = match read {
            Ok(0) | Err(_) => break,
            Ok(_) if !line.ends_with('\n') && line.len() as u64 >= MAX_LINE => {
                let _ = write_half.write_all(b"error: line too long\n").await;
                break;
            }
            Ok(_) => match Verb::parse(&line) {
                Err(e) => format!("error: {e:?}\n"),
                // The lock is held only inside this block, never across an `.await`.
                Ok(verb) => {
                    let mut guard = lock(&shared);
                    let sent = send_to_pods(&guard, &verb);
                    match sent.and_then(|()| state::apply(&mut guard.status, verb)) {
                        Err(msg) => format!("error: {msg}\n"),
                        Ok(()) => {
                            if let Err(e) = publish(&mut guard, &state_file) {
                                eprintln!("cannot write state file: {e}");
                            }
                            if verb == Verb::Status { guard.status.to_line() } else { "ok\n".to_string() }
                        }
                    }
                }
            },
        };
        if write_half.write_all(reply.as_bytes()).await.is_err() {
            break;
        }
    }
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let (Some(sock), Some(state_file)) = (socket_path(), state_path()) else {
        eprintln!("XDG_RUNTIME_DIR and HOME/XDG_STATE_HOME must be set");
        return std::process::ExitCode::FAILURE;
    };
    // `--stub [MODEL]` pretends to be a device; without it the daemon watches BlueZ.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stub = args.first().is_some_and(|a| a == "--stub");
    let model = args.get(1).cloned().unwrap_or_else(|| "A3064".into());

    // A second daemon would delete the first one's socket and leave it unreachable. If something
    // answers on the socket, another daemon is running, so stop.
    if UnixStream::connect(&sock).await.is_ok() {
        eprintln!("hypora-pods is already running ({})", sock.display());
        return std::process::ExitCode::FAILURE;
    }
    let _ = std::fs::remove_file(&sock); // nothing answered: a stale socket from a crash would block bind
    let listener = match UnixListener::bind(&sock) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot bind {}: {e}", sock.display());
            return std::process::ExitCode::FAILURE;
        }
    };
    let _ = std::fs::set_permissions(&sock, std::os::unix::fs::PermissionsExt::from_mode(0o600));

    let shared: SharedState = Arc::new(Mutex::new(Shared {
        status: if stub { state::sample(&model) } else { Status::new() },
        last_line: String::new(),
        live: state::Live::default(),
        link_tx: None,
        link_task: None,
        audio_task: None,
        stub,
        keys: if stub { None } else { keys::load() },
        address: String::new(),
        sleeping: false,
        active_path: None,
        link_ready: false,
        battery_watch: notify::BatteryWatch::default(),
    }));
    if let Err(e) = publish(&mut lock(&shared), &state_file) {
        eprintln!("cannot write state file: {e}");
    }
    eprintln!("hypora-pods ({}) listening on {}", if stub { "stub" } else { "bluez" }, sock.display());

    if !stub {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        tokio::spawn(async move {
            if let Err(e) = bluez::watch(tx).await {
                eprintln!("cannot watch BlueZ: {e}");
            }
        });
        let (shared, state_file) = (shared.clone(), state_file.clone());
        let (adv_tx, mut adv_rx) = tokio::sync::mpsc::channel::<ble::AdvertEvent>(64);
        let (scan_tx, scan_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            if let Err(e) = ble::scan(adv_tx, scan_rx).await {
                eprintln!("cannot scan BLE: {e}");
            }
        });
        let (ear_tx, ear_rx) = tokio::sync::watch::channel(media::EarState::default());
        tokio::spawn(media::run(ear_rx));
        let (addr_tx, addr_rx) = tokio::sync::watch::channel(String::new());
        tokio::spawn(taps::run(addr_rx));
        let (raw_sleep_tx, mut raw_sleep_rx) = tokio::sync::mpsc::channel::<bool>(4);
        tokio::spawn(async move {
            if let Err(e) = sleep::watch(raw_sleep_tx).await {
                eprintln!("cannot watch suspend/resume: {e}");
            }
        });
        // After waking, give BlueZ a moment to settle before we scan or touch audio.
        let (sleep_tx, mut sleep_rx) = tokio::sync::mpsc::channel::<bool>(4);
        tokio::spawn(async move {
            while let Some(going_to_sleep) = raw_sleep_rx.recv().await {
                if !going_to_sleep {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                if sleep_tx.send(going_to_sleep).await.is_err() {
                    break;
                }
            }
        });
        let (in_tx, mut in_rx) = tokio::sync::mpsc::channel::<aap_protocol::inbound::Inbound>(32);
        tokio::spawn(async move {
            loop {
                // Wait for a BlueZ event or a packet from the pods.
                enum Msg { Bluez(bluez::Event), Packet(aap_protocol::inbound::Inbound), Advert(ble::AdvertEvent), Sleep(bool) }
                let msg = tokio::select! {
                    Some(e) = rx.recv() => Msg::Bluez(e),
                    Some(p) = in_rx.recv() => Msg::Packet(p),
                    Some(a) = adv_rx.recv() => Msg::Advert(a),
                    Some(s) = sleep_rx.recv() => Msg::Sleep(s),
                    else => break,
                };
                let mut guard = lock(&shared);
                match msg {
                    Msg::Bluez(bluez::Event::Connected { path, address, name }) => {
                        // Serve one pair at a time, and ignore a repeat report for the pair already set up.
                        let other_pair = guard.status.connected && guard.active_path.as_deref().is_some_and(|p| p != path);
                        let already_set_up = guard.status.connected && guard.active_path.as_deref() == Some(path.as_str()) && guard.link_task.is_some();
                        if !other_pair && !already_set_up {
                            if let Some(old) = guard.link_task.take() {
                                old.abort();
                            }
                            guard.status.connected = true;
                            guard.status.device_name = name;
                            guard.address = address.clone();
                            guard.active_path = Some(path);
                            guard.link_ready = false;
                            eprintln!("bluez: {:?} connected ({address})", guard.status.device_name);
                            // Wait for the connection to prove stable: switching codecs on a link that is
                            // about to drop only makes it worse. A disconnect aborts this task.
                            if let Some(old) = guard.audio_task.take() {
                                old.abort();
                            }
                            let stable = address.clone();
                            guard.audio_task = Some(tokio::spawn(async move {
                                tokio::time::sleep(link::SETTLE).await;
                                audio::activate(stable).await;
                            }));
                            let (out_tx, out_rx) = tokio::sync::mpsc::channel(16);
                            guard.link_tx = Some(out_tx);
                            guard.link_task = Some(tokio::spawn(link::run(address, in_tx.clone(), out_rx)));
                        }
                    }
                    Msg::Bluez(bluez::Event::Disconnected { path }) => {
                        if guard.active_path.as_deref() == Some(path.as_str()) {
                            if guard.status.connected {
                                eprintln!("bluez: AirPods disconnected");
                            }
                            if let Some(old) = guard.link_task.take() {
                                old.abort();
                            }
                            if let Some(old) = guard.audio_task.take() {
                                old.abort();
                            }
                            guard.link_tx = None;
                            guard.link_ready = false;
                            guard.active_path = None;
                            // Battery, model and case state stay: they are what the user wants to see
                            // with the pods in the case. Only the live link state goes.
                            guard.status.connected = false;
                            guard.status.noise_mode = -1;
                        }
                    }
                    Msg::Packet(p) => {
                        use aap_protocol::inbound::Inbound;
                        if let Inbound::MagicKeys { irk, enc_key } = &p {
                            let new = keys::Keys { irk: irk.0, enc_key: enc_key.0 };
                            if let Err(e) = keys::save(&new) {
                                eprintln!("cannot store pairing keys: {e}");
                            }
                            guard.keys = Some(new);
                        }
                        guard.link_ready = true;
                        let is_metadata = matches!(p, Inbound::Metadata { .. });
                        let Shared { status, live, .. } = &mut *guard;
                        state::apply_inbound(status, live, p);
                        // The keys let us recognise the pods' adverts once the link is gone.
                        if is_metadata {
                            if let Some(tx) = &guard.link_tx {
                                let _ = tx.try_send(aap_protocol::REQUEST_MAGIC_CLOUD_KEYS.to_vec());
                            }
                        }
                    }
                    Msg::Sleep(going_to_sleep) => {
                        guard.sleeping = going_to_sleep;
                        // The profile can be dropped during suspend, so set it again on wake.
                        if !going_to_sleep && guard.status.connected {
                            if let Some(old) = guard.audio_task.take() {
                                old.abort();
                            }
                            guard.audio_task = Some(tokio::spawn(audio::activate(guard.address.clone())));
                        }
                    }
                    Msg::Advert(a) => {
                        let Shared { status, live, keys, .. } = &mut *guard;
                        if let Some(keys) = keys {
                            state::apply_advert(status, live, keys, &a.address, &a.data);
                        }
                    }
                }
                // Scan only while the pods are away: discovery beside a live link crackles the audio.
                let want_scan = guard.keys.is_some() && !guard.status.connected && !guard.sleeping;
                scan_tx.send_if_modified(|v| std::mem::replace(v, want_scan) != want_scan);
                // Media follows the ears, but only while the pods are connected.
                let ears = media::EarState {
                    left_in: guard.status.left.in_ear,
                    right_in: guard.status.right.in_ear,
                    behavior: if guard.status.connected { guard.status.ear_detection_behavior } else { 2 },
                    address: guard.address.clone(),
                };
                ear_tx.send_if_modified(|v| std::mem::replace(v, ears.clone()) != ears);
                // Tap controls follow only the pods that are connected now
                let pods = if guard.status.connected { guard.address.clone() } else { String::new() };
                addr_tx.send_if_modified(|v| std::mem::replace(v, pods.clone()) != pods);
                let Shared { status, battery_watch, .. } = &mut *guard;
                for text in battery_watch.check(status) {
                    tokio::spawn(async move { notify::send("AirPods", &text).await });
                }
                if let Err(e) = publish(&mut guard, &state_file) {
                    eprintln!("cannot write state file: {e}");
                }
            }
        });
    }

    // systemd stops a service with SIGTERM, a terminal with SIGINT (Ctrl-C).
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("cannot install SIGTERM handler");
    loop {
        // Wait for a new client or a stop signal, whichever comes first.
        tokio::select! {
            accepted = listener.accept() => {
                if let Ok((stream, _)) = accepted {
                    tokio::spawn(handle_client(stream, shared.clone(), state_file.clone()));
                }
            }
            _ = tokio::signal::ctrl_c() => break,
            _ = sigterm.recv() => break,
        }
    }
    // An absent state file is how the widget learns the daemon stopped.
    let _ = std::fs::remove_file(&state_file);
    let _ = std::fs::remove_file(&sock);
    std::process::ExitCode::SUCCESS
}
