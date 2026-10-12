//! The AAP control link to one pair of AirPods.

use crate::l2cap::{parse_address, L2capStream};
use aap_protocol::inbound::{classify, Inbound};
use aap_protocol::{HANDSHAKE, REQUEST_NOTIFICATIONS, SET_SPECIFIC_FEATURES};
use std::io;
use std::time::Duration;
use tokio::sync::mpsc::{Receiver, Sender};

/// L2CAP PSM the pods listen on for AAP (assumed 0x1001, as upstream librepods uses).
pub const AAP_PSM: u16 = 0x1001;

/// Connects, runs the handshake, and forwards packets until the channel closes.
/// Returns `Ok(())` on a clean close by the pods, `Err` on any failure.
pub async fn run_once(address: &str, inbound: &Sender<Inbound>, outgoing: &mut Receiver<Vec<u8>>) -> io::Result<()> {
    let addr = parse_address(address).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad address"))?;
    let stream = L2capStream::connect(addr, AAP_PSM).await?;
    eprintln!("control link: connected to {address}");
    // Commands queued while the link was down are stale by now; replaying them later would be a surprise.
    while outgoing.try_recv().is_ok() {}
    stream.write(&HANDSHAKE).await?;

    let mut buf = [0u8; 1024];
    loop {
        tokio::select! {
            n = stream.read(&mut buf) => {
                let n = n?;
                if n == 0 {
                    return Ok(());
                }
                match classify(&buf[..n]) {
                    Inbound::HandshakeAck => stream.write(&SET_SPECIFIC_FEATURES).await?,
                    Inbound::FeaturesAck => stream.write(&REQUEST_NOTIFICATIONS).await?,
                    other => {
                        if inbound.send(other).await.is_err() {
                            return Ok(());
                        }
                    }
                }
            }
            packet = outgoing.recv() => match packet {
                Some(p) => stream.write(&p).await?,
                None => return Ok(()), // the daemon dropped its sender: stop
            }
        }
    }
}

/// Waiting this long before the first attempt lets a flapping connection settle, so we
/// do not hammer a link BlueZ is still rebuilding. The task is aborted if the device drops.
pub const SETTLE: Duration = Duration::from_secs(3);

/// Retries with a growing delay. Runs until the task is aborted (device disconnected).
pub async fn run(address: String, inbound: Sender<Inbound>, mut outgoing: Receiver<Vec<u8>>) {
    tokio::time::sleep(SETTLE).await;
    let mut delay = Duration::from_secs(2);
    loop {
        match run_once(&address, &inbound, &mut outgoing).await {
            Ok(()) => {
                eprintln!("control link: closed by the AirPods");
                delay = Duration::from_secs(2)
            }
            Err(e) => eprintln!("control link: {address} unreachable ({e}), retrying in {}s", delay.as_secs()),
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(60));
    }
}
