//! AirPods AAP wire protocol: packet builders and parsers. Pure, no I/O.
//! Derived from librepods by Kavish Devar (GPL-3.0).

pub mod battery;
pub mod control;
pub mod ear;
pub mod inbound;
pub mod model;
pub mod ipc;
pub mod noise;
pub mod status;

/// L2CAP service UUID the AirPods expose for AAP.
pub const AAP_SERVICE_UUID: &str = "74ec2172-0bad-4d01-8f77-997b2be0722a";

pub const HANDSHAKE: [u8; 16] = [0, 0, 4, 0, 1, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
pub const SET_SPECIFIC_FEATURES: [u8; 14] =
    [0x04, 0, 0x04, 0, 0x4d, 0, 0xd7, 0, 0, 0, 0, 0, 0, 0];
pub const REQUEST_NOTIFICATIONS: [u8; 11] =
    [0x04, 0, 0x04, 0, 0x0f, 0, 0xff, 0xff, 0xff, 0xff, 0xff];

/// Headers that identify inbound packets.
pub const EAR_DETECTION_HEADER: [u8; 6] = [0x04, 0, 0x04, 0, 0x06, 0];
pub const BATTERY_HEADER: [u8; 6] = [0x04, 0, 0x04, 0, 0x04, 0];

/// Asks the pods for the keys that let us recognise their BLE advertisements.
pub const REQUEST_MAGIC_CLOUD_KEYS: [u8; 8] = [0x04, 0x00, 0x04, 0x00, 0x30, 0x00, 0x05, 0x00];
