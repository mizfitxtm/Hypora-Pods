import Quickshell
import Quickshell.Io
import QtQuick

// Reads the status file hypora-pods publishes and sends verbs through hypora-pods-ctl.
// Nothing is polled: the daemon writes on change and removes the file when it stops.
Item {
    id: root

    property var s: ({})
    property bool daemonUp: false
    property string error: ""

    readonly property bool connected: daemonUp && s.connected === true
    readonly property bool isHeadset: s.is_headset === true
    readonly property string statePath: (Quickshell.env("XDG_STATE_HOME")
        || Quickshell.env("HOME") + "/.local/state") + "/hypora-pods/status.json"

    // -1 means the pod has not reported a level.
    function level(o) { return o && o.available ? o.level : -1 }
    readonly property int leftLevel: level(s.left)
    readonly property int rightLevel: level(s.right)
    readonly property int caseLevel: level(s.case)
    readonly property int headsetLevel: level(s.headset)
    readonly property int lowest: {
        const all = isHeadset ? [headsetLevel] : [leftLevel, rightLevel]
        const known = all.filter(l => l >= 0)
        return known.length ? Math.min(...known) : -1
    }

    // Listening modes this device really has, as [wire value, label, verb].
    readonly property var modes: {
        if (s.supports_noise_control === false) return []
        const m = []
        if (s.supports_noise_off !== false) m.push([0, "Off", "noise:off"])
        m.push([2, "Transparency", "noise:transparency"])
        if (s.supports_adaptive) m.push([3, "Adaptive", "noise:adaptive"])
        m.push([1, "Noise Cancellation", "noise:anc"])
        return m
    }

    // The installer (hypora-plugin or install-widget) replaces @BIN_DIR@ with the real directory.
    // Quickshell often starts without ~/.local/bin on PATH, so a bare name could not be found.
    readonly property string ctlPath: "@BIN_DIR@".startsWith("@") ? "hypora-pods-ctl" : "@BIN_DIR@/hypora-pods-ctl"

    function send(verb) {
        ctl.command = [ctlPath, verb]
        ctl.running = true
    }

    FileView {
        path: root.statePath
        watchChanges: true
        printErrors: false
        onFileChanged: reload()
        onLoaded: {
            try {
                root.s = JSON.parse(text())
                root.daemonUp = true
                root.error = ""
            } catch (e) {
                // A line we cannot read still proves the daemon is writing.
                root.daemonUp = true
                root.error = "unreadable status"
            }
        }
        // The daemon removes the file when it stops.
        onLoadFailed: { root.daemonUp = false; root.s = ({}) }
    }

    Process {
        id: ctl
        stderr: StdioCollector { id: err }
        stdout: StdioCollector { id: out }
        onExited: code => root.error = code === 0 ? "" : (out.text || err.text).trim()
    }
}
