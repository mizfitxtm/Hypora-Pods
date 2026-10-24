#!/bin/sh
# Builds hypora-pods, installs it under ~/.local, starts the service, and
# adds the widget to the Quickshell config. Safe to run again.
set -eu

cd "$(dirname "$0")/.."
PREFIX="${PREFIX:-$HOME/.local}"

if ! command -v cargo >/dev/null; then
    echo "cargo (Rust) is required to build the daemon." >&2
    echo "  Fedora / Hypora:  sudo dnf install cargo" >&2
    echo "  Anywhere else:    https://rustup.rs" >&2
    exit 1
fi
command -v pactl >/dev/null || echo "note: pactl not found; the audio codec will not be selected" >&2

# --locked builds exactly the dependency versions in Cargo.lock and refuses to change them
cargo build --release --locked
install -Dm755 target/release/hypora-pods "$PREFIX/bin/hypora-pods"
install -Dm755 target/release/hypora-pods-ctl "$PREFIX/bin/hypora-pods-ctl"
# The unit's ExecStart assumes ~/.local; point it at PREFIX if that was changed
UNIT="$HOME/.config/systemd/user/hypora-pods.service"
install -Dm644 systemd/hypora-pods.service "$UNIT"
if [ "$PREFIX" != "$HOME/.local" ]; then
    sed -i "s|^ExecStart=.*|ExecStart=$PREFIX/bin/hypora-pods|" "$UNIT"
fi

# Tap controls need one udev rule so you can read the AirPods' key device (only that device).
# This is the one step that asks for sudo; without a terminal it prints the command instead.
RULE_SRC="udev/70-hypora-pods.rules"
RULE_DEST="/etc/udev/rules.d/70-hypora-pods.rules"
# Run the commands directly: pasting $PWD into a string for `sh -c` would let a quote in the folder name run as root.
install_rule() {
    sudo install -m644 "$RULE_SRC" "$RULE_DEST" && sudo udevadm control --reload && sudo udevadm trigger --subsystem-match=input
}
if ! cmp -s "$RULE_SRC" "$RULE_DEST" 2>/dev/null; then
    echo "Tap controls: installing $RULE_DEST (needs sudo; lets you read the AirPods key device only)"
    if [ -t 0 ] && command -v sudo >/dev/null && install_rule; then
        echo "Tap controls enabled."
    else
        echo "Tap controls not enabled yet. To enable them, run this from the hypora-pods folder:" >&2
        echo "  sudo install -m644 $RULE_SRC $RULE_DEST && sudo udevadm control --reload && sudo udevadm trigger --subsystem-match=input" >&2
    fi
fi

systemctl --user daemon-reload
systemctl --user enable hypora-pods.service
systemctl --user restart hypora-pods.service

# Skip the widget with SKIP_WIDGET=1, or point WIDGET_DIR at another config.
if [ -z "${SKIP_WIDGET:-}" ]; then
    "$PREFIX/bin/hypora-pods-ctl" install-widget ${WIDGET_DIR:+"$WIDGET_DIR"} ||
        echo "widget not installed; see the message above" >&2
fi
echo "installed. Pair and connect your AirPods, then run: hypora-pods-ctl status"
