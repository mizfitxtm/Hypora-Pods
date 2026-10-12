//! BLE scanning through BlueZ: reports Apple manufacturer data seen in advertisements.
//! Scanning runs only while `enabled` is true, because discovery alongside a live
//! control link is what causes audio crackle.

use futures_util::StreamExt;
use std::collections::HashMap;
use tokio::sync::{mpsc::Sender, watch};
use zbus::zvariant::{OwnedValue, Value};
use zbus::{Connection, MatchRule, MessageStream};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvertEvent {
    pub address: String,
    pub data: Vec<u8>,
}

type Props = HashMap<String, OwnedValue>;

/// "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF" -> "AA:BB:CC:DD:EE:FF".
fn address_from_path(path: &str) -> Option<String> {
    let tail = path.rsplit('/').next()?.strip_prefix("dev_")?;
    if tail.len() != 17 {
        return None;
    }
    Some(tail.replace('_', ":"))
}

/// Pulls Apple's manufacturer data out of a device's properties.
fn apple_data(props: &Props) -> Option<Vec<u8>> {
    let manufacturer: HashMap<u16, OwnedValue> = props.get("ManufacturerData")?.try_clone().ok()?.try_into().ok()?;
    let value = manufacturer.get(&ble_adv::APPLE_COMPANY_ID)?.try_clone().ok()?;
    Vec::<u8>::try_from(value).ok()
}

fn event_for(path: &str, props: &Props) -> Option<AdvertEvent> {
    let data = apple_data(props)?;
    // Apple broadcasts many unrelated message types; only type 0x07 is proximity pairing.
    // Dropping the rest here keeps a busy room from waking the main loop for nothing.
    if data.first() != Some(&0x07) {
        return None;
    }
    Some(AdvertEvent { address: address_from_path(path)?, data })
}

pub async fn scan(tx: Sender<AdvertEvent>, mut enabled: watch::Receiver<bool>) -> zbus::Result<()> {
    let conn = Connection::system().await?;
    // Assumes the first adapter; multi-adapter machines are not handled.
    let adapter = zbus::Proxy::new(&conn, "org.bluez", "/org/bluez/hci0", "org.bluez.Adapter1").await?;

    let changed_rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.bluez")?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .build();
    let added_rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.bluez")?
        .interface("org.freedesktop.DBus.ObjectManager")?
        .member("InterfacesAdded")?
        .build();
    let mut messages = futures_util::stream::select(
        MessageStream::for_match_rule(changed_rule, &conn, None).await?,
        MessageStream::for_match_rule(added_rule, &conn, None).await?,
    );

    let mut scanning = false;
    loop {
        tokio::select! {
            changed = enabled.changed() => {
                if changed.is_err() {
                    break; // the daemon dropped the sender
                }
                let want = *enabled.borrow();
                if want == scanning {
                    continue;
                }
                if want {
                    let filter: HashMap<&str, Value> =
                        HashMap::from([("Transport", Value::from("le")), ("DuplicateData", Value::from(true))]);
                    if let Err(e) = adapter.call_method("SetDiscoveryFilter", &(filter,)).await {
                        eprintln!("BLE filter: {e}");
                    }
                    match adapter.call_method("StartDiscovery", &()).await {
                        Ok(_) => scanning = true,
                        Err(e) => eprintln!("BLE scan did not start: {e}"),
                    }
                } else {
                    let _ = adapter.call_method("StopDiscovery", &()).await;
                    scanning = false;
                }
            }
            Some(Ok(msg)) = messages.next() => {
                if !scanning {
                    continue;
                }
                let header = msg.header();
                let event = match header.member().map(|m| m.as_str()) {
                    Some("PropertiesChanged") => {
                        let Some(path) = header.path().map(|p| p.to_string()) else { continue };
                        let Ok((iface, changed, _)) = msg.body().deserialize::<(String, Props, Vec<String>)>() else { continue };
                        if iface != "org.bluez.Device1" { continue }
                        event_for(&path, &changed)
                    }
                    Some("InterfacesAdded") => {
                        let Ok((path, ifaces)) =
                            msg.body().deserialize::<(zbus::zvariant::OwnedObjectPath, HashMap<String, Props>)>() else { continue };
                        ifaces.get("org.bluez.Device1").and_then(|p| event_for(path.as_str(), p))
                    }
                    _ => None,
                };
                if let Some(ev) = event {
                    if tx.send(ev).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
    if scanning {
        let _ = adapter.call_method("StopDiscovery", &()).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_from_object_path() {
        assert_eq!(address_from_path("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF").as_deref(), Some("AA:BB:CC:DD:EE:FF"));
        assert_eq!(address_from_path("/org/bluez/hci0"), None);
        assert_eq!(address_from_path("/org/bluez/hci0/dev_AA_BB"), None);
    }

    #[test]
    fn finds_apple_manufacturer_data() {
        let mut m: HashMap<u16, Value> = HashMap::new();
        m.insert(0x004C, Value::from(vec![7u8, 1, 2]));
        let mut props = Props::new();
        props.insert("ManufacturerData".into(), Value::from(m).try_into().unwrap());
        assert_eq!(apple_data(&props), Some(vec![7, 1, 2]));

        let mut other: HashMap<u16, Value> = HashMap::new();
        other.insert(0x0006, Value::from(vec![1u8]));
        let mut props = Props::new();
        props.insert("ManufacturerData".into(), Value::from(other).try_into().unwrap());
        assert_eq!(apple_data(&props), None);
    }
}
