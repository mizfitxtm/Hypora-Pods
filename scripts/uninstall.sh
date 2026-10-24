#!/bin/sh
# Removes what install.sh added. Pairing keys and the state file are deleted too.
set -eu

PREFIX="${PREFIX:-$HOME/.local}"

# On Hypora, hypora-plugin owns the widget files and removes them itself.
[ -n "${SKIP_WIDGET:-}" ] || "$PREFIX/bin/hypora-pods-ctl" uninstall-widget ${WIDGET_DIR:+"$WIDGET_DIR"} 2>/dev/null || true
systemctl --user disable --now hypora-pods.service 2>/dev/null || true
rm -f "$HOME/.config/systemd/user/hypora-pods.service" "$PREFIX/bin/hypora-pods" "$PREFIX/bin/hypora-pods-ctl"
# The udev rule is root-owned, so removing it asks for sudo too; without a terminal it is left and named.
RULE_DEST="/etc/udev/rules.d/70-hypora-pods.rules"
if [ -e "$RULE_DEST" ]; then
    if [ -t 0 ] && command -v sudo >/dev/null && sudo rm -f "$RULE_DEST" && sudo udevadm control --reload; then
        echo "removed $RULE_DEST"
    else
        echo "left $RULE_DEST in place; remove it with: sudo rm $RULE_DEST" >&2
    fi
fi
systemctl --user daemon-reload
rm -rf "$HOME/.config/hypora-pods" "$HOME/.local/state/hypora-pods"
echo "removed"
