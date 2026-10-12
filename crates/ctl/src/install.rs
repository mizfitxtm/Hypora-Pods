//! `install-widget` / `uninstall-widget`: put the Quickshell widget into a
//! Hypora shell config, without anyone editing files by hand.

use std::path::{Path, PathBuf};

// `include_str!` copies the file's text into the binary at compile time, so the
// installed `hypora-pods-ctl` carries the widget with it.
const FILES: [(&str, &str); 3] = [
    ("AirPods.qml", include_str!("../../../quickshell/AirPods.qml")),
    ("AirPodsPanel.qml", include_str!("../../../quickshell/AirPodsPanel.qml")),
    ("AirPodsService.qml", include_str!("../../../quickshell/AirPodsService.qml")),
];

const BIN_PLACEHOLDER: &str = "@BIN_DIR@";

/// Escapes text for a double-quoted QML string.
fn qml_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Writes the file next to its target and renames it into place, so a crash cannot leave
/// a half-written `Bar.qml`.
fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("qml.hypora-tmp");
    std::fs::write(&tmp, contents)?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path)
}

/// The marker lets us find (and remove) exactly the line we added.
const MARKER: &str = "// hypora-pods";
const BAR_LINE: &str = "AirPods { window: bar; anchors.verticalCenter: parent.verticalCenter }";
/// The bar item we insert in front of.
const ANCHOR: &str = "Tray {";

#[derive(Debug, PartialEq, Eq)]
pub enum PatchError {
    /// No `Tray {` line to insert before; the user must add the line by hand.
    AnchorNotFound,
}

/// Adds the widget line before the `Tray {` line. Already patched input is unchanged.
pub fn patch_bar(src: &str) -> Result<String, PatchError> {
    if src.contains(MARKER) {
        return Ok(src.to_string());
    }
    let mut out = String::with_capacity(src.len() + BAR_LINE.len() + 32);
    let mut done = false;
    for line in src.split_inclusive('\n') {
        if !done && line.trim_start().starts_with(ANCHOR) {
            let indent = &line[..line.len() - line.trim_start().len()];
            out.push_str(&format!("{indent}{BAR_LINE} {MARKER}\n"));
            done = true;
        }
        out.push_str(line);
    }
    if done { Ok(out) } else { Err(PatchError::AnchorNotFound) }
}

/// Removes every line carrying the marker.
pub fn unpatch_bar(src: &str) -> String {
    src.split_inclusive('\n').filter(|l| !l.contains(MARKER)).collect()
}

pub fn default_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(c) => PathBuf::from(c),
        None => PathBuf::from(std::env::var_os("HOME").filter(|v| !v.is_empty())?).join(".config"),
    };
    Some(base.join("quickshell"))
}

/// Returns a message for the user. `Err` means something needs their attention.
pub fn install(dir: &Path) -> Result<String, String> {
    let bar = dir.join("Bar.qml");
    let bar_src = std::fs::read_to_string(&bar)
        .map_err(|e| format!("cannot read {}: {e} (is this a Hypora Quickshell config?)", bar.display()))?;
    // Decide before writing anything, so a refusal leaves the config untouched.
    let patched = patch_bar(&bar_src).map_err(|_| {
        format!("no `{ANCHOR}` line in {}; add this inside the bar's right-hand Row:\n  {BAR_LINE}", bar.display())
    })?;
    // The directory this tool lives in, which is where the daemon is installed too.
    let ctl = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .ok()
        .and_then(|p| p.parent().map(|d| qml_escape(&d.to_string_lossy())))
        .unwrap_or_default();
    for (name, text) in FILES {
        let text = if ctl.is_empty() { text.to_string() } else { text.replace(BIN_PLACEHOLDER, &ctl) };
        std::fs::write(dir.join(name), text).map_err(|e| format!("cannot write {name}: {e}"))?;
    }
    if patched != bar_src {
        std::fs::write(dir.join("Bar.qml.hypora-pods.bak"), &bar_src).map_err(|e| format!("cannot back up Bar.qml: {e}"))?;
        write_atomic(&bar, &patched).map_err(|e| format!("cannot write Bar.qml: {e}"))?;
    }
    Ok(format!("installed into {} (Quickshell reloads on its own)", dir.display()))
}

pub fn uninstall(dir: &Path) -> Result<String, String> {
    let bar = dir.join("Bar.qml");
    let bar_src = std::fs::read_to_string(&bar).map_err(|e| format!("cannot read {}: {e}", bar.display()))?;
    let cleaned = unpatch_bar(&bar_src);
    if cleaned != bar_src {
        write_atomic(&bar, &cleaned).map_err(|e| format!("cannot write Bar.qml: {e}"))?;
    }
    for (name, _) in FILES {
        let _ = std::fs::remove_file(dir.join(name)); // already gone is fine
    }
    let _ = std::fs::remove_file(dir.join("Bar.qml.hypora-pods.bak"));
    Ok(format!("removed from {}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAR: &str = "Row {\n    spacing: 10\n\n    Tray { window: bar; anchors.verticalCenter: parent.verticalCenter }\n    Clock {}\n}\n";

    #[test]
    fn inserts_before_tray_with_same_indent() {
        let out = patch_bar(BAR).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[3].starts_with("    AirPods {") && lines[3].ends_with(MARKER));
        assert!(lines[4].trim_start().starts_with("Tray {"));
    }

    #[test]
    fn patch_is_idempotent_and_reversible() {
        let once = patch_bar(BAR).unwrap();
        assert_eq!(patch_bar(&once).unwrap(), once);
        assert_eq!(unpatch_bar(&once), BAR);
    }

    #[test]
    fn missing_anchor_is_an_error() {
        assert_eq!(patch_bar("Row { Clock {} }\n"), Err(PatchError::AnchorNotFound));
    }

    #[test]
    fn install_and_uninstall_roundtrip() {
        let dir = std::env::temp_dir().join(format!("hypora-pods-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Bar.qml"), BAR).unwrap();
        install(&dir).unwrap();
        assert!(dir.join("AirPods.qml").exists());
        assert!(std::fs::read_to_string(dir.join("Bar.qml")).unwrap().contains(MARKER));
        install(&dir).unwrap(); // second run changes nothing
        uninstall(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("Bar.qml")).unwrap(), BAR);
        assert!(!dir.join("AirPods.qml").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn install_bakes_in_an_absolute_tool_path() {
        let dir = std::env::temp_dir().join(format!("hypora-pods-test-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Bar.qml"), BAR).unwrap();
        install(&dir).unwrap();
        let service = std::fs::read_to_string(dir.join("AirPodsService.qml")).unwrap();
        let exe = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let bin_dir = exe.parent().unwrap().to_str().unwrap();
        assert!(service.contains(&format!("\"{bin_dir}/hypora-pods-ctl\"")), "the tool's directory is written into the widget");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn qml_strings_are_escaped() {
        assert_eq!(qml_escape(r#"a"b\c"#), r#"a\"b\\c"#);
    }

    #[test]
    fn refused_install_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("hypora-pods-test-refuse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Bar.qml"), "Row {}\n").unwrap();
        assert!(install(&dir).is_err());
        assert!(!dir.join("AirPods.qml").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
