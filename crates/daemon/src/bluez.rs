//! BlueZ over D-Bus: finds paired AirPods and reports when they connect or disconnect.
//! It only observes. Opening the AAP control link is phase 6.

use aap_protocol::AAP_SERVICE_UUID;
use futures_util::StreamExt;
use std::collections::HashMap;
use tokio::sync::mpsc::Sender;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
use zbus::{Connection, MatchRule, MessageStream};

const DEVICE_IFACE: &str = "org.bluez.Device1";

/// What the rest of the daemon hears from this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Connected { path: String, address: String, name: String },
    Disconnected { path: String },
}

/// AirPods advertise the AAP service UUID; that is how we tell them from any other headset.
pub fn is_airpods(uuids: &[String]) -> bool {
    uuids.iter().any(|u| u.eq_ignore_ascii_case(AAP_SERVICE_UUID))
}

type Props = HashMap<String, OwnedValue>;

fn string_of(props: &Props, key: &str) -> Option<String> {
    props.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok())
}

fn bool_of(props: &Props, key: &str) -> Option<bool> {
    props.get(key).and_then(|v| bool::try_from(v).ok())
}

fn uuids_of(props: &Props) -> Vec<String> {
    props
        .get("UUIDs")
        .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
        .unwrap_or_default()
}

fn event_for(path: &str, props: &Props) -> Option<Event> {
    if !is_airpods(&uuids_of(props)) || bool_of(props, "Connected")? != true {
        return None;
    }
    Some(Event::Connected {
        path: path.to_string(),
        address: string_of(props, "Address").unwrap_or_default(),
        name: aap_protocol::status::sanitize_name(&string_of(props, "Alias").or_else(|| string_of(props, "Name")).unwrap_or_default()),
    })
}

/// `Some(now)` when the connection state really changed, else `None`.
/// BlueZ reports many unrelated property changes on a connected device; without this each
/// one would look like a fresh connection and restart the control link and audio setup.
fn transition(previous: Option<bool>, now: Option<bool>) -> Option<bool> {
    match now {
        Some(n) if previous != Some(n) => Some(n),
        _ => None,
    }
}

/// Sends the current state, then keeps sending changes until the receiver is dropped.
pub async fn watch(tx: Sender<Event>) -> zbus::Result<()> {
    let conn = Connection::system().await?;

    // Subscribe before reading the current state so a change in between is not lost.
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.bluez")?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .build();
    // A device BlueZ adds later (a fresh pairing) arrives as InterfacesAdded, not as a change.
    let added_rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.bluez")?
        .interface("org.freedesktop.DBus.ObjectManager")?
        .member("InterfacesAdded")?
        .build();
    let mut changes = futures_util::stream::select(
        MessageStream::for_match_rule(rule, &conn, None).await?,
        MessageStream::for_match_rule(added_rule, &conn, None).await?,
    );

    // Remembers the paths of devices we know are AirPods, so a later
    // PropertiesChanged (which carries no UUIDs) can be matched to them.
    let mut known: HashMap<String, Props> = HashMap::new();
    // Last connection state we reported for each of those devices.
    let mut reported: HashMap<String, bool> = HashMap::new();

    let objects: HashMap<OwnedObjectPath, HashMap<zbus::names::OwnedInterfaceName, Props>> =
        zbus::fdo::ObjectManagerProxy::builder(&conn)
            .destination("org.bluez")?
            .path("/")?
            .build()
            .await?
            .get_managed_objects()
            .await?;
    for (path, ifaces) in objects {
        if let Some(props) = ifaces.into_iter().find(|(n, _)| n.as_str() == DEVICE_IFACE).map(|(_, p)| p) {
            if is_airpods(&uuids_of(&props)) {
                if let Some(ev) = event_for(path.as_str(), &props) {
                    if tx.send(ev).await.is_err() {
                        return Ok(());
                    }
                }
                reported.insert(path.to_string(), bool_of(&props, "Connected").unwrap_or(false));
                known.insert(path.to_string(), props);
            }
        }
    }

    while let Some(msg) = changes.next().await {
        let Ok(msg) = msg else { continue };
        if msg.header().member().is_some_and(|m| m.as_str() == "InterfacesAdded") {
            let Ok((path, ifaces)) = msg.body().deserialize::<(OwnedObjectPath, HashMap<String, Props>)>() else {
                continue;
            };
            if let Some(props) = ifaces.get(DEVICE_IFACE).filter(|p| is_airpods(&uuids_of(p))) {
                if let Some(ev) = event_for(path.as_str(), props) {
                    if tx.send(ev).await.is_err() {
                        return Ok(());
                    }
                }
                reported.insert(path.to_string(), bool_of(props, "Connected").unwrap_or(false));
                known.insert(path.to_string(), props.clone());
            }
            continue;
        }
        let Some(path) = msg.header().path().map(|p| p.to_string()) else { continue };
        let Ok((iface, changed, _invalidated)) = msg.body().deserialize::<(String, Props, Vec<String>)>() else {
            continue;
        };
        if iface != DEVICE_IFACE {
            continue;
        }
        // A change for a device we have not seen only matters once it reveals its UUIDs.
        if !known.contains_key(&path) && !changed.contains_key("UUIDs") {
            continue;
        }
        let entry = known.entry(path.clone()).or_default();
        for (k, v) in changed {
            entry.insert(k, v);
        }
        if !is_airpods(&uuids_of(entry)) {
            known.remove(&path);
            continue;
        }
        let now = bool_of(entry, "Connected");
        let ev = match transition(reported.get(&path).copied(), now) {
            Some(false) => Some(Event::Disconnected { path: path.clone() }),
            Some(true) => event_for(&path, entry),
            None => None,
        };
        if let Some(n) = now {
            reported.insert(path.clone(), n);
        }
        if let Some(ev) = ev {
            if tx.send(ev).await.is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    fn props(connected: bool, uuids: &[&str]) -> Props {
        let mut p = Props::new();
        p.insert("Connected".into(), OwnedValue::from(connected));
        p.insert("Address".into(), Value::from("AA:BB:CC:DD:EE:FF").try_into().unwrap());
        p.insert("Alias".into(), Value::from("My Pods").try_into().unwrap());
        let list: Vec<String> = uuids.iter().map(|s| s.to_string()).collect();
        p.insert("UUIDs".into(), Value::from(list).try_into().unwrap());
        p
    }

    #[test]
    fn only_real_changes_are_transitions() {
        assert_eq!(transition(None, Some(true)), Some(true));
        assert_eq!(transition(Some(false), Some(true)), Some(true));
        assert_eq!(transition(Some(true), Some(false)), Some(false));
        // the case that used to restart everything: a connected device changing some other property
        assert_eq!(transition(Some(true), Some(true)), None);
        assert_eq!(transition(Some(false), Some(false)), None);
        assert_eq!(transition(Some(true), None), None);
    }

    #[test]
    fn recognises_aap_uuid_any_case() {
        assert!(is_airpods(&["0000110b-0000-1000-8000-00805f9b34fb".into(), AAP_SERVICE_UUID.to_uppercase()]));
        assert!(!is_airpods(&["0000110b-0000-1000-8000-00805f9b34fb".into()]));
        assert!(!is_airpods(&[]));
    }

    #[test]
    fn connected_airpods_become_an_event() {
        let ev = event_for("/org/bluez/hci0/dev_AA", &props(true, &[AAP_SERVICE_UUID]));
        assert_eq!(
            ev,
            Some(Event::Connected { path: "/org/bluez/hci0/dev_AA".into(), address: "AA:BB:CC:DD:EE:FF".into(), name: "My Pods".into() })
        );
    }

    #[test]
    fn disconnected_or_other_devices_are_not_events() {
        assert_eq!(event_for("/p", &props(false, &[AAP_SERVICE_UUID])), None);
        assert_eq!(event_for("/p", &props(true, &["0000110b-0000-1000-8000-00805f9b34fb"])), None);
    }
}
