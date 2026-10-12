//! Tells the daemon when the machine goes to sleep and wakes (systemd-logind).

use futures_util::StreamExt;
use tokio::sync::mpsc::Sender;
use zbus::{Connection, Proxy};

/// Sends `true` when going to sleep and `false` after waking.
pub async fn watch(tx: Sender<bool>) -> zbus::Result<()> {
    let conn = Connection::system().await?;
    let manager = Proxy::new(&conn, "org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager").await?;
    let mut signals = manager.receive_signal("PrepareForSleep").await?;
    while let Some(msg) = signals.next().await {
        if let Ok(going_to_sleep) = msg.body().deserialize::<bool>() {
            if tx.send(going_to_sleep).await.is_err() {
                break;
            }
        }
    }
    Ok(())
}
