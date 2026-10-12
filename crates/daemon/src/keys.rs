//! The pairing keys the pods hand over, stored owner-only so later BLE
//! advertisements can be recognised and decrypted.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;

#[derive(Clone, PartialEq, Eq)]
pub struct Keys {
    pub irk: [u8; 16],
    pub enc_key: [u8; 16],
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keys(redacted)")
    }
}

/// On-disk form: hex strings, so the file is readable and diff-able.
#[derive(Serialize, Deserialize)]
struct Stored {
    irk: String,
    enc_key: String,
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex16(text: &str) -> Option<[u8; 16]> {
    if text.len() != 32 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

impl Keys {
    fn to_json(&self) -> String {
        serde_json::to_string(&Stored { irk: to_hex(&self.irk), enc_key: to_hex(&self.enc_key) }).expect("plain data serializes")
    }

    fn from_json(text: &str) -> Option<Keys> {
        let s: Stored = serde_json::from_str(text).ok()?;
        Some(Keys { irk: from_hex16(&s.irk)?, enc_key: from_hex16(&s.enc_key)? })
    }
}

pub fn path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(c) => PathBuf::from(c),
        None => PathBuf::from(std::env::var_os("HOME").filter(|v| !v.is_empty())?).join(".config"),
    };
    Some(base.join("hypora-pods").join("keys.json"))
}

pub fn load() -> Option<Keys> {
    Keys::from_json(&std::fs::read_to_string(path()?).ok()?)
}

pub fn save(keys: &Keys) -> std::io::Result<()> {
    let path = path().ok_or_else(|| std::io::Error::other("no config directory"))?;
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    // Write beside the target and rename, so a crash never leaves half a key file.
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    // `mode` above only applies to a new file, so set it again in case the temp file was left behind
    file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    file.write_all(keys.to_json().as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip() {
        let k = Keys { irk: core::array::from_fn(|i| i as u8), enc_key: [0xAB; 16] };
        assert_eq!(Keys::from_json(&k.to_json()), Some(k));
    }

    #[test]
    fn debug_hides_the_keys() {
        let k = Keys { irk: [0xAB; 16], enc_key: [0xCD; 16] };
        assert_eq!(format!("{k:?}"), "Keys(redacted)");
    }

    #[test]
    fn rejects_bad_files() {
        assert_eq!(Keys::from_json("not json"), None);
        assert_eq!(Keys::from_json(r#"{"irk":"00","enc_key":"00"}"#), None);
        let bad = format!(r#"{{"irk":"{}","enc_key":"{}"}}"#, "zz".repeat(16), "00".repeat(16));
        assert_eq!(Keys::from_json(&bad), None);
    }
}
